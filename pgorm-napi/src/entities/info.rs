//! What the binding knows of a registered entity: its table, its columns —
//! each one's SQL name, nullability, key membership and the value kind its
//! values bind and read as, from its `ColumnType` — and its relations as
//! `describe()` reports them.

use pgorm::pgorm_query::{
    ArrayType, ColumnType, ConditionType, Deferrability, Enforcement, FromItem,
};
use pgorm::{
    ColumnTrait, EntityTrait, FromQueryResult, Iterable, PrimaryKeyToColumn, PrimaryKeyTrait,
    RelationDef, RelationTrait, RelationType, StaticName,
};
use serde_json::{Value as Json, json};

use crate::{
    errors::Failure,
    models::kind_key,
    values::{CREATED_SUBTYPES, CreatedKind, Tag, TypeName},
};

/// The scalar kind a column type's values are, where it has one.
fn scalar(ty: &ColumnType) -> Option<ArrayType> {
    Some(match ty {
        ColumnType::Char(Some(1)) => ArrayType::Char,
        ColumnType::Char(_) | ColumnType::String(_) | ColumnType::Text => ArrayType::String,
        ColumnType::Bytea => ArrayType::Bytes,
        ColumnType::SmallInteger => ArrayType::SmallInt,
        ColumnType::Integer => ArrayType::Int,
        ColumnType::BigInteger => ArrayType::BigInt,
        ColumnType::Float => ArrayType::Float,
        ColumnType::Double => ArrayType::Double,
        ColumnType::Decimal(_) => ArrayType::Decimal,
        ColumnType::Boolean => ArrayType::Bool,
        ColumnType::Date => ArrayType::Date,
        ColumnType::Time => ArrayType::Time,
        ColumnType::Timestamp => ArrayType::DateTime,
        ColumnType::TimestampWithTimeZone => ArrayType::DateTimeWithTimeZone,
        ColumnType::Json | ColumnType::JsonBinary => ArrayType::Json,
        ColumnType::Uuid => ArrayType::Uuid,
        ColumnType::Vector(_) => ArrayType::Vector,
        ColumnType::Inet | ColumnType::Cidr => ArrayType::IpNetwork,
        ColumnType::MacAddr => ArrayType::MacAddress,
        ColumnType::Interval(_) => ArrayType::Interval,
        ColumnType::Range(range) => ArrayType::Range(*range),
        ColumnType::Multirange(range) => ArrayType::Multirange(*range),
        _ => return None,
    })
}

/// The kind a column's values bind and read as, or `None` for a type the
/// binding has no kind for, whose values a caller declares with `Value`. A
/// domain's values travel as its base type's do, so a domain column is the
/// kind of the type it is over.
// [spec:pgorm:req:bindings.entities+4]
// [spec:pgorm:req:bindings.schema-domains+2]
pub(crate) fn column_tag(ty: &ColumnType) -> Option<Tag> {
    let name =
        |name: &pgorm::pgorm_query::Name, schema: &Option<pgorm::pgorm_query::Name>| TypeName {
            name: name.to_string(),
            schema: schema.as_ref().map(|schema| schema.to_string()),
        };
    match ty {
        ColumnType::Enum {
            name: n, schema, ..
        } => Some(Tag::Enum(name(n, schema))),
        ColumnType::CreatedRange {
            name: n,
            schema,
            subtype,
        }
        | ColumnType::CreatedMultirange {
            name: n,
            schema,
            subtype,
        } => {
            let subtype = scalar(subtype).filter(|kind| CREATED_SUBTYPES.contains(kind))?;
            Some(Tag::Created(CreatedKind {
                name: name(n, schema),
                subtype,
                multirange: matches!(ty, ColumnType::CreatedMultirange { .. }),
            }))
        }
        ColumnType::Array(member) => column_tag(member).map(|tag| Tag::Array(Box::new(tag))),
        ColumnType::Domain { base, .. } => column_tag(base),
        // A `money` column is read as `numeric` and written cast to `money`
        // by the entity's own casts, so its values are decimals; no range
        // the binding knows is over it, so `scalar` does not name it.
        ColumnType::Money => Some(Tag::Scalar(ArrayType::Decimal)),
        other => scalar(other).map(Tag::Scalar),
    }
}

