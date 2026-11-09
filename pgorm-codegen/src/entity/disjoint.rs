//! A disjoint entity's tables, recognised by the constraints pgorm's disjoint
//! schema writes, and the `#[derive(Disjoint)]` enum they generate.
//!
//! The shape is a base table with a kind column of an enum type and a
//! `UNIQUE` key over its key and that column; for each kind with a table,
//! that table keyed as the base, with a stored kind column generated as its
//! kind and a cascading foreign key from its key and that column to the
//! `UNIQUE` key; and on the base, for each such kind, its key as stored
//! generated columns null for every other kind, with a foreign key from them
//! to the kind's table. Every foreign key is `DEFERRABLE INITIALLY
//! IMMEDIATE`. Only the whole shape holds every base row to exactly its own
//! kind's table, so a set missing any part, or with any part different, is
//! the ordinary tables it is.
//!
//! A set without the kind column, which nothing holds to one kind per base
//! row, is generated as an unkinded entity only where the caller names it
//! ([`UnkindedDisjoint`]), and refused where it does not have the shape.

use crate::{
    CaseExpr, ColumnExpr, Entity, Error, Operator, Relation, RelationType, TableIdent,
    entity::{active_enum::variant_name, transformer::resolve_reference},
    util::escape_rust_keyword,
};
use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::{ColumnType, Deferrability, ForeignKeyAction, GeneratedKind, Name};
use std::collections::{BTreeMap, BTreeSet};

/// The `#[derive(Disjoint)]` enum a base's module declares: its kind column
/// and kind type, which an unkinded enum lacks, and its kinds, in the kind
/// type's order or the order the caller named them.
// [spec:pgorm:sem:codegen.entity.disjoint+3]
// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1]
#[derive(Clone, Debug)]
pub(crate) struct DisjointEnum {
    pub(crate) kinded: Option<KindColumn>,
    pub(crate) kinds: Vec<Kind>,
    /// Whether every kind's `Model`, and so the enum holding them, is `Eq`,
    /// which the Rust writer derives.
    #[cfg_attr(not(feature = "rust"), allow(dead_code))]
    pub(crate) eq: bool,
    /// The base as a refusal of the enum names it: as the caller named it,
    /// for an unkinded enum, and as the schema does for a kinded one.
    #[cfg_attr(not(feature = "rust"), allow(dead_code))]
    pub(crate) named: TableIdent,
}

/// A kinded enum's kind column and the kind type it holds.
#[derive(Clone, Debug)]
pub(crate) struct KindColumn {
    pub(crate) column: String,
    pub(crate) kind_type: Name,
    pub(crate) kind_schema: Option<Name>,
}

/// One variant: its name, the `value` it carries where the kind type's value
/// is not its name in snake_case, and the module and identity of its table
/// when it has one.
#[derive(Clone, Debug)]
pub(crate) struct Kind {
    pub(crate) variant: String,
    pub(crate) value: Option<String>,
    #[cfg_attr(not(feature = "rust"), allow(dead_code))]
    pub(crate) table: Option<String>,
    pub(crate) ident: Option<TableIdent>,
}

/// A base table and its kinds' tables, named to generate as an unkinded
/// disjoint entity, `#[pgorm(unkinded)]`: one whose base has no kind column,
/// each base row read as the kind whose table has a row for it, and never
/// written, since nothing in its schema holds a row to one kind.
///
/// Every table one row of which extends another's one to one is keyed the
/// same way, so a set is named rather than guessed, and one named without the
/// shape is refused.
// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnkindedDisjoint {
    /// The base, whose module the enum is generated into, resolved as a
    /// foreign key's target is.
    pub base: TableIdent,
    /// Each kind's table, keyed by a foreign key from its primary key onto
    /// the base's, in the order the enum's variants take.
    pub kinds: Vec<TableIdent>,
    /// The name of the unit variant, the kind of a base row with a row in no
    /// kind's table, or `None`, where such a row is not one the entity
    /// describes.
    pub unit: Option<String>,
}

