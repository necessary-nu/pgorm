//! The transformed model read as the bindings' declared models declare a
//! schema, which the Python and TypeScript writers both write: each column's
//! kind, the column type it states, what fills it, each table's unique keys,
//! the relations in both directions, the junction paths, and the disjoint and
//! one-of shapes. Each part is named in the vocabulary the two bindings share
//! and spelled by each writer in its own language; what a declared model
//! cannot say is refused here, by name, for both.

use crate::{
    Column, ColumnExpr, Entities, Entity, Error, OneOfEnum, Relation, RelationType, TableIdent,
    entity::column::under_domains,
};
use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::{
    ArrayType, ColumnType, Deferrability, Enforcement, ForeignKeyAction, GeneratedKind,
    IdentityGeneration, IntervalSpec, Name, PgInterval, StringLen,
};
use std::collections::BTreeMap;

/// The options both declared-model writers take.
///
/// `Default` is the no-options shape: unqualified types in `public`, tables
/// unqualified, no unkinded disjoint model.
// [spec:pgorm:def:codegen.models]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelWriterOptions {
    /// The schema an unqualified table and an unqualified type are in, as
    /// `EntityWriterOptions::schema_name` is for Rust. A declared model names
    /// an enum's and a created range's schema, so an unqualified type is in
    /// `public` when this is `None`; an unqualified table stays unqualified.
    pub schema_name: Option<String>,
    /// The table sets to generate as unkinded disjoint models, read as
    /// `EntityWriterOptions::unkinded_disjoint` is.
    pub unkinded_disjoint: Vec<crate::UnkindedDisjoint>,
}

/// A type a declared model names by its name and schema: an enum's
/// `TypeName`, a created range's `CreatedRange`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TypeRef {
    pub(crate) name: String,
    pub(crate) schema: String,
}

/// What a declared column's values are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A kind by its name: `i32`, `text`, `int4range`.
    Scalar(&'static str),
    /// An enum, by its type, which the module declares once.
    Enum(TypeRef),
    /// A range type the schema created, or its multirange, by its type.
    Created(TypeRef),
}

/// A column type a declared column states with `column_type`, where its
/// kind's own type is not the column's: a built-in type by the name the
/// bindings' `DataType` takes, one with a size, an interval with fields or a
/// precision, an enum or created range as a domain's base, or a domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DdlType {
    Builtin(&'static str),
    /// `char(n)` or `varchar(n)`.
    Sized(&'static str, u32),
    /// `numeric(p, s)`.
    Numeric(u32, u32),
    /// `interval` with its fields, in snake_case, and a precision.
    Interval(Option<&'static str>, Option<u8>),
    Enum(TypeRef),
    Created(TypeRef),
    Domain {
        name: String,
        schema: Option<String>,
        base: Box<DdlType>,
    },
}

/// A literal default, as a declared column's `default_value` holds it: a
/// value the column's kind takes as it is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Literal {
    Int(i64),
    Float(String),
    Decimal(String),
    Text(String),
    Bool(bool),
}

/// What fills a column an insert leaves out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fill<'a> {
    None,
    /// The serial family, `auto_increment`.
    AutoIncrement,
    /// An identity, `generated`.
    Identity(IdentityGeneration),
    /// A literal default, `default_value`.
    Value(Literal),
    /// A default expression, `default_expr`.
    Expr(&'a ColumnExpr),
    /// A literal default the kind does not take as it is, bound as the
    /// value it is, `default_expr` of a bound value.
    Bound(Literal),
    /// A generation expression, `generated_stored` or `generated_virtual`.
    Generated(&'a ColumnExpr, GeneratedKind),
}

/// One declared column.
#[derive(Clone, Debug)]
pub(crate) struct DeclaredColumn<'a> {
    pub(crate) column: &'a Column,
    pub(crate) kind: Kind,
    pub(crate) array: bool,
    pub(crate) nullable: bool,
    pub(crate) primary_key: bool,
    pub(crate) values: Option<Vec<String>>,
    pub(crate) column_type: Option<DdlType>,
    pub(crate) fill: Fill<'a>,
    pub(crate) without_overlaps: bool,
}

/// A unique key a model lists: its columns, its name and whether nulls are
/// equal under it.
#[derive(Clone, Debug)]
pub(crate) struct DeclaredKey {
    pub(crate) columns: Vec<String>,
    pub(crate) name: Option<String>,
    pub(crate) nulls_not_distinct: bool,
}

/// One declared model.
#[derive(Clone, Debug)]
pub(crate) struct DeclaredModel<'a> {
    pub(crate) entity: &'a Entity,
    /// The model's name, the table's in UpperCamelCase, as the Rust entity's.
    pub(crate) name: String,
    pub(crate) columns: Vec<DeclaredColumn<'a>>,
    pub(crate) unique_keys: Vec<DeclaredKey>,
}