/// One column of a registered entity.
#[derive(Clone, Debug)]
pub(crate) struct ColumnInfo {
    pub(crate) name: String,
    /// The key the Rust field serializes under, `ColumnTrait::json_key`.
    pub(crate) json_key: String,
    pub(crate) sql_type: String,
    pub(crate) rust_type: Option<&'static str>,
    pub(crate) nullable: bool,
    pub(crate) primary_key: bool,
    pub(crate) tag: Option<Tag>,
    pub(crate) labels: Option<Vec<String>>,
}

/// An enum column's labels, in declaration order, an array of one's and a
/// domain over one included: what a generated declaration types its values
/// as.
// [spec:pgorm:req:bindings.schema-domains+2]
fn labels(ty: &ColumnType) -> Option<Vec<String>> {
    use pgorm::pgorm_query::SqlName;
    match ty {
        ColumnType::Enum { variants, .. } => Some(
            variants
                .iter()
                .map(|variant| SqlName::to_string(&**variant))
                .collect(),
        ),
        ColumnType::Array(member) | ColumnType::Domain { base: member, .. } => labels(member),
        _ => None,
    }
}

impl ColumnInfo {
    fn describe(&self) -> Result<Json, Failure> {
        Ok(json!({
            "name": self.name,
            "jsonKey": self.json_key,
            "sqlType": self.sql_type,
            "rustType": self.rust_type,
            "nullable": self.nullable,
            "primaryKey": self.primary_key,
            "kind": self.tag.as_ref().map(kind_key).transpose()?,
            "values": self.labels,
        }))
    }
}

/// A registered entity: its registration name, table, columns, key and
/// relations, and the Rust types behind it.
#[derive(Debug)]
pub(crate) struct EntityInfo {
    pub(crate) name: String,
    pub(crate) schema: Option<String>,
    pub(crate) table: String,
    pub(crate) columns: Vec<ColumnInfo>,
    pub(crate) primary_key: Vec<String>,
    without_overlaps: bool,
    relations: Vec<Json>,
    rust: [&'static str; 4],
}

/// A registration name: 1–255 UTF-8 bytes without NUL.
pub(crate) fn registration_name(name: &str) -> Result<(), Failure> {
    if name.is_empty() || name.len() > 255 || name.contains('\0') {
        return Err(Failure::Construction(
            "a registration's name is 1–255 UTF-8 bytes without NUL".to_owned(),
        ));
    }
    Ok(())
}

impl EntityInfo {
    pub(crate) fn of<E: EntityTrait>(name: &str) -> Result<Self, Failure> {
        registration_name(name)?;
        let entity = E::default();
        let primary_key: Vec<String> = <E::PrimaryKey as Iterable>::iter()
            .map(|key| key.into_column().as_str().to_owned())
            .collect();
        let reflected = E::Model::expected_columns();
        let mut columns: Vec<ColumnInfo> = Vec::new();
        for column in <E::Column as Iterable>::iter() {
            let column_name = column.as_str().to_owned();
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
                tag: column_tag(definition.get_column_type()),
                labels: labels(definition.get_column_type()),
                name: column_name,
            });
        }
        if columns.is_empty() {
            return Err(Failure::Construction(format!("{name} has no column")));
        }
        Ok(Self {
            name: name.to_owned(),
            schema: entity.schema_name().map(str::to_owned),
            table: entity.table_name().to_owned(),
            columns,
            primary_key,
            without_overlaps: <E::PrimaryKey as PrimaryKeyTrait>::without_overlaps(),
            relations: <E::Relation as Iterable>::iter()
                .map(|relation| relation_json(&format!("{relation:?}"), &relation.def()))
                .collect(),
            rust: [
                std::any::type_name::<E>(),
                std::any::type_name::<E::Model>(),
                std::any::type_name::<E::ActiveModel>(),
                std::any::type_name::<E::Column>(),
            ],
        })
    }

    pub(crate) fn column(&self, name: &str) -> Result<&ColumnInfo, Failure> {
        self.columns
            .iter()
            .find(|column| column.name == name)
            .ok_or_else(|| Failure::Construction(format!("{} has no column {name:?}", self.name)))
    }

    /// The registration as `describe()` gives it.
    // [spec:pgorm:req:bindings.entities+4]
    pub(crate) fn describe(&self) -> Result<Json, Failure> {
        Ok(json!({
            "name": self.name,
            "schema": self.schema,
            "table": self.table,
            "columns": self.columns.iter().map(ColumnInfo::describe).collect::<Result<Vec<_>, _>>()?,
            "primaryKey": self.primary_key,
            "primaryKeyWithoutOverlaps": self.without_overlaps,
            "relations": self.relations,
            "rust": {
                "entity": self.rust[0],
                "model": self.rust[1],
                "activeModel": self.rust[2],
                "column": self.rust[3],
            },
        }))
    }
}