/// Where an enum a column names is declared, when it is not
/// `pgorm_active_enums`: its Rust name, and the module declaring it, `None`
/// for the column's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnumHome {
    pub(crate) name: String,
    pub(crate) module: Option<String>,
}

/// A unique key of a table as it was read: its name when it has one, its
/// columns in order, whether nulls are equal under it, and whether it is the
/// plain key the disjoint schema writes — no `NULLS NOT DISTINCT`, `INCLUDE`
/// or deferrability.
#[derive(Clone, Debug)]
pub(crate) struct UniqueKey {
    pub(crate) name: Option<String>,
    pub(crate) columns: Vec<String>,
    pub(crate) nulls_not_distinct: bool,
    pub(crate) plain: bool,
}

/// A recognised disjoint entity's kind type: the base it belongs to, and
/// the Rust name its derive gives it.
#[derive(Clone, Debug)]
pub(crate) struct KindType {
    pub(crate) base: TableIdent,
    pub(crate) rust_name: String,
}

pub(crate) fn enum_key(name: &Name, schema: &Option<Name>) -> String {
    match schema {
        Some(schema) => format!("{}.{}", schema.to_string(), name.to_string()),
        None => name.to_string(),
    }
}

/// The kind type the derive declares for the enum named `ident`.
pub(crate) fn kind_type_ident(ident: &str) -> String {
    format!("{ident}Kind")
}

/// One kind's table as the shape requires it.
struct KindTable {
    ident: TableIdent,
    key: Vec<String>,
}

/// A complete shape found on a base, before it is applied.
struct Shape {
    base: TableIdent,
    kind_column: String,
    kind_type: (Name, Option<Name>),
    values: Vec<String>,
    kinds: BTreeMap<String, KindTable>,
    /// The base's generated key columns.
    bookkeeping: BTreeSet<String>,
    /// The columns of each of the base's keys from them to a kind's table.
    base_keys: Vec<Vec<String>>,
}

/// Find every disjoint entity among `entities` and rewrite its tables into
/// the entities it generates: the base holding the enum, without its
/// generated key columns or their keys; each kind's table without its kind
/// column, its cascading key replaced by a relation from its key to the
/// base's; and every column of a kind type typed as the derive's
/// `<Enum>Kind`. Returns the kind types by identity.
// [spec:pgorm:sem:codegen.entity.disjoint+3]
pub(crate) fn recognise(
    entities: &mut BTreeMap<TableIdent, Entity>,
    unique_keys: &BTreeMap<TableIdent, Vec<UniqueKey>>,
) -> Result<BTreeMap<String, KindType>, Error> {
    let mut shapes: Vec<Shape> = Vec::new();
    for (ident, base) in entities.iter() {
        let found: Vec<Shape> = base
            .columns
            .iter()
            .filter_map(|column| shape(entities, unique_keys, ident, base, &column.name))
            .collect();
        if let [first, second, ..] = found.as_slice() {
            return Err(Error::TransformError(format!(
                "table `{ident}`: columns `{}` and `{}` each make it the base of a disjoint \
                 entity, whose enum is named for the table, so only one can be generated",
                first.kind_column, second.kind_column
            )));
        }
        shapes.extend(found);
    }

    let mut kind_types: BTreeMap<String, KindType> = BTreeMap::new();
    for shape in &shapes {
        let key = enum_key(&shape.kind_type.0, &shape.kind_type.1);
        let Some(base) = entities.get(&shape.base) else {
            continue;
        };
        let rust_name = kind_type_ident(&escape_rust_keyword(base.get_table_name_camel_case()));
        if let Some(first) = kind_types.insert(
            key.clone(),
            KindType {
                base: shape.base.clone(),
                rust_name,
            },
        ) {
            return Err(Error::TransformError(format!(
                "enum `{key}` is the kind type of the disjoint entities of tables `{}` and `{}`; \
                 each one's derive declares a kind type of its own",
                first.base, shape.base
            )));
        }
    }

    for shape in shapes {
        apply(entities, shape);
    }
    for (key, kind_type) in &kind_types {
        let module = entities
            .get(&kind_type.base)
            .map(|base| escape_rust_keyword(base.get_table_name_snake_case()));
        for (ident, entity) in entities.iter_mut() {
            let home = EnumHome {
                name: kind_type.rust_name.clone(),
                module: (*ident != kind_type.base).then(|| module.clone()).flatten(),
            };
            for column in entity.columns.iter_mut() {
                if let ColumnType::Enum { name, schema, .. } = column.get_inner_col_type()
                    && enum_key(name, schema) == *key
                {
                    column.enum_home = Some(home.clone());
                }
            }
        }
    }
    Ok(kind_types)
}