/// A relation's foreign-key action, by the neutral name the bindings spell.
pub(crate) fn action(action: &ForeignKeyAction) -> Option<&'static str> {
    Some(match action {
        ForeignKeyAction::NoAction => "no_action",
        ForeignKeyAction::Restrict => "restrict",
        ForeignKeyAction::Cascade => "cascade",
        ForeignKeyAction::SetNull => "set_null",
        ForeignKeyAction::SetDefault => "set_default",
        _ => return None,
    })
}

/// One relation of a model, as its `belongs_to`, `has_one` or `has_many`
/// declares it: the Rust `Relation` variant it is, so each binding names it
/// for the model it starts at and that variant.
#[derive(Clone, Debug)]
pub(crate) struct DeclaredRelation<'a> {
    pub(crate) from: &'a DeclaredModel<'a>,
    pub(crate) to: &'a DeclaredModel<'a>,
    pub(crate) kind: RelationType,
    /// The Rust variant's name: the target table's in UpperCamelCase, or
    /// `SelfRef`, numbered where several reach one table.
    pub(crate) variant: String,
    pub(crate) from_columns: Vec<String>,
    pub(crate) to_columns: Vec<String>,
    pub(crate) on_delete: Option<&'static str>,
    pub(crate) on_update: Option<&'static str>,
}

/// A path through a junction, as a Rust `Related` impl with a `via`
/// declares it: `via`, from the model to the junction, then `relation`, the
/// junction's to the far end, each an index into the relations.
#[derive(Clone, Debug)]
pub(crate) struct DeclaredRelated<'a> {
    pub(crate) from: &'a DeclaredModel<'a>,
    pub(crate) to: &'a DeclaredModel<'a>,
    pub(crate) relation: usize,
    pub(crate) via: usize,
}

/// One kind of a disjoint model: its value and its table's model, if any.
#[derive(Clone, Debug)]
pub(crate) struct DeclaredKind<'a> {
    pub(crate) value: String,
    pub(crate) model: Option<&'a DeclaredModel<'a>>,
}

/// A disjoint model over a base.
#[derive(Clone, Debug)]
pub(crate) struct DeclaredDisjoint<'a> {
    pub(crate) base: &'a DeclaredModel<'a>,
    /// The disjoint model's name, its Rust enum's in snake_case.
    pub(crate) name: String,
    /// The kind column, its kind type's name where it is not `{name}_kind`,
    /// and its schema; `None` for an unkinded one.
    pub(crate) kinded: Option<(String, Option<String>, String)>,
    pub(crate) kinds: Vec<DeclaredKind<'a>>,
}

/// One target of a one-of reference: its value, the column holding its key
/// and the target's model.
#[derive(Clone, Debug)]
pub(crate) struct DeclaredTarget<'a> {
    pub(crate) value: String,
    pub(crate) column: String,
    pub(crate) model: &'a DeclaredModel<'a>,
}

/// A one-of reference of a model.
#[derive(Clone, Debug)]
pub(crate) struct DeclaredOneOf<'a> {
    pub(crate) model: &'a DeclaredModel<'a>,
    /// The reference's name, which its `CHECK`, `ck-{table}-{name}`, carries.
    pub(crate) name: String,
    /// The Rust enum's name, which the binding's variable is named after.
    pub(crate) rust_name: String,
    pub(crate) targets: Vec<DeclaredTarget<'a>>,
    pub(crate) optional: bool,
}

/// The types the module declares: each enum by its type and labels, and each
/// created range by its type, its subtype's kind and whether it is the
/// multirange.
#[derive(Clone, Debug, Default)]
pub(crate) struct DeclaredTypes {
    pub(crate) enums: BTreeMap<TypeRef, Vec<String>>,
    pub(crate) ranges: BTreeMap<TypeRef, (&'static str, bool)>,
}

/// Everything a declared-model module declares, in its order.
#[derive(Clone, Debug)]
pub(crate) struct Declared<'a> {
    pub(crate) types: DeclaredTypes,
    pub(crate) models: Vec<DeclaredModel<'a>>,
    pub(crate) options: &'a ModelWriterOptions,
}