/// A relation as `describe()` reports it: the parts of a compiled
/// declaration that change what a join along it means, and what the server
/// holds its foreign key's rows to. `onCondition` is present or absent only:
/// it is a Rust closure over the two tables' aliases, which has no rendering
/// of its own until a join supplies them.
// [spec:pgorm:req:bindings.entities+4]
// [spec:pgorm:req:bindings.relation-conditions]
fn relation_json(name: &str, def: &RelationDef) -> Json {
    json!({
        "name": name,
        "type": match def.rel_type {
            RelationType::HasOne => "hasOne",
            RelationType::HasMany => "hasMany",
        },
        "from": table(&def.from_tbl),
        "to": table(&def.to_tbl),
        "columns": def
            .columns
            .iter()
            .map(|(from, to)| [from.to_string(), to.to_string()])
            .collect::<Vec<_>>(),
        "period": def
            .columns
            .period()
            .map(|(from, to)| [from.to_string(), to.to_string()]),
        "conditionType": match def.condition_type {
            ConditionType::All => "all",
            ConditionType::Any => "any",
        },
        "onCondition": def.on_condition.is_some(),
        "enforcement": def.enforcement.map(|enforcement| match enforcement {
            Enforcement::Enforced => "enforced",
            Enforcement::NotEnforced => "notEnforced",
        }),
        "deferrability": def.deferrability.map(|deferrability| match deferrability {
            Deferrability::NotDeferrable => "notDeferrable",
            Deferrability::DeferrableInitiallyImmediate => "deferrableInitiallyImmediate",
            Deferrability::DeferrableInitiallyDeferred => "deferrableInitiallyDeferred",
        }),
    })
}