/// Give the base of each set `sets` names the unkinded enum it generates,
/// once the set is found to have the shape: each kind's table keyed by a
/// foreign key from exactly its primary key onto the base's, column for
/// column and of the same types. A set without it is refused, naming why.
/// `declared` is every table of the schema, which a set's names resolve
/// against as a foreign key's target does; kinded entities are recognised
/// first.
// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1]
pub(crate) fn unkinded(
    entities: &mut BTreeMap<TableIdent, Entity>,
    declared: &[TableIdent],
    sets: &[UnkindedDisjoint],
) -> Result<(), Error> {
    let mut bases: BTreeSet<TableIdent> = BTreeSet::new();
    for set in sets {
        let refuse = |why: String| {
            Error::TransformError(format!(
                "unkinded disjoint entity over `{}`: {why}",
                set.base
            ))
        };
        let resolve = |ident: &TableIdent| {
            resolve_reference(declared, ident)
                .cloned()
                .ok_or_else(|| refuse(format!("table `{ident}` is not in the schema")))
        };
        let named = std::iter::once(&set.base)
            .chain(&set.kinds)
            .map(resolve)
            .collect::<Result<Vec<_>, _>>()?;
        for (index, table) in named.iter().enumerate() {
            if named[..index].contains(table) {
                return Err(refuse(format!("it names table `{table}` twice")));
            }
        }
        let (base_ident, kinds) = (&named[0], &named[1..]);
        if kinds.is_empty() {
            return Err(refuse("it names no kind's table".to_owned()));
        }
        if !bases.insert(base_ident.clone()) {
            return Err(refuse(
                "it is named twice, and a base's module holds one enum".to_owned(),
            ));
        }
        let variants =
            unkinded_kinds(entities, base_ident, kinds, set.unit.as_deref()).map_err(refuse)?;
        let eq = all_eq(entities, kinds.iter());
        if let Some(base) = entities.get_mut(base_ident) {
            base.disjoint = Some(DisjointEnum {
                kinded: None,
                kinds: variants,
                eq,
                named: set.base.clone(),
            });
        }
    }
    Ok(())
}