impl<'a> Declared<'a> {
    /// Read `entities` as declared models, refusing what a declared model
    /// cannot say.
    // [spec:pgorm:req:codegen.models.refused]
    pub(crate) fn read(
        entities: &'a Entities,
        options: &'a ModelWriterOptions,
    ) -> Result<Self, Error> {
        let schema = options.schema_name.as_deref().unwrap_or("public");
        let mut types = DeclaredTypes::default();
        for active_enum in entities.enums.values() {
            types.enums.insert(
                type_ref(&active_enum.enum_name, &active_enum.schema, schema),
                active_enum
                    .values
                    .iter()
                    .map(|name| name.to_string())
                    .collect(),
            );
        }
        for range in entities.ranges.values() {
            let subtype = scalar_kind(&range.subtype).ok_or_else(|| {
                Error::TransformError(format!(
                    "range type `{}` is over a type no declared model's created range takes",
                    range.name.to_string()
                ))
            })?;
            types.ranges.insert(
                type_ref(&range.name, &range.schema, schema),
                (subtype.kind_name(), range.multirange),
            );
        }
        let models = entities
            .entities
            .iter()
            .map(|entity| model(entity, schema))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            types,
            models,
            options,
        })
    }

    /// The model of the table `ident` names.
    pub(crate) fn model(&self, ident: &TableIdent) -> Option<&DeclaredModel<'a>> {
        self.models
            .iter()
            .find(|model| model.entity.ident() == *ident)
    }

    /// Every model's relations, in model order, each model's in its Rust
    /// `Relation` enum's order.
    // [spec:pgorm:sem:codegen.models.relations]
    pub(crate) fn relations(&self) -> Result<Vec<DeclaredRelation<'_>>, Error> {
        let mut relations = Vec::new();
        for from in &self.models {
            for relation in &from.entity.relations {
                relations.push(self.relation(from, relation)?);
            }
        }
        Ok(relations)
    }

    fn relation<'m>(
        &'m self,
        from: &'m DeclaredModel<'a>,
        relation: &Relation,
    ) -> Result<DeclaredRelation<'m>, Error> {
        let table = from.entity.ident();
        let target = relation.ref_ident();
        let context = format!("table `{table}`: its relation to `{target}`");
        let to = self.model(&target).ok_or_else(|| {
            Error::TransformError(format!("{context} names a table the schema lacks"))
        })?;
        let (from_columns, to_columns) = match relation.rel_type {
            RelationType::BelongsTo => {
                refuse_unsaid_key_parts(relation, &context)?;
                (relation.columns.clone(), relation.ref_columns.clone())
            }
            RelationType::HasOne | RelationType::HasMany => {
                let key = to
                    .entity
                    .relations
                    .iter()
                    .find(|key| {
                        matches!(key.rel_type, RelationType::BelongsTo)
                            && key.ref_ident() == table
                            && key.num_suffix == 0
                            && !key.self_referencing
                    })
                    .ok_or_else(|| {
                        Error::TransformError(format!(
                            "{context} reads a foreign key `{target}` does not hold"
                        ))
                    })?;
                (key.ref_columns.clone(), key.columns.clone())
            }
        };
        let variant = if relation.self_referencing {
            "SelfRef".to_owned()
        } else {
            relation.ref_table.to_upper_camel_case()
        };
        let variant = match relation.num_suffix {
            0 => variant,
            n => format!("{variant}{n}"),
        };
        Ok(DeclaredRelation {
            from,
            to,
            kind: relation.rel_type.clone(),
            variant,
            from_columns,
            to_columns,
            on_delete: key_action(relation, &relation.on_delete),
            on_update: key_action(relation, &relation.on_update),
        })
    }

    /// Every junction path, each a model's conjunct relation: the index of
    /// the junction's relation to the far end and of the model's to the
    /// junction among `relations`.
    // [spec:pgorm:sem:codegen.models.relations]
    pub(crate) fn related<'m>(
        &'m self,
        relations: &[DeclaredRelation<'m>],
    ) -> Result<Vec<DeclaredRelated<'m>>, Error> {
        let mut related = Vec::new();
        for from in &self.models {
            for conjunct in &from.entity.conjunct_relations {
                let context = format!(
                    "table `{}`: its path to `{}` through `{}`",
                    from.entity.ident(),
                    conjunct.to,
                    conjunct.via
                );
                let find = |start: &str, end: &str, kinds: &[&str]| {
                    relations.iter().position(|relation| {
                        relation.from.entity.table_name == start
                            && relation.to.entity.table_name == end
                            && kinds.contains(&relation_kind(&relation.kind))
                    })
                };
                let relation = find(&conjunct.via, &conjunct.to, &["belongs_to"]);
                let via = find(
                    &from.entity.table_name,
                    &conjunct.via,
                    &["has_one", "has_many"],
                );
                let (Some(relation), Some(via)) = (relation, via) else {
                    return Err(Error::TransformError(format!(
                        "{context} has no relation to the junction a declared model can follow"
                    )));
                };
                let to = relations
                    .get(relation)
                    .map(|relation| relation.to)
                    .ok_or_else(|| Error::TransformError(context.clone()))?;
                related.push(DeclaredRelated {
                    from,
                    to,
                    relation,
                    via,
                });
            }
        }
        Ok(related)
    }

    /// Every disjoint model, in model order.
    // [spec:pgorm:sem:codegen.models.disjoint]
    pub(crate) fn disjoints(&self) -> Vec<DeclaredDisjoint<'_>> {
        let schema = self.options.schema_name.as_deref().unwrap_or("public");
        self.models
            .iter()
            .filter_map(|base| {
                let disjoint = base.entity.disjoint.as_ref()?;
                let name = base.name.to_snake_case();
                let kinded = disjoint.kinded.as_ref().map(|kinded| {
                    let kind_type = kinded.kind_type.to_string();
                    let enum_name = (kind_type != format!("{name}_kind")).then_some(kind_type);
                    let kind_schema = kinded
                        .kind_schema
                        .as_ref()
                        .map_or_else(|| schema.to_owned(), |name| name.to_string());
                    (kinded.column.clone(), enum_name, kind_schema)
                });
                let kinds = disjoint
                    .kinds
                    .iter()
                    .map(|kind| DeclaredKind {
                        value: kind
                            .value
                            .clone()
                            .unwrap_or_else(|| kind.variant.to_snake_case()),
                        model: kind.ident.as_ref().and_then(|ident| self.model(ident)),
                    })
                    .collect();
                Some(DeclaredDisjoint {
                    base,
                    name,
                    kinded,
                    kinds,
                })
            })
            .collect()
    }

    /// Every one-of reference, in model order and each model's in its
    /// `CHECK`s' order.
    // [spec:pgorm:sem:codegen.models.one-of]
    pub(crate) fn one_ofs(&self) -> Result<Vec<DeclaredOneOf<'_>>, Error> {
        let mut found = Vec::new();
        for model in &self.models {
            for one_of in &model.entity.one_of {
                found.push(self.one_of(model, one_of)?);
            }
        }
        Ok(found)
    }

    fn one_of<'m>(
        &'m self,
        model: &'m DeclaredModel<'a>,
        one_of: &OneOfEnum,
    ) -> Result<DeclaredOneOf<'m>, Error> {
        let targets = one_of
            .targets
            .iter()
            .map(|target| {
                let reached = self.model(&target.target).ok_or_else(|| {
                    Error::TransformError(format!(
                        "table `{}`: its one-of reference `{}` reaches `{}`, which the schema lacks",
                        model.entity.ident(),
                        one_of.reference,
                        target.target
                    ))
                })?;
                Ok(DeclaredTarget {
                    value: target.variant.trim_start_matches("r#").to_snake_case(),
                    column: target.column.clone(),
                    model: reached,
                })
            })
            .collect::<Result<_, Error>>()?;
        Ok(DeclaredOneOf {
            model,
            name: one_of.reference.clone(),
            rust_name: one_of.name.trim_start_matches("r#").to_owned(),
            targets,
            optional: one_of.none,
        })
    }
}

