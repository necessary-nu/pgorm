//! A table's reference to one of several tables: nullable foreign-key
//! columns covered by the `CHECK` `Schema::create_one_of` writes, generated
//! as a `#[derive(OneOf)]` enum in the table's module.

use heck::ToUpperCamelCase;
use pgorm_query::{Enforcement, TableCreateStatement};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Column, ColumnExpr, Entity, Error, Operator, RelationType, TableIdent,
    util::{escape_rust_keyword, safe_ident},
};

/// The `#[derive(OneOf)]` enum one `CHECK` of a table generates.
// [spec:pgorm:def:disjoint.one-of+2]
#[derive(Clone, Debug)]
pub(crate) struct OneOfEnum {
    pub(crate) name: String,
    pub(crate) constraint: String,
    /// The variants naming a target, in the `CHECK`'s order.
    pub(crate) targets: Vec<OneOfTarget>,
    /// Whether a row may reference none, the `CHECK` being `<= 1`.
    pub(crate) none: bool,
}

/// One variant naming a target: its name, the column holding its key, and
/// the target entity's module, `None` for the table's own.
// [spec:pgorm:def:disjoint.one-of+2]
#[derive(Clone, Debug)]
pub(crate) struct OneOfTarget {
    pub(crate) variant: String,
    pub(crate) column: String,
    pub(crate) module: Option<String>,
}

/// A table's `CHECK` as the transform reads it: the name it has in the
/// database, its expression where it is within the subset codegen reads,
/// and whether the server holds rows to it.
pub(crate) struct TableCheck {
    name: String,
    expr: Option<ColumnExpr>,
    enforced: bool,
}

/// The unit variant of an enum whose `CHECK` lets a row reference none.
const NONE: &str = "None";

/// `table`'s `CHECK`s, each unnamed one named as PostgreSQL names it:
/// `{table}_{column}_check` over one column and `{table}_check` otherwise,
/// numbered from 1 where an earlier `CHECK`, or one the table names, took it.
// [spec:pgorm:def:disjoint.one-of+2]
pub(crate) fn table_checks(table: &TableCreateStatement) -> Vec<TableCheck> {
    let bare = table.get_table_name().table().to_string();
    let mut taken: BTreeSet<String> = table
        .get_checks()
        .iter()
        .filter_map(|check| check.get_name().map(|name| name.to_string()))
        .collect();
    table
        .get_checks()
        .iter()
        .map(|check| {
            let expr = ColumnExpr::read(check.get_expr()).ok();
            let name = match check.get_name() {
                Some(name) => name.to_string(),
                None => {
                    let read: BTreeSet<&str> = expr
                        .as_ref()
                        .map(|expr| expr.columns().into_iter().collect())
                        .unwrap_or_default();
                    let base = match read.into_iter().collect::<Vec<_>>().as_slice() {
                        [column] => format!("{bare}_{column}_check"),
                        _ => format!("{bare}_check"),
                    };
                    let name = (0..)
                        .map(|n| match n {
                            0 => base.clone(),
                            n => format!("{base}{n}"),
                        })
                        .find(|name| !taken.contains(name))
                        .unwrap_or(base);
                    taken.insert(name.clone());
                    name
                }
            };
            TableCheck {
                name,
                expr,
                enforced: check.get_enforcement() != Some(Enforcement::NotEnforced)
                    && !check.is_no_inherit(),
            }
        })
        .collect()
}

/// Give each entity the enums of its `CHECK`s that hold its nullable
/// foreign-key columns to one target: an enforced `num_nonnulls(..) = 1` or
/// `<= 1` over distinct columns, each the one column of a foreign key onto
/// a table's one-column primary key, of that key's type, and that table one
/// the entity's `Relation` reaches by that key's variant alone, so the derive
/// finds the entity `Related` to it.
///
/// Any other `CHECK` is refused, naming it: the entity holds no `CHECK` but
/// a one-of reference's, so an entity generated without it would not create
/// the table it was read from.
// [spec:pgorm:def:disjoint.one-of+2]
pub(crate) fn recognise(
    entities: &mut [Entity],
    checks: &BTreeMap<TableIdent, Vec<TableCheck>>,
) -> Result<(), Error> {
    let found: Vec<Vec<OneOfEnum>> = entities
        .iter()
        .map(|entity| {
            checks
                .get(&entity.ident())
                .into_iter()
                .flatten()
                .map(|check| {
                    shape(entities, entity, check).ok_or_else(|| {
                        Error::TransformError(format!(
                            "table `{}`: an entity cannot hold the CHECK `{}`, which is not a \
                             one-of reference's",
                            entity.ident(),
                            check.name
                        ))
                    })
                })
                .collect()
        })
        .collect::<Result<_, _>>()?;
    for (entity, one_of) in entities.iter_mut().zip(found) {
        entity.one_of = one_of;
    }
    Ok(())
}