/// The variants of the unkinded enum over `base_ident` with the tables
/// `kinds` and the unit variant `unit`, or why the set does not have the
/// shape.
// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1]
fn unkinded_kinds(
    entities: &BTreeMap<TableIdent, Entity>,
    base_ident: &TableIdent,
    kinds: &[TableIdent],
    unit: Option<&str>,
) -> Result<Vec<Kind>, String> {
    let entity = |ident: &TableIdent| {
        entities
            .get(ident)
            .ok_or_else(|| format!("table `{ident}` is not in the schema"))
    };
    let base = entity(base_ident)?;
    if let Some(kinded) = base.disjoint.as_ref().and_then(|d| d.kinded.as_ref()) {
        return Err(format!(
            "its base is the base of a disjoint entity whose kind column `{}` holds each row's \
             kind",
            kinded.column
        ));
    }
    let base_key: Vec<&String> = base.primary_keys.iter().map(|key| &key.name).collect();
    if base_key.is_empty() {
        return Err("its base has no primary key".to_owned());
    }
    if base.without_overlaps {
        return Err("its base's primary key ends WITHOUT OVERLAPS".to_owned());
    }
    let mut variants = Vec::with_capacity(kinds.len() + 1);
    for ident in kinds {
        let kind = entity(ident)?;
        let key: Vec<&String> = kind.primary_keys.iter().map(|key| &key.name).collect();
        let pairs: BTreeSet<(&String, &String)> =
            key.iter().copied().zip(base_key.iter().copied()).collect();
        let keyed = !kind.without_overlaps
            && key.len() == base_key.len()
            && kind.relations.iter().any(|relation| {
                let declared: BTreeSet<(&String, &String)> =
                    relation.columns.iter().zip(&relation.ref_columns).collect();
                relation.ref_ident() == *base_ident
                    && relation.period.is_none()
                    && relation.columns.len() == key.len()
                    && declared == pairs
            });
        if !keyed {
            return Err(format!(
                "the primary key of `{ident}` is not a foreign key onto the base's primary key, \
                 column for column"
            ));
        }
        for (column, base_column) in key.iter().zip(&base_key) {
            let type_of = |entity: &Entity, name: &str| {
                entity
                    .columns
                    .iter()
                    .find(|each| each.name == name)
                    .map(|each| each.col_type.clone())
            };
            if type_of(kind, column) != type_of(base, base_column) {
                return Err(format!(
                    "column `{ident}`.`{column}` is not of the type of \
                     `{base_ident}`.`{base_column}`, which it is keyed by"
                ));
            }
        }
        variants.push(Kind {
            variant: escape_rust_keyword(kind.get_table_name_camel_case()),
            value: None,
            table: module_of(entities, ident),
            ident: Some(ident.clone()),
        });
    }
    if let Some(unit) = unit {
        let name = escape_rust_keyword(unit.to_upper_camel_case());
        if let Some((_, ident)) = variants
            .iter()
            .zip(kinds)
            .find(|(kind, _)| kind.variant == name)
        {
            return Err(format!(
                "its unit variant `{name}` is named as the variant of `{ident}`"
            ));
        }
        variants.push(Kind {
            variant: name,
            value: None,
            table: None,
            ident: None,
        });
    }
    Ok(variants)
}

/// Rewrite the tables of one recognised shape.
fn apply(entities: &mut BTreeMap<TableIdent, Entity>, shape: Shape) {
    let Shape {
        base: base_ident,
        kind_column,
        kind_type,
        values,
        kinds,
        bookkeeping,
        base_keys,
    } = shape;
    let variants: Vec<Kind> = values
        .iter()
        .map(|value| {
            let variant = variant_name(value.trim());
            let kind = kinds.get(value);
            Kind {
                value: (variant.to_snake_case() != *value).then(|| value.clone()),
                variant,
                table: kind.and_then(|kind| module_of(entities, &kind.ident)),
                ident: kind.map(|kind| kind.ident.clone()),
            }
        })
        .collect();
    let eq = all_eq(entities, kinds.values().map(|kind| &kind.ident));
    let Some(base) = entities.get_mut(&base_ident) else {
        return;
    };
    let base_key: Vec<String> = base
        .primary_keys
        .iter()
        .map(|key| key.name.clone())
        .collect();
    base.columns
        .retain(|column| !bookkeeping.contains(&column.name));
    base.relations
        .retain(|relation| !base_keys.contains(&relation.columns));
    // The shape's own UNIQUE pair is written by `create_disjoint`, so the
    // entity declaring it too would write it twice.
    let pair: Vec<String> = base_key
        .iter()
        .cloned()
        .chain([kind_column.clone()])
        .collect();
    base.unique_keys
        .retain(|key| !(key.plain && key.columns == pair));
    base.disjoint = Some(DisjointEnum {
        kinded: Some(KindColumn {
            column: kind_column.clone(),
            kind_type: kind_type.0,
            kind_schema: kind_type.1,
        }),
        kinds: variants,
        eq,
        named: base_ident.clone(),
    });
    for kind in kinds.values() {
        let Some(entity) = entities.get_mut(&kind.ident) else {
            continue;
        };
        entity.columns.retain(|column| column.name != kind_column);
        for relation in entity.relations.iter_mut() {
            if relation.ref_ident() == base_ident && relation.ref_columns.contains(&kind_column) {
                *relation = key_to_base(&base_ident, &kind.key, &base_key);
            }
        }
    }
}