/// A `belongs_to` relation's foreign-key action, which its table's key
/// takes; the other side's relation holds no key, and says none.
fn key_action(relation: &Relation, action: &Option<ForeignKeyAction>) -> Option<&'static str> {
    match relation.rel_type {
        RelationType::BelongsTo => action.as_ref().and_then(self::action),
        RelationType::HasOne | RelationType::HasMany => None,
    }
}

/// The neutral name of a relation's kind.
pub(crate) fn relation_kind(kind: &RelationType) -> &'static str {
    match kind {
        RelationType::BelongsTo => "belongs_to",
        RelationType::HasOne => "has_one",
        RelationType::HasMany => "has_many",
    }
}

/// A foreign key's parts a declared model's relation has no word for — a
/// deferrable or unenforced key, a temporal key's period — refused, since a
/// relation declared without them would write another key.
// [spec:pgorm:req:codegen.models.refused]
fn refuse_unsaid_key_parts(relation: &Relation, context: &str) -> Result<(), Error> {
    let unsaid = match (
        &relation.deferrability,
        &relation.enforcement,
        &relation.period,
    ) {
        (Some(Deferrability::DeferrableInitiallyDeferred), _, _) => "DEFERRABLE INITIALLY DEFERRED",
        (Some(Deferrability::DeferrableInitiallyImmediate), _, _) => {
            "DEFERRABLE INITIALLY IMMEDIATE"
        }
        (_, Some(Enforcement::NotEnforced), _) => "NOT ENFORCED",
        (_, _, Some(_)) => "a PERIOD",
        _ => return Ok(()),
    };
    Err(Error::TransformError(format!(
        "{context}: its foreign key is {unsaid}, which a declared model's relation cannot say"
    )))
}

fn type_ref(name: &Name, schema: &Option<Name>, default: &str) -> TypeRef {
    TypeRef {
        name: name.to_string(),
        schema: schema
            .as_ref()
            .map_or_else(|| default.to_owned(), |name| name.to_string()),
    }
}

