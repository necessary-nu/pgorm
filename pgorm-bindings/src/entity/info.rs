//! What a binding knows of a registered entity: its table, its columns, each
//! one's SQL name, nullability, key membership and the kind its values bind
//! and read as, and its relations as `describe()` reports them.

use std::sync::Arc;

use pgorm::pgorm_query::{ArrayType, ConditionType, Deferrability, Enforcement, FromItem, Value};
use pgorm::{
    ColumnTrait, EntityTrait, FromQueryResult, Iterable, PrimaryKeyToColumn, PrimaryKeyTrait,
    RelationDef, RelationTrait, RelationType, StaticName,
};
use serde_json::{Map, Value as Json};

use crate::{Dialect, Failure, Tag, kind::labels};

/// A JSON object whose keys are neutral names, spelled as `dialect` spells
/// them.
pub fn object<const N: usize>(dialect: Dialect, fields: [(&str, Json); N]) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(key, value)| (dialect.option(key), value))
            .collect::<Map<_, _>>(),
    )
}

/// One column of a registered entity.
#[derive(Clone, Debug)]
pub struct ColumnInfo {
    /// The column's name in its table.
    pub name: String,
    /// The key the Rust field serializes under, `ColumnTrait::json_key`.
    pub json_key: String,
    /// The `ColumnType` it declares, as Rust's `Debug` writes it.
    pub sql_type: String,
    /// The Rust type of its field, where the model reflects it.
    pub rust_type: Option<&'static str>,
    /// Whether the column takes null.
    pub nullable: bool,
    /// Whether it is part of the primary key.
    pub primary_key: bool,
    /// The kind its values bind and read as, or `None` for a type no kind
    /// names, whose values are given tagged.
    pub tag: Option<Tag>,
    /// An enum column's labels, an array's or a domain's over one included.
    pub labels: Option<Vec<String>>,
}

impl ColumnInfo {
    /// The kind `value`, read from the column, carries: the column's own
    /// where it names a type, an enum's or a created range's, that the
    /// value, a string or an array of strings, does not carry, and otherwise
    /// the value's own. A value of another shape than such a column holds is
    /// refused.
    pub fn tag_of(&self, value: &Value) -> Result<Tag, Failure> {
        let incompatible = |what: &str| {
            Failure::Decode(format!(
                "compiled {what} returned an incompatible Rust Value"
            ))
        };
        Ok(match &self.tag {
            Some(tag @ Tag::Enum(_)) => match value {
                Value::String(_) => tag.clone(),
                _ => return Err(incompatible("enum")),
            },
            Some(tag @ Tag::Created(_)) => match value {
                Value::String(_) => tag.clone(),
                _ => return Err(incompatible("created range")),
            },
            Some(tag @ Tag::Array(element)) => match (element.as_ref(), value) {
                (Tag::Enum(_) | Tag::Created(_), Value::Array(ArrayType::String, _)) => tag.clone(),
                (Tag::Enum(_), _) => return Err(incompatible("enum array")),
                (Tag::Created(_), _) => return Err(incompatible("created range array")),
                _ => Tag::of_value(value),
            },
            _ => Tag::of_value(value),
        })
    }

    fn describe(&self, dialect: Dialect) -> Json {
        object(
            dialect,
            [
                ("name", self.name.clone().into()),
                ("json_key", self.json_key.clone().into()),
                ("sql_type", self.sql_type.clone().into()),
                ("rust_type", self.rust_type.into()),
                ("nullable", self.nullable.into()),
                ("primary_key", self.primary_key.into()),
                ("kind", self.tag.as_ref().map(Tag::key).into()),
                ("values", self.labels.clone().into()),
            ],
        )
    }
}

/// A registration name: 1–255 UTF-8 bytes without NUL.
pub fn registration_name(name: &str) -> Result<(), Failure> {
    if name.is_empty() || name.len() > 255 || name.contains('\0') {
        return Err(Failure::Construction(
            "a registration's name is 1–255 UTF-8 bytes without NUL".to_owned(),
        ));
    }
    Ok(())
}

/// An identifier part an entity names: 1–63 UTF-8 bytes without NUL, as
/// PostgreSQL keeps one.
fn identifier(name: &str) -> Result<(), Failure> {
    if name.is_empty() || name.contains('\0') || name.len() > 63 {
        return Err(Failure::Construction(format!(
            "{name:?} is no identifier: identifier parts are 1–63 UTF-8 bytes without NUL"
        )));
    }
    Ok(())
}