fn table(item: &FromItem) -> Json {
    match item {
        FromItem::Table(table) => json!({
            "schema": table.name.schema().map(|schema| schema.to_string()),
            "table": table.name.table().to_string(),
        }),
        other => json!({"schema": null, "table": other.qualifier().to_string()}),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pgorm::pgorm_query::tests_cfg::Variants;
    use pgorm::pgorm_query::{
        ColumnType, Condition, IntervalSpec, IntoName, Name, RangeType, StringLen, TableName,
    };
    use pgorm::{ColumnPairs, alias};

    use super::*;

    fn relation() -> RelationDef {
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
            enforcement: None,
            deferrability: None,
            condition_type: ConditionType::All,
        }
    }

    // [spec:pgorm:req:bindings.entities+4/test]
    // [spec:pgorm:req:bindings.relation-conditions/test]
    #[test]
    fn join_condition_shape_is_reported() {
        let plain = relation_json("Account", &relation());
        assert_eq!(plain["conditionType"], "all");
        assert_eq!(plain["onCondition"], false);
        let mut either = relation();
        either.condition_type = ConditionType::Any;
        either.on_condition = Some(Box::new(|_, _| Condition::all()));
        let either = relation_json("Account", &either);
        assert_eq!(either["conditionType"], "any");
        assert_eq!(either["onCondition"], true);
        assert_eq!(either["columns"], plain["columns"]);
    }

    fn key(ty: &ColumnType) -> Option<String> {
        column_tag(ty)
            .as_ref()
            .map(|tag| kind_key(tag).unwrap_or_else(|failure| panic!("{failure}")))
    }

    // [spec:pgorm:req:bindings.entities+4/test]    each column type maps to the kind
    // a decoded column of it carries, so a registered column's values bind and
    // read as the binding's own; a type it has no kind for takes a `Value`
    #[test]
    fn column_types_map_to_decoded_kinds() {
        assert_eq!(key(&ColumnType::Integer).as_deref(), Some("i32"));
        assert_eq!(
            key(&ColumnType::String(StringLen::None)).as_deref(),
            Some("text")
        );
        assert_eq!(key(&ColumnType::Char(Some(1))).as_deref(), Some("char"));
        assert_eq!(
            key(&ColumnType::TimestampWithTimeZone).as_deref(),
            Some("datetime_utc")
        );
        assert_eq!(
            key(&ColumnType::Array(Arc::new(ColumnType::BigInteger))).as_deref(),
            Some("i64[]")
        );
        assert_eq!(
            key(&ColumnType::Range(RangeType::Int4)).as_deref(),
            Some("int4range")
        );
        let mood = ColumnType::Enum {
            name: Name::runtime("mood"),
            schema: Some(Name::runtime("app")),
            variants: Vec::new(),
        };
        assert_eq!(key(&mood).as_deref(), Some("enum \"app\".\"mood\""));
        let created = ColumnType::CreatedRange {
            name: Name::runtime("floatrange"),
            schema: None,
            subtype: Arc::new(ColumnType::Double),
        };
        assert_eq!(
            key(&created).as_deref(),
            Some("range \"floatrange\" of f64")
        );
        assert_eq!(key(&ColumnType::Money).as_deref(), Some("decimal"));
        assert_eq!(
            key(&ColumnType::Interval(IntervalSpec::Any(None))).as_deref(),
            Some("interval")
        );
    }

    // [spec:pgorm:req:bindings.schema-domains+2/test]    a domain column is the kind
    // of the type it is over, through a domain over a domain, an array of one
    // and a domain over an enum, whose labels it carries
    #[test]
    fn domain_columns_take_their_base_kind() {
        let over = |name: &str, base: ColumnType| ColumnType::Domain {
            name: Name::runtime(name),
            schema: Some(Name::runtime("billing")),
            base: Arc::new(base),
        };
        let quantity = over("quantity", ColumnType::Integer);
        assert_eq!(key(&quantity).as_deref(), Some("i32"));
        assert_eq!(
            key(&over("small", quantity.clone())).as_deref(),
            Some("i32")
        );
        assert_eq!(
            key(&ColumnType::Array(Arc::new(quantity))).as_deref(),
            Some("i32[]")
        );
        assert_eq!(
            key(&over("tags", ColumnType::Array(Arc::new(ColumnType::Text)))).as_deref(),
            Some("text[]")
        );
        let mood = ColumnType::Enum {
            name: Name::runtime("mood"),
            schema: None,
            variants: vec![Name::runtime("calm"), Name::runtime("tense")],
        };
        let calm = over("calm_mood", mood);
        assert_eq!(key(&calm).as_deref(), Some("enum \"mood\""));
        assert_eq!(
            labels(&calm),
            Some(vec!["calm".to_owned(), "tense".to_owned()])
        );
        assert_eq!(
            key(&over(
                "money",
                ColumnType::Array(Arc::new(ColumnType::Money))
            ))
            .as_deref(),
            Some("decimal[]")
        );
    }

    // [spec:pgorm:req:bindings.entities+4/test]
    #[test]
    fn only_types_without_a_kind_have_no_tag() {
        let untagged: Vec<&str> = ColumnType::variants()
            .iter()
            .filter(|ty| column_tag(ty).is_none())
            .map(Variants::variant_name)
            .collect();
        assert_eq!(untagged, ["Bit", "VarBit", "Named", "LTree"]);
    }
}