/// The scalar kind a column of `col_type` decodes as, if it is a scalar a
/// declared model takes.
fn scalar_kind(col_type: &ColumnType) -> Option<ArrayType> {
    Some(match col_type {
        ColumnType::Char(_) | ColumnType::String(_) | ColumnType::Text => ArrayType::String,
        ColumnType::SmallInteger => ArrayType::SmallInt,
        ColumnType::Integer => ArrayType::Int,
        ColumnType::BigInteger => ArrayType::BigInt,
        ColumnType::Float => ArrayType::Float,
        ColumnType::Double => ArrayType::Double,
        ColumnType::Json | ColumnType::JsonBinary => ArrayType::Json,
        ColumnType::Date => ArrayType::Date,
        ColumnType::Time => ArrayType::Time,
        ColumnType::Timestamp => ArrayType::DateTime,
        ColumnType::TimestampWithTimeZone => ArrayType::DateTimeWithTimeZone,
        ColumnType::Decimal(_) | ColumnType::Money => ArrayType::Decimal,
        ColumnType::Uuid => ArrayType::Uuid,
        ColumnType::Bytea => ArrayType::Bytes,
        ColumnType::Boolean => ArrayType::Bool,
        ColumnType::Interval(_) => ArrayType::Interval,
        ColumnType::Range(range) => ArrayType::Range(*range),
        ColumnType::Multirange(range) => ArrayType::Multirange(*range),
        _ => return None,
    })
}