/// A registered entity: its registration name, table, columns, key and
/// relations, and the Rust types behind it.
#[derive(Debug)]
pub struct EntityInfo {
    /// The name the registration is found by.
    pub name: String,
    /// The table's schema, the entity instance's own.
    pub schema: Option<String>,
    /// The table, the entity instance's own.
    pub table: String,
    /// The columns, in the entity's `Column` order.
    pub columns: Vec<ColumnInfo>,
    /// The primary key's columns, in order.
    pub primary_key: Vec<String>,
    /// Whether the key's last part is a period matched `WITHOUT OVERLAPS`.
    /// A lookup, update or delete by the key still compares it for equality.
    pub without_overlaps: bool,
    /// The table's unique keys over two columns or more, as the entity's
    /// `EntityName::unique_keys` declares them.
    pub unique_keys: &'static [pgorm::UniqueKey],
    relations: Vec<(String, RelationDef)>,
    rust: [&'static str; 4],
    /// The registered entity value whose names the registration's statements
    /// read, kept type-erased because the metadata is shared by every adapter
    /// of the registration whatever its Rust type.
    entity: Arc<dyn std::any::Any + Send + Sync>,
}

impl EntityInfo {
    /// The metadata of `entity` registered under `name`, its table and
    /// schema the instance's own.
    // [spec:pgorm:req:bindings.entities+6]
    pub fn of<E>(name: &str, entity: E) -> Result<Self, Failure>
    where
        E: EntityTrait + Send + Sync + 'static,
    {
        registration_name(name)?;
        let table = entity.table_name().to_owned();
        let schema = entity.schema_name().map(str::to_owned);
        identifier(&table)?;
        if let Some(schema) = &schema {
            identifier(schema)?;
        }
        let primary_key: Vec<String> = <E::PrimaryKey as Iterable>::iter()
            .map(|key| key.into_column().as_str().to_owned())
            .collect();
        let reflected = E::Model::expected_columns();
        let mut columns: Vec<ColumnInfo> = Vec::new();
        for column in <E::Column as Iterable>::iter() {
            let column_name = column.as_str().to_owned();
            identifier(&column_name)?;
            if columns.iter().any(|seen| seen.name == column_name) {
                return Err(Failure::Construction(format!(
                    "{name} names the column {column_name:?} twice"
                )));
            }
            let definition = column.def();
            columns.push(ColumnInfo {
                rust_type: reflected.as_ref().and_then(|fields| {
                    fields
                        .iter()
                        .find(|field| field.name() == column_name)
                        .map(|field| field.rust_type())
                }),
                primary_key: primary_key.contains(&column_name),
                json_key: column.json_key().to_owned(),
                sql_type: format!("{:?}", definition.get_column_type()),
                nullable: definition.is_null(),
                tag: Tag::of_column(definition.get_column_type()),
                labels: labels(definition.get_column_type()),
                name: column_name,
            });
        }
        if columns.is_empty() {
            return Err(Failure::Construction(format!("{name} has no column")));
        }
        Ok(Self {
            name: name.to_owned(),
            schema,
            table,
            columns,
            primary_key,
            without_overlaps: <E::PrimaryKey as PrimaryKeyTrait>::without_overlaps(),
            unique_keys: entity.unique_keys(),
            relations: <E::Relation as Iterable>::iter()
                .map(|relation| (format!("{relation:?}"), relation.def()))
                .collect(),
            rust: [
                std::any::type_name::<E>(),
                std::any::type_name::<E::Model>(),
                std::any::type_name::<E::ActiveModel>(),
                std::any::type_name::<E::Column>(),
            ],
            entity: Arc::new(entity),
        })
    }

    /// The entity value this registration's statements name their table by,
    /// refused for a type the registration was not made for.
    // [spec:pgorm:req:bindings.entities+6]
    pub fn entity<E: EntityTrait>(&self) -> Result<E, pgorm::Error> {
        self.entity.downcast_ref::<E>().copied().ok_or_else(|| {
            pgorm::Error::Custom("the ActiveModel belongs to another registration".to_owned())
        })
    }

    /// The column whose SQL name is `name`, refused as one the registration
    /// lacks.
    pub fn column(&self, name: &str) -> Result<&ColumnInfo, Failure> {
        self.columns
            .iter()
            .find(|column| column.name == name)
            .ok_or_else(|| Failure::Construction(format!("{} has no column {name:?}", self.name)))
    }