/// The module the entity of `ident` is generated into, keyword-escaped as
/// the writer names it, or `None` for a table the schema lacks.
fn module_of(entities: &BTreeMap<TableIdent, Entity>, ident: &TableIdent) -> Option<String> {
    entities
        .get(ident)
        .map(|entity| escape_rust_keyword(entity.get_table_name_snake_case()))
}

/// Whether every one of `kinds`' entities derives `Eq`, so that the enum
/// holding their `Model`s can.
fn all_eq<'a>(
    entities: &BTreeMap<TableIdent, Entity>,
    mut kinds: impl Iterator<Item = &'a TableIdent>,
) -> bool {
    kinds.all(|kind| entities.get(kind).is_some_and(|entity| !entity.has_float()))
}

/// The relation a kind's table generates in place of its cascading key: from
/// its key to the base's, which that key implies and the disjoint schema
/// writes no key of its own for.
fn key_to_base(base: &TableIdent, key: &[String], base_key: &[String]) -> Relation {
    Relation {
        ref_table: base.table.clone(),
        ref_schema: base.schema.clone(),
        columns: key.to_vec(),
        ref_columns: base_key.to_vec(),
        rel_type: RelationType::BelongsTo,
        on_update: None,
        on_delete: None,
        enforcement: None,
        deferrability: None,
        period: None,
        self_referencing: false,
        num_suffix: 0,
        impl_related: true,
    }
}

/// `NO ACTION` stated or left unsaid, which are one action.
fn no_action(action: &Option<ForeignKeyAction>) -> bool {
    matches!(action, None | Some(ForeignKeyAction::NoAction))
}

/// Whether `relation` is a key as the disjoint schema writes one: checked at
/// the end of each statement, enforced, matching no period, `ON UPDATE NO
/// ACTION`, and `ON DELETE CASCADE` when `cascade` and `NO ACTION` otherwise.
fn deferrable_key(relation: &Relation, cascade: bool) -> bool {
    relation.deferrability == Some(Deferrability::DeferrableInitiallyImmediate)
        && relation.enforcement.is_none()
        && relation.period.is_none()
        && no_action(&relation.on_update)
        && if cascade {
            matches!(relation.on_delete, Some(ForeignKeyAction::Cascade))
        } else {
            no_action(&relation.on_delete)
        }
}