/// The declared model of `entity`.
fn model<'a>(entity: &'a Entity, schema: &str) -> Result<DeclaredModel<'a>, Error> {
    let table = entity.ident();
    let last_key = entity.primary_keys.last().map(|key| key.name.as_str());
    let columns = entity
        .columns
        .iter()
        .map(|column| {
            let primary_key = entity
                .primary_keys
                .iter()
                .any(|key| key.name == column.name);
            let without_overlaps =
                entity.without_overlaps && last_key == Some(column.name.as_str());
            declared_column(column, primary_key, without_overlaps, schema).map_err(|problem| {
                Error::TransformError(format!(
                    "table `{table}`: column `{}` {problem}",
                    column.name
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    let single = entity
        .columns
        .iter()
        .filter(|column| column.unique)
        .map(|column| DeclaredKey {
            columns: vec![column.name.clone()],
            name: None,
            nulls_not_distinct: false,
        });
    let unique_keys = single
        .chain(entity.unique_keys.iter().map(|key| DeclaredKey {
            columns: key.columns.clone(),
            name: key.name.clone(),
            nulls_not_distinct: key.nulls_not_distinct,
        }))
        .collect();
    Ok(DeclaredModel {
        entity,
        name: entity.table_name.to_upper_camel_case(),
        columns,
        unique_keys,
    })
}

/// `column` as a declared column, or what keeps it from being one.
// [spec:pgorm:sem:codegen.models.columns]
fn declared_column<'a>(
    column: &'a Column,
    primary_key: bool,
    without_overlaps: bool,
    schema: &str,
) -> Result<DeclaredColumn<'a>, String> {
    let (element, array) = match &column.col_type {
        ColumnType::Array(element) => (element.as_ref(), true),
        other => (other, false),
    };
    let values = match under_domains(element) {
        ColumnType::Enum { variants, .. } => {
            Some(variants.iter().map(|name| name.to_string()).collect())
        }
        _ => None,
    };
    let (kind, stated) = kind_of(element, schema)?;
    let fill = fill(column, &kind, primary_key, array)?;
    Ok(DeclaredColumn {
        column,
        kind,
        array,
        nullable: !column.not_null && !primary_key,
        primary_key,
        values,
        column_type: stated,
        fill,
        without_overlaps,
    })
}

/// The kind a column of `col_type` is declared with, and the column type it
/// states where the kind's own is not `col_type`.
fn kind_of(col_type: &ColumnType, schema: &str) -> Result<(Kind, Option<DdlType>), String> {
    match col_type {
        ColumnType::Array(_) => {
            Err("is an array of arrays, which a declared model cannot hold".into())
        }
        ColumnType::Domain { base, .. } => {
            if matches!(under_domains(base), ColumnType::Array(_)) {
                return Err(
                    "is typed as a domain over an array, which a declared model's column_type \
                     cannot state: an array column's is its elements'"
                        .into(),
                );
            }
            let (kind, _) = kind_of(base, schema)?;
            if matches!(kind, Kind::Created(_)) {
                return Err(
                    "is typed as a domain over a created range, whose type a declared model \
                     takes as its kind's own"
                        .into(),
                );
            }
            Ok((kind, Some(spell(col_type, schema)?)))
        }
        ColumnType::Enum {
            name, schema: own, ..
        } => Ok((Kind::Enum(type_ref(name, own, schema)), None)),
        ColumnType::CreatedRange {
            name, schema: own, ..
        }
        | ColumnType::CreatedMultirange {
            name, schema: own, ..
        } => Ok((Kind::Created(type_ref(name, own, schema)), None)),
        other => {
            let kind = scalar_kind(other)
                .ok_or_else(|| format!("is of a type no declared model's kind takes: {other:?}"))?;
            let own = kind.column_type();
            let stated = match own {
                Some(own) if own == *other => None,
                _ => Some(spell(other, schema)?),
            };
            Ok((Kind::Scalar(kind.kind_name()), stated))
        }
    }
}

/// `col_type` as the bindings' `DataType` spells it.
fn spell(col_type: &ColumnType, schema: &str) -> Result<DdlType, String> {
    Ok(match col_type {
        ColumnType::Char(Some(length)) => DdlType::Sized("char", *length),
        ColumnType::Char(None) => {
            return Err("is a `bpchar`, which no declared model's column_type spells".into());
        }
        ColumnType::String(StringLen::None) => DdlType::Builtin("varchar"),
        ColumnType::String(StringLen::N(length)) => DdlType::Sized("varchar", *length),
        ColumnType::Text => DdlType::Builtin("text"),
        ColumnType::SmallInteger => DdlType::Builtin("smallint"),
        ColumnType::Integer => DdlType::Builtin("integer"),
        ColumnType::BigInteger => DdlType::Builtin("bigint"),
        ColumnType::Float => DdlType::Builtin("real"),
        ColumnType::Double => DdlType::Builtin("double"),
        ColumnType::Decimal(None) => DdlType::Builtin("numeric"),
        ColumnType::Decimal(Some((precision, scale))) => DdlType::Numeric(*precision, *scale),
        ColumnType::Money => DdlType::Builtin("money"),
        ColumnType::Timestamp => DdlType::Builtin("timestamp"),
        ColumnType::TimestampWithTimeZone => DdlType::Builtin("timestamptz"),
        ColumnType::Time => DdlType::Builtin("time"),
        ColumnType::Date => DdlType::Builtin("date"),
        ColumnType::Interval(spec) => interval(spec),
        ColumnType::Boolean => DdlType::Builtin("boolean"),
        ColumnType::Json => DdlType::Builtin("json"),
        ColumnType::JsonBinary => DdlType::Builtin("jsonb"),
        ColumnType::Uuid => DdlType::Builtin("uuid"),
        ColumnType::Bytea => DdlType::Builtin("bytea"),
        ColumnType::Range(range) => DdlType::Builtin(range.range_type_name()),
        ColumnType::Multirange(range) => DdlType::Builtin(range.multirange_type_name()),
        ColumnType::Enum {
            name, schema: own, ..
        } => DdlType::Enum(type_ref(name, own, schema)),
        ColumnType::CreatedRange {
            name, schema: own, ..
        }
        | ColumnType::CreatedMultirange {
            name, schema: own, ..
        } => DdlType::Created(type_ref(name, own, schema)),
        ColumnType::Domain {
            name,
            schema: own,
            base,
        } => DdlType::Domain {
            name: name.to_string(),
            schema: own.as_ref().map(|name| name.to_string()),
            base: Box::new(spell(base, schema)?),
        },
        other => {
            return Err(format!(
                "is of a type no declared model's column_type spells: {other:?}"
            ));
        }
    })
}

/// An interval's type: its fields in snake_case, as the bindings spell
/// them, and its precision.
fn interval(spec: &IntervalSpec) -> DdlType {
    let (fields, precision) = match spec {
        IntervalSpec::Any(precision) => (None, *precision),
        IntervalSpec::Fields(fields) => match fields {
            PgInterval::Year => (Some("year"), None),
            PgInterval::Month => (Some("month"), None),
            PgInterval::Day => (Some("day"), None),
            PgInterval::Hour => (Some("hour"), None),
            PgInterval::Minute => (Some("minute"), None),
            PgInterval::Second(precision) => (Some("second"), *precision),
            PgInterval::YearToMonth => (Some("year_to_month"), None),
            PgInterval::DayToHour => (Some("day_to_hour"), None),
            PgInterval::DayToMinute => (Some("day_to_minute"), None),
            PgInterval::DayToSecond(precision) => (Some("day_to_second"), *precision),
            PgInterval::HourToMinute => (Some("hour_to_minute"), None),
            PgInterval::HourToSecond(precision) => (Some("hour_to_second"), *precision),
            PgInterval::MinuteToSecond(precision) => (Some("minute_to_second"), *precision),
        },
    };
    match (fields, precision) {
        (None, None) => DdlType::Builtin("interval"),
        (fields, precision) => DdlType::Interval(fields, precision.map(|p| p.digits())),
    }
}

/// What fills `column`, or why a declared model cannot say it.
fn fill<'a>(
    column: &'a Column,
    kind: &Kind,
    primary_key: bool,
    array: bool,
) -> Result<Fill<'a>, String> {
    if column.auto_increment {
        let serial = matches!(kind, Kind::Scalar("i16" | "i32" | "i64"));
        if !primary_key || array || !serial {
            return Err(
                "is drawn from a sequence outside an integer primary key, which a declared \
                 model's auto_increment cannot say"
                    .into(),
            );
        }
        return Ok(Fill::AutoIncrement);
    }
    if let Some(identity) = column.identity {
        return Ok(Fill::Identity(identity));
    }
    if let Some((expr, generated)) = &column.generated {
        return Ok(Fill::Generated(expr, *generated));
    }
    let Some(default) = &column.default else {
        return Ok(Fill::None);
    };
    if matches!(default, ColumnExpr::Null) {
        return Err("defaults to NULL, which a declared model has no word for".into());
    }
    let Some(literal) = literal(default) else {
        return Ok(Fill::Expr(default));
    };
    Ok(match (kind, array, literal) {
        (_, true, literal) => Fill::Bound(literal),
        (Kind::Scalar("i16" | "i32" | "i64" | "decimal"), _, Literal::Int(int)) => {
            Fill::Value(Literal::Int(int))
        }
        (Kind::Scalar("f32" | "f64"), _, Literal::Int(int)) => {
            Fill::Value(Literal::Float(format!("{int}.0")))
        }
        (Kind::Scalar("f32" | "f64"), _, Literal::Decimal(text)) => {
            Fill::Value(Literal::Float(text))
        }
        (Kind::Scalar("decimal"), _, Literal::Decimal(text)) => Fill::Value(Literal::Decimal(text)),
        (Kind::Scalar("text") | Kind::Enum(_), _, Literal::Text(text)) => {
            Fill::Value(Literal::Text(text))
        }
        (Kind::Scalar("bool"), _, Literal::Bool(value)) => Fill::Value(Literal::Bool(value)),
        (_, _, literal) => Fill::Bound(literal),
    })
}

/// A literal expression as the value it is.
pub(crate) fn literal(expr: &ColumnExpr) -> Option<Literal> {
    Some(match expr {
        ColumnExpr::Int(int) => Literal::Int(i64::from(*int)),
        ColumnExpr::BigInt(big) => Literal::Int(*big),
        ColumnExpr::Decimal(mantissa, scale) => {
            Literal::Decimal(rust_decimal::Decimal::new(*mantissa, *scale).to_string())
        }
        ColumnExpr::Text(text) => Literal::Text(text.clone()),
        ColumnExpr::Bool(value) => Literal::Bool(*value),
        _ => return None,
    })
}

/// Names a module claims, each with what claimed it, so a second claim on
/// one is refused naming both.
#[derive(Default)]
struct Claims(BTreeMap<String, String>);

impl Claims {
    // [spec:pgorm:req:codegen.models.refused]
    fn claim(&mut self, name: &str, what: String) -> Result<(), Error> {
        if let Some(first) = self.0.get(name) {
            return Err(Error::TransformError(format!(
                "{what} is named `{name}`, as {first} is"
            )));
        }
        self.0.insert(name.to_owned(), what);
        Ok(())
    }
}

/// How a language names what a declared-model module declares, each from
/// the snake_case or UpperCamelCase parts both languages start from.
pub(crate) struct Naming {
    pub(crate) language: &'static str,
    /// The words no declared name is; one of them takes a trailing `_`.
    pub(crate) keywords: &'static [&'static str],
    /// Every name the module may import, which no declared name takes.
    pub(crate) importable: &'static [&'static str],
    /// Whether `$` is an identifier character.
    pub(crate) dollar: bool,
    /// A name of snake_case parts joined by `_`: the language's
    /// snake_case or lowerCamelCase.
    pub(crate) joined: fn(&str) -> String,
    /// A type's constant, from its name, schema-prefixed outside the
    /// default schema.
    pub(crate) constant: fn(&str) -> String,
}