    /// The registration as `describe()` gives it, its keys and closed
    /// choices spelled as `dialect` spells them.
    // [spec:pgorm:req:bindings.entities+6]
    pub fn describe(&self, dialect: Dialect) -> Json {
        object(
            dialect,
            [
                ("name", self.name.clone().into()),
                ("schema", self.schema.clone().into()),
                ("table", self.table.clone().into()),
                (
                    "columns",
                    self.columns
                        .iter()
                        .map(|column| column.describe(dialect))
                        .collect::<Vec<_>>()
                        .into(),
                ),
                ("primary_key", self.primary_key.clone().into()),
                ("primary_key_without_overlaps", self.without_overlaps.into()),
                (
                    "unique_keys",
                    self.unique_keys
                        .iter()
                        .map(|key| {
                            object(
                                dialect,
                                [
                                    ("name", key.name.into()),
                                    ("columns", key.columns.to_vec().into()),
                                    ("nulls_not_distinct", key.nulls_not_distinct.into()),
                                ],
                            )
                        })
                        .collect::<Vec<_>>()
                        .into(),
                ),
                (
                    "relations",
                    self.relations
                        .iter()
                        .map(|(name, def)| relation(dialect, name, def))
                        .collect::<Vec<_>>()
                        .into(),
                ),
                (
                    "rust",
                    object(
                        dialect,
                        [
                            ("entity", self.rust[0].into()),
                            ("model", self.rust[1].into()),
                            ("active_model", self.rust[2].into()),
                            ("column", self.rust[3].into()),
                        ],
                    ),
                ),
            ],
        )
    }
}

/// A relation as `describe()` reports it: the parts of a compiled
/// declaration that change what a join along it means, and what the server
/// holds its foreign key's rows to. `on_condition` is present or absent only:
/// it is a Rust closure over the two tables' aliases, which has no rendering
/// of its own until a join supplies them.
// [spec:pgorm:req:bindings.entities+6]
// [spec:pgorm:req:bindings.relation-conditions]
pub(crate) fn relation(dialect: Dialect, name: &str, def: &RelationDef) -> Json {
    let pairs = |pairs: Vec<(String, String)>| -> Json {
        pairs
            .into_iter()
            .map(|(from, to)| vec![from, to])
            .collect::<Vec<_>>()
            .into()
    };
    object(
        dialect,
        [
            ("name", name.into()),
            (
                "type",
                Json::String(dialect.option(match def.rel_type {
                    RelationType::HasOne => "has_one",
                    RelationType::HasMany => "has_many",
                })),
            ),
            ("from", table(dialect, &def.from_tbl)),
            ("to", table(dialect, &def.to_tbl)),
            (
                "columns",
                pairs(
                    def.columns
                        .iter()
                        .map(|(from, to)| (from.to_string(), to.to_string()))
                        .collect(),
                ),
            ),
            (
                "period",
                def.columns
                    .period()
                    .map(|(from, to)| vec![from.to_string(), to.to_string()])
                    .into(),
            ),
            (
                "condition_type",
                Json::String(dialect.option(match def.condition_type {
                    ConditionType::All => "all",
                    ConditionType::Any => "any",
                })),
            ),
            ("on_condition", def.on_condition.is_some().into()),
            (
                "enforcement",
                def.enforcement
                    .map(|enforcement| {
                        Json::String(dialect.option(match enforcement {
                            Enforcement::Enforced => "enforced",
                            Enforcement::NotEnforced => "not_enforced",
                        }))
                    })
                    .unwrap_or(Json::Null),
            ),
            (
                "deferrability",
                def.deferrability
                    .map(|deferrability| {
                        Json::String(dialect.option(match deferrability {
                            Deferrability::NotDeferrable => "not_deferrable",
                            Deferrability::DeferrableInitiallyImmediate => {
                                "deferrable_initially_immediate"
                            }
                            Deferrability::DeferrableInitiallyDeferred => {
                                "deferrable_initially_deferred"
                            }
                        }))
                    })
                    .unwrap_or(Json::Null),
            ),
        ],
    )
}

/// A relation's table as `{schema, table}`; a relation joins entities, whose
/// FROM item is always a named table.
fn table(dialect: Dialect, item: &FromItem) -> Json {
    match item {
        FromItem::Table(table) => object(
            dialect,
            [
                (
                    "schema",
                    table.name.schema().map(|schema| schema.to_string()).into(),
                ),
                ("table", table.name.table().to_string().into()),
            ],
        ),
        other => object(
            dialect,
            [
                ("schema", Json::Null),
                ("table", other.qualifier().to_string().into()),
            ],
        ),
    }
}

#[cfg(test)]
mod tests {
    use pgorm::pgorm_query::{Condition, IntoName, TableName};
    use pgorm::{ColumnPairs, alias};

    use super::*;

    fn def() -> RelationDef {
        RelationDef {
            rel_type: RelationType::HasOne,
            from_tbl: FromItem::from(TableName::Table(alias("note").into_name())),
            to_tbl: FromItem::from(TableName::Table(alias("account").into_name())),
            columns: ColumnPairs::new(alias("account_id"), alias("id")),
            is_owner: true,
            on_delete: None,
            on_update: None,
            on_condition: None,
            fk_name: None,
            enforcement: Some(Enforcement::NotEnforced),
            deferrability: None,
            condition_type: ConditionType::All,
        }
    }

    // [spec:pgorm:req:bindings.entities+6/test]
    // [spec:pgorm:req:bindings.relation-conditions/test]    a relation's join shape is reported,
    // its keys and choices spelled by each binding's rule
    #[test]
    fn join_condition_shape_is_reported_in_each_spelling() {
        let plain = relation(Dialect::Python, "Account", &def());
        assert_eq!(plain["condition_type"], "all");
        assert_eq!(plain["on_condition"], false);
        assert_eq!(plain["type"], "has_one");
        assert_eq!(plain["enforcement"], "not_enforced");
        let mut either = def();
        either.condition_type = ConditionType::Any;
        either.on_condition = Some(Box::new(|_, _| Condition::all()));
        let either = relation(Dialect::JavaScript, "Account", &either);
        assert_eq!(either["conditionType"], "any");
        assert_eq!(either["onCondition"], true);
        assert_eq!(either["type"], "hasOne");
        assert_eq!(either["enforcement"], "notEnforced");
        assert_eq!(either["columns"], plain["columns"]);
    }
}