/// The complete shape with `base`'s column `kind_column` as its kind column,
/// or `None` where any part of it is missing or different.
fn shape(
    entities: &BTreeMap<TableIdent, Entity>,
    all_unique_keys: &BTreeMap<TableIdent, Vec<UniqueKey>>,
    base_ident: &TableIdent,
    base: &Entity,
    kind_column: &str,
) -> Option<Shape> {
    let unique_keys_of = |ident: &TableIdent| {
        all_unique_keys
            .get(ident)
            .map(Vec::as_slice)
            .unwrap_or_default()
    };
    let unique_keys = unique_keys_of(base_ident);
    let base_key: Vec<String> = base
        .primary_keys
        .iter()
        .map(|key| key.name.clone())
        .collect();
    if base_key.is_empty() || base.without_overlaps || base_key.iter().any(|k| k == kind_column) {
        return None;
    }
    let column = base
        .columns
        .iter()
        .find(|column| column.name == kind_column)?;
    let ColumnType::Enum {
        name,
        schema,
        variants,
    } = &column.col_type
    else {
        return None;
    };
    let written = column.not_null
        && column.generated.is_none()
        && column.identity.is_none()
        && !column.auto_increment;
    let unique: Vec<String> = base_key
        .iter()
        .cloned()
        .chain([kind_column.to_owned()])
        .collect();
    let keyed = unique_keys
        .iter()
        .any(|key| key.plain && key.columns == unique);
    if !written || !keyed {
        return None;
    }
    let kind_type = &column.col_type;
    let values: Vec<String> = variants.iter().map(|value| value.to_string()).collect();
    let key_types: Vec<&ColumnType> = base_key
        .iter()
        .map(|key| base.columns.iter().find(|column| &column.name == key))
        .map(|column| column.map(|column| &column.col_type))
        .collect::<Option<_>>()?;

    let mut kinds: BTreeMap<String, KindTable> = BTreeMap::new();
    for (ident, entity) in entities.iter().filter(|(ident, _)| *ident != base_ident) {
        let mut to_pair = entity.relations.iter().filter(|relation| {
            relation.ref_ident() == *base_ident
                && relation
                    .ref_columns
                    .iter()
                    .any(|column| column == kind_column)
        });
        let Some(relation) = to_pair.next() else {
            continue;
        };
        if to_pair.next().is_some() {
            return None;
        }
        let (value, key) = kind_table(
            entity,
            relation,
            &unique,
            kind_column,
            kind_type,
            &key_types,
        )?;
        if !values.contains(&value) || base_key_relation(entity, base_ident, &key, &base_key) {
            return None;
        }
        let kind_keys = unique_keys_of(ident);
        if kind_keys
            .iter()
            .any(|key| key.columns.iter().any(|column| column == kind_column))
        {
            return None;
        }
        let kind = KindTable {
            ident: ident.clone(),
            key,
        };
        if kinds.insert(value, kind).is_some() {
            return None;
        }
    }

    let generated_key = |column_name: &str| -> Option<(String, usize)> {
        let column = base
            .columns
            .iter()
            .find(|column| column.name == column_name)?;
        let plain = !column.not_null
            && !column.unique
            && column.default.is_none()
            && column.identity.is_none()
            && !column.auto_increment
            && !base_key.iter().any(|key| key == column_name);
        let Some((ColumnExpr::Case(case), GeneratedKind::Stored)) =
            column.generated.as_ref().filter(|_| plain)
        else {
            return None;
        };
        let CaseExpr {
            first: (condition, ColumnExpr::Column(read)),
            rest,
            otherwise: None,
        } = case.as_ref()
        else {
            return None;
        };
        let ColumnExpr::Binary(left, Operator::Eq, right) = condition else {
            return None;
        };
        let (ColumnExpr::Column(tested), ColumnExpr::Text(value)) = (left.as_ref(), right.as_ref())
        else {
            return None;
        };
        let position = base_key.iter().position(|key| key == read)?;
        let typed = key_types
            .get(position)
            .is_some_and(|ty| **ty == column.col_type);
        (rest.is_empty() && tested == kind_column && typed).then(|| (value.clone(), position))
    };

    let mut base_keys: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for relation in &base.relations {
        let shaped: Option<Vec<(String, usize)>> = relation
            .columns
            .iter()
            .map(|column| generated_key(column))
            .collect();
        let Some(shaped) = shaped.filter(|shaped| !shaped.is_empty()) else {
            continue;
        };
        let value = shaped.first().map(|(value, _)| value.clone())?;
        let in_order = shaped
            .iter()
            .enumerate()
            .all(|(position, (each, at))| *each == value && *at == position);
        let kind = kinds.get(&value)?;
        let to_kind = in_order
            && shaped.len() == base_key.len()
            && relation.ref_ident() == kind.ident
            && relation.ref_columns == kind.key
            && deferrable_key(relation, false);
        if !to_kind || base_keys.insert(value, relation.columns.clone()).is_some() {
            return None;
        }
    }
    if base_keys.keys().ne(kinds.keys()) {
        return None;
    }

    let bookkeeping: BTreeSet<String> = base_keys.values().flatten().cloned().collect();
    let base_keys: Vec<Vec<String>> = base_keys.into_values().collect();
    let used_elsewhere = base
        .relations
        .iter()
        .filter(|relation| !base_keys.contains(&relation.columns))
        .any(|relation| relation.columns.iter().any(|c| bookkeeping.contains(c)))
        || unique_keys
            .iter()
            .any(|key| key.columns.iter().any(|c| bookkeeping.contains(c)));
    let referenced = entities.iter().any(|(_, entity)| {
        entity.relations.iter().any(|relation| {
            let target = relation.ref_ident();
            let reads = |columns: &BTreeSet<String>| {
                relation
                    .ref_columns
                    .iter()
                    .any(|column| columns.contains(column))
            };
            (target == *base_ident && reads(&bookkeeping))
                || (kinds.values().any(|kind| kind.ident == target)
                    && reads(&BTreeSet::from([kind_column.to_owned()])))
        })
    });
    if used_elsewhere || referenced {
        return None;
    }
    Some(Shape {
        base: base_ident.clone(),
        kind_column: kind_column.to_owned(),
        kind_type: (name.clone(), schema.clone()),
        values,
        kinds,
        bookkeeping,
        base_keys,
    })
}