/// The name each declared thing takes in one module, each list in the
/// order of what it names.
#[derive(Debug, Default)]
pub(crate) struct Names {
    pub(crate) types: BTreeMap<TypeRef, String>,
    pub(crate) models: Vec<String>,
    pub(crate) relations: Vec<String>,
    pub(crate) related: Vec<String>,
    pub(crate) disjoints: Vec<String>,
    pub(crate) one_ofs: Vec<String>,
}

/// A schema's declared models, every part read and named.
pub(crate) struct Plan<'d> {
    pub(crate) declared: &'d Declared<'d>,
    pub(crate) relations: Vec<DeclaredRelation<'d>>,
    pub(crate) related: Vec<DeclaredRelated<'d>>,
    pub(crate) disjoints: Vec<DeclaredDisjoint<'d>>,
    pub(crate) one_ofs: Vec<DeclaredOneOf<'d>>,
    pub(crate) names: Names,
}

impl<'d> Plan<'d> {
    /// Every part of `declared`, named as `naming` names them: a model for
    /// its table, a relation for the model it starts at and its Rust
    /// variant, a junction path for its two ends, a disjoint model for its
    /// base, a one-of reference for its model and Rust enum, a type for its
    /// name. A name that is no identifier, or that two things take, is
    /// refused.
    // [spec:pgorm:sem:codegen.models.names]
    // [spec:pgorm:req:codegen.models.refused]
    pub(crate) fn new(declared: &'d Declared<'d>, naming: &Naming) -> Result<Self, Error> {
        let relations = declared.relations()?;
        let related = declared.related(&relations)?;
        let disjoints = declared.disjoints();
        let one_ofs = declared.one_ofs()?;
        let mut claims = Claims::default();
        for name in naming.importable {
            claims.claim(name, format!("the `{name}` the module may import"))?;
        }
        let mut claim = |name: String, what: String| -> Result<String, Error> {
            let name = if naming.keywords.contains(&name.as_str()) {
                format!("{name}_")
            } else {
                name
            };
            if !is_identifier(&name, naming.dollar) {
                return Err(Error::TransformError(format!(
                    "{what} would be named `{name}`, which is no {} identifier",
                    naming.language
                )));
            }
            claims.claim(&name, what)?;
            Ok(name)
        };
        let default_schema = declared.options.schema_name.as_deref().unwrap_or("public");
        let mut names = Names::default();
        let enums = declared.types.enums.keys().map(|ty| (ty, "enum"));
        let ranges = declared.types.ranges.keys().map(|ty| (ty, "range type"));
        for (ty, what) in enums.chain(ranges) {
            let bare = match ty.schema == default_schema {
                true => ty.name.clone(),
                false => format!("{}_{}", ty.schema, ty.name),
            };
            let name = claim(
                (naming.constant)(&bare),
                format!("the {what} `{}.{}`", ty.schema, ty.name),
            )?;
            names.types.insert(ty.clone(), name);
        }
        for model in &declared.models {
            names.models.push(claim(
                model.name.clone(),
                format!("the model of table `{}`", model.entity.ident()),
            )?);
        }
        for relation in &relations {
            names.relations.push(claim(
                (naming.joined)(&format!(
                    "{}_{}",
                    relation.from.entity.table_name.to_snake_case(),
                    relation.variant.to_snake_case()
                )),
                format!(
                    "the relation of table `{}` to `{}`",
                    relation.from.entity.ident(),
                    relation.to.entity.ident()
                ),
            )?);
        }
        for path in &related {
            names.related.push(claim(
                (naming.joined)(&format!(
                    "{}_related_{}",
                    path.from.entity.table_name.to_snake_case(),
                    path.to.entity.table_name.to_snake_case()
                )),
                format!(
                    "the path of table `{}` to `{}` through a junction",
                    path.from.entity.ident(),
                    path.to.entity.ident()
                ),
            )?);
        }
        let mut plan = Self {
            declared,
            relations,
            related,
            disjoints,
            one_ofs,
            names,
        };
        for at in 0..plan.disjoints.len() {
            let Some(disjoint) = plan.disjoints.get(at) else {
                continue;
            };
            let name = format!("{}Disjoint", plan.model(disjoint.base));
            let what = format!(
                "the disjoint model over table `{}`",
                disjoint.base.entity.ident()
            );
            plan.names.disjoints.push(claim(name, what)?);
        }
        for at in 0..plan.one_ofs.len() {
            let Some(one_of) = plan.one_ofs.get(at) else {
                continue;
            };
            let name = format!("{}{}", plan.model(one_of.model), one_of.rust_name);
            let what = format!(
                "the one-of reference `{}` of table `{}`",
                one_of.name,
                one_of.model.entity.ident()
            );
            plan.names.one_ofs.push(claim(name, what)?);
        }
        Ok(plan)
    }

    /// The name of `model`, one of the plan's.
    pub(crate) fn model(&self, model: &DeclaredModel<'_>) -> String {
        self.declared
            .models
            .iter()
            .position(|candidate| std::ptr::eq(candidate, model))
            .and_then(|at| self.names.models.get(at))
            .cloned()
            .unwrap_or_default()
    }

    /// The name of the type `ty`, one the plan declares.
    pub(crate) fn type_name(&self, ty: &TypeRef) -> String {
        self.names.types.get(ty).cloned().unwrap_or_default()
    }
}

/// Whether `name` is an identifier: a letter, `_` or, where `dollar` says
/// so, `$`, then letters, digits, `_` and `$`, as Unicode's identifier
/// properties have them.
fn is_identifier(name: &str, dollar: bool) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let extra = |c: char| c == '_' || (dollar && c == '$');
    (unicode_ident::is_xid_start(first) || extra(first))
        && chars.all(|c| unicode_ident::is_xid_continue(c) || extra(c))
}