/// The enum `check` generates on `entity`'s table, if it has the shape.
// [spec:pgorm:def:disjoint.one-of+2]
fn shape(entities: &[Entity], entity: &Entity, check: &TableCheck) -> Option<OneOfEnum> {
    if !check.enforced {
        return None;
    }
    let ColumnExpr::Binary(count, op, one) = check.expr.as_ref()? else {
        return None;
    };
    let none = match op {
        Operator::Eq => false,
        Operator::Le => true,
        _ => return None,
    };
    let (ColumnExpr::Call(function, args), ColumnExpr::Int(1)) = (&**count, &**one) else {
        return None;
    };
    if function != "num_nonnulls" || args.is_empty() {
        return None;
    }
    let columns: Vec<&str> = args
        .iter()
        .map(|arg| match arg {
            ColumnExpr::Column(column) => Some(column.as_str()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    if columns.iter().collect::<BTreeSet<_>>().len() != columns.len() {
        return None;
    }
    let targets = columns
        .into_iter()
        .map(|column| target(entities, entity, column))
        .collect::<Option<_>>()?;
    let prefix = format!("ck-{}-", entity.table_name);
    let named = check.name.strip_prefix(&prefix).unwrap_or(&check.name);
    Some(OneOfEnum {
        name: escape_rust_keyword(named.to_upper_camel_case()),
        constraint: check.name.clone(),
        targets,
        none,
    })
}

/// The variant `column` of `entity` names, if it is nullable and the one
/// column of a foreign key onto a table's one-column primary key of its
/// type, a table the entity's `Relation` reaches by that key's variant
/// alone.
// [spec:pgorm:def:disjoint.one-of+2]
fn target(entities: &[Entity], entity: &Entity, column: &str) -> Option<OneOfTarget> {
    let held = entity.columns.iter().find(|held| held.name == column)?;
    if held.not_null {
        return None;
    }
    let mut keys = entity.relations.iter().filter(|relation| {
        matches!(relation.rel_type, RelationType::BelongsTo) && relation.columns == [column]
    });
    let (Some(relation), None) = (keys.next(), keys.next()) else {
        return None;
    };
    let reaching = entity
        .relations
        .iter()
        .filter(|other| {
            other.ref_table == relation.ref_table
                && other.self_referencing == relation.self_referencing
        })
        .count();
    if relation.period.is_some() || reaching != 1 || !relation.impl_related {
        return None;
    }
    let ident = relation.ref_ident();
    let referenced = entities.iter().find(|target| target.ident() == ident)?;
    let [key] = referenced.primary_keys.as_slice() else {
        return None;
    };
    if relation.ref_columns != [key.name.as_str()] {
        return None;
    }
    let key_column = referenced
        .columns
        .iter()
        .find(|candidate| candidate.name == key.name)?;
    if key_type(key_column) != key_type(held) {
        return None;
    }
    Some(OneOfTarget {
        variant: escape_rust_keyword(referenced.table_name.to_upper_camel_case()),
        column: column.to_owned(),
        module: relation.get_module_name().map(|module| module.to_string()),
    })
}

/// The Rust type of `column`'s values, whether or not it is nullable.
fn key_type(column: &Column) -> String {
    Column {
        not_null: true,
        ..column.clone()
    }
    .get_rs_type()
    .to_string()
}

impl OneOfEnum {
    /// The enum as the derive reads it, after its entity's `Relation` enum.
    ///
    /// Its variants are named by the schema, for the target tables, so
    /// clippy's `enum_variant_names` is allowed on it as on a disjoint enum.
    // [spec:pgorm:def:disjoint.one-of+2]
    pub(crate) fn tokens(&self) -> TokenStream {
        let ident = format_ident!("{}", self.name);
        let variants = self.targets.iter().map(|target| {
            let variant = format_ident!("{}", target.variant);
            let column = format_ident!(
                "{}",
                escape_rust_keyword(target.column.to_upper_camel_case())
            );
            let entity = match &target.module {
                Some(module) => {
                    let module = format_ident!("{}", module);
                    quote! { super::#module::Entity }
                }
                None => quote! { Entity },
            };
            quote! {
                #[pgorm(column = Column::#column)]
                #variant(#entity)
            }
        });
        let none = self.none.then(|| {
            let none = format_ident!("{}", NONE);
            quote! { #none, }
        });
        quote! {
            #[allow(clippy::enum_variant_names)]
            #[derive(Clone, Copy, Debug, OneOf)]
            #[pgorm(entity = Entity)]
            pub enum #ident {
                #( #variants, )*
                #none
            }
        }
    }

    /// The items the enum's derive declares in the module, each with what
    /// it is: the enum, `<Enum>Key` and `<Enum>Model`.
    // [spec:pgorm:def:disjoint.one-of+2]
    pub(crate) fn items(&self) -> [(String, &'static str); 3] {
        [
            (self.name.clone(), "enum"),
            (format!("{}Key", self.name), "key enum"),
            (format!("{}Model", self.name), "model enum"),
        ]
    }

    /// Refuse a variant named as another of the enum's, or an enum named as
    /// no Rust identifier.
    // [spec:pgorm:def:disjoint.one-of+2]
    pub(crate) fn refuse_variant_clashes(&self, table: &TableIdent) -> Result<(), Error> {
        safe_ident(
            &format!(
                "table `{table}`: the one-of reference named for constraint `{}`",
                self.constraint
            ),
            &self.name,
        )?;
        let mut named: BTreeMap<&str, String> = BTreeMap::new();
        if self.none {
            named.insert(NONE, "its unit variant".to_owned());
        }
        for target in &self.targets {
            let other = format!("the variant for column `{}`", target.column);
            if let Some(first) = named.insert(&target.variant, other) {
                return Err(Error::TransformError(format!(
                    "table `{table}`: its one-of reference's variant `{}` is named as {first}",
                    target.variant
                )));
            }
        }
        Ok(())
    }
}