/// The value and key of `entity` as the table of a kind, `relation` being
/// its one key to the base's `UNIQUE` columns: keyed as the base, by columns
/// the database does not fill, with a stored kind column generated as its
/// kind's value and a cascading deferrable key from its key and that column.
fn kind_table(
    entity: &Entity,
    relation: &Relation,
    unique: &[String],
    kind_column: &str,
    kind_type: &ColumnType,
    key_types: &[&ColumnType],
) -> Option<(String, Vec<String>)> {
    let key: Vec<String> = entity
        .primary_keys
        .iter()
        .map(|key| key.name.clone())
        .collect();
    let pairs: Vec<String> = key
        .iter()
        .cloned()
        .chain([kind_column.to_owned()])
        .collect();
    let keyed = !entity.without_overlaps
        && key.len() == key_types.len()
        && relation.columns == pairs
        && relation.ref_columns == unique
        && deferrable_key(relation, true);
    if !keyed || key.iter().any(|column| column == kind_column) {
        return None;
    }
    for (column, ty) in key.iter().zip(key_types) {
        let column = entity.columns.iter().find(|each| &each.name == column)?;
        let given = column.default.is_none()
            && column.generated.is_none()
            && column.identity.is_none()
            && !column.auto_increment;
        if !given || column.col_type != **ty {
            return None;
        }
    }
    let column = entity
        .columns
        .iter()
        .find(|column| column.name == kind_column)?;
    let plain = column.col_type == *kind_type
        && !column.not_null
        && !column.unique
        && column.default.is_none()
        && column.identity.is_none()
        && !column.auto_increment;
    let used = entity
        .relations
        .iter()
        .filter(|other| !std::ptr::eq(*other, relation))
        .any(|other| other.columns.iter().any(|column| column == kind_column));
    match column.generated.as_ref() {
        Some((ColumnExpr::Text(value), GeneratedKind::Stored)) if plain && !used => {
            Some((value.clone(), key))
        }
        _ => None,
    }
}

/// Whether `entity` has a key from exactly `key` to the base's key, which
/// the cascading key implies and the disjoint schema therefore never writes.
fn base_key_relation(
    entity: &Entity,
    base: &TableIdent,
    key: &[String],
    base_key: &[String],
) -> bool {
    let implied: BTreeSet<(&String, &String)> = key.iter().zip(base_key).collect();
    entity.relations.iter().any(|relation| {
        let pairs: BTreeSet<(&String, &String)> =
            relation.columns.iter().zip(&relation.ref_columns).collect();
        relation.ref_ident() == *base && relation.period.is_none() && pairs == implied
    })
}
