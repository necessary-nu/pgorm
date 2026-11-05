use pgorm::pgorm_query::{ArrayType, ColumnType, Value};
use pgorm::{
    ColumnTrait, EntityTrait, FromQueryResult, Iterable, PrimaryKeyToColumn, PrimaryKeyTrait,
    StaticName,
};
use pyo3::{prelude::*, types::PyString};
use serde_json::{Value as Json, json};

use crate::{
    errors::{ConstructionError, DecodeError},
    identifiers::validate_name,
    values::{CreatedKind, PyTypeName, PyValue, Tag},
};

#[derive(Clone, Debug)]
pub(crate) enum InputKind {
    Scalar(ArrayType),
    Enum(PyTypeName),
    Created(CreatedKind),
    Array(Box<Self>),
    Explicit,
}

impl InputKind {
    pub(super) fn from_column(ty: &ColumnType) -> Self {
        if let Some(kind) = CreatedKind::from_column(ty) {
            return Self::Created(kind);
        }
        let scalar = match ty {
            ColumnType::Char(Some(1)) => ArrayType::Char,
            ColumnType::Char(_) | ColumnType::String(_) | ColumnType::Text => ArrayType::String,
            ColumnType::Bytea => ArrayType::Bytes,
            ColumnType::SmallInteger => ArrayType::SmallInt,
            ColumnType::Integer => ArrayType::Int,
            ColumnType::BigInteger => ArrayType::BigInt,
            ColumnType::Float => ArrayType::Float,
            ColumnType::Double => ArrayType::Double,
            // A `money` column is read as `numeric` and written cast to
            // `money` by the entity's own casts, so its values are decimals.
            // [spec:pgorm:req:bindings.entities+5]
            ColumnType::Decimal(_) | ColumnType::Money => ArrayType::Decimal,
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
            ColumnType::Enum { name, schema, .. } => {
                return Self::Enum(PyTypeName {
                    name: name.to_string(),
                    schema: schema.as_ref().map(|s| s.to_string()),
                });
            }
            ColumnType::Range(range) => ArrayType::Range(*range),
            ColumnType::Multirange(range) => ArrayType::Multirange(*range),
            ColumnType::Array(member) => return Self::Array(Box::new(Self::from_column(member))),
            // A domain's values travel as its base type's do, so its column
            // takes and reads back what a column of the base would.
            // [spec:pgorm:req:bindings.schema-domains+2]
            ColumnType::Domain { base, .. } => return Self::from_column(base),
            _ => return Self::Explicit,
        };
        Self::Scalar(scalar)
    }

    fn kind<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match self {
            Self::Scalar(kind) => Ok(PyString::new(py, kind.kind_name()).into_any()),
            Self::Enum(name) => Ok(Py::new(py, name.clone())?.into_bound(py).into_any()),
            Self::Created(kind) => Ok(kind.to_python(py)?.into_bound(py)),
            _ => Err(ConstructionError::new_err(
                "this column requires an explicit tagged Value",
            )),
        }
    }

    pub(crate) fn coerce(&self, value: &Bound<'_, PyAny>) -> PyResult<PyValue> {
        if let Ok(value) = value.extract::<PyRef<'_, PyValue>>() {
            if let Some(kind) = value.created()
                && !matches!(self, Self::Created(expected) if expected == kind)
            {
                return Err(ConstructionError::new_err(
                    "created range Value belongs to a different declared column type",
                ));
            }
            if let Some(cast) = value.enum_cast() {
                let (expected, array) = match self {
                    Self::Enum(name) => (Some(name), false),
                    Self::Array(member) => (
                        match member.as_ref() {
                            Self::Enum(name) => Some(name),
                            _ => None,
                        },
                        true,
                    ),
                    _ => (None, false),
                };
                if !expected.is_some_and(|name| {
                    cast.name.to_string() == name.name
                        && cast.schema.as_ref().map(|s| s.to_string()) == name.schema
                        && cast.array == array
                }) {
                    return Err(ConstructionError::new_err(
                        "enum Value belongs to a different declared column type",
                    ));
                }
            }
            return Ok(value.clone());
        }
        let py = value.py();
        let class = py.get_type::<PyValue>();
        let result = match self {
            Self::Array(member) => class.call_method1("array", (member.kind(py)?, value))?,
            _ => class.call1((value, self.kind(py)?))?,
        };
        Ok(result.extract()?)
    }

    pub(crate) fn tagged(&self, value: Value) -> PyResult<PyValue> {
        match self {
            Self::Enum(name) if matches!(value, Value::String(_)) => {
                Ok(PyValue::from_enum(value, name.clone(), false))
            }
            Self::Created(kind) => match value {
                Value::String(text) => {
                    Ok(PyValue::from_created(text.map(|text| *text), kind.clone()))
                }
                _ => Err(DecodeError::new_err(
                    "compiled created range returned an incompatible Rust Value",
                )),
            },
            Self::Array(member) => match member.as_ref() {
                Self::Enum(name) if matches!(value, Value::Array(ArrayType::String, _)) => {
                    Ok(PyValue::from_enum(value, name.clone(), true))
                }
                Self::Enum(_) => Err(DecodeError::new_err(
                    "compiled enum array returned an incompatible Rust Value",
                )),
                _ => Ok(PyValue::from_rust(value)),
            },
            Self::Enum(_) => Err(DecodeError::new_err(
                "compiled enum returned an incompatible Rust Value",
            )),
            _ => Ok(PyValue::from_rust(value)),
        }
    }

    /// The kind a value of the column carries, or `None` for a column that
    /// takes an explicit `Value`, an array of one included.
    fn tag(&self) -> Option<Tag> {
        Some(match self {
            Self::Scalar(kind) => Tag::Scalar(kind.clone()),
            Self::Enum(name) => Tag::Enum(name.clone()),
            Self::Created(kind) => Tag::Created(kind.clone()),
            Self::Array(member) => Tag::Array(Box::new(member.tag()?)),
            Self::Explicit => return None,
        })
    }

    /// The kind as `describe()` spells it.
    // [spec:pgorm:req:bindings.entities+5]
    fn key(&self) -> Option<String> {
        self.tag().map(|tag| tag.key())
    }
}

/// An enum column's labels, in declaration order, an array of one's and a
/// domain over one included.
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

#[derive(Clone, Debug)]
pub(crate) struct ColumnInfo {
    pub(crate) name: String,
    pub(crate) json_key: String,
    pub(crate) sql_type: String,
    pub(crate) rust_type: Option<&'static str>,
    pub(crate) nullable: bool,
    pub(crate) primary_key: bool,
    pub(crate) input: InputKind,
    pub(crate) labels: Option<Vec<String>>,
}

impl ColumnInfo {
    fn describe(&self) -> Json {
        json!({
            "name": self.name,
            "json_key": self.json_key,
            "sql_type": self.sql_type,
            "rust_type": self.rust_type,
            "nullable": self.nullable,
            "primary_key": self.primary_key,
            "kind": self.input.key(),
            "values": self.labels,
        })
    }
}

#[derive(Debug)]
pub(crate) struct EntityInfo {
    pub(crate) name: String,
    pub(crate) schema: Option<String>,
    pub(crate) table: String,
    pub(crate) entity_type: &'static str,
    pub(crate) column_type: &'static str,
    pub(crate) model_type: &'static str,
    pub(crate) active_type: &'static str,
    pub(crate) columns: Vec<ColumnInfo>,
    pub(crate) primary_key: Vec<String>,
    /// Whether the key's last part is a period matched `WITHOUT OVERLAPS`.
    /// A lookup, update or delete by the key still compares it for equality.
    pub(crate) without_overlaps: bool,
    /// The table's unique keys over two columns or more, as the entity's
    /// `EntityName::unique_keys` declares them.
    pub(crate) unique_keys: &'static [pgorm::UniqueKey],
    pub(crate) relations: Vec<Json>,
}

impl EntityInfo {
    pub(crate) fn of<E: EntityTrait>(name: &str) -> PyResult<Self> {
        if name.is_empty() || name.len() > 255 || name.contains('\0') {
            return Err(ConstructionError::new_err(
                "registration names require 1–255 UTF-8 bytes without NUL",
            ));
        }
        let entity = E::default();
        let table = entity.table_name().to_owned();
        let schema = entity.schema_name().map(str::to_owned);
        validate_name(&table)?;
        if let Some(schema) = &schema {
            validate_name(schema)?;
        }
        let keys: Vec<_> = E::PrimaryKey::iter()
            .map(|k| k.into_column().as_str().to_owned())
            .collect();
        let mut names = std::collections::HashSet::new();
        let mut columns = Vec::new();
        let reflected = E::Model::expected_columns();
        for column in E::Column::iter() {
            let name = column.as_str().to_owned();
            validate_name(&name)?;
            if !names.insert(name.clone()) {
                return Err(ConstructionError::new_err(
                    "compiled entity has duplicate SQL column names",
                ));
            }
            let definition = column.def();
            columns.push(ColumnInfo {
                rust_type: reflected.as_ref().and_then(|fields| {
                    fields
                        .iter()
                        .find(|field| field.name() == name)
                        .map(|field| field.rust_type())
                }),
                primary_key: keys.contains(&name),
                name,
                json_key: column.json_key().to_owned(),
                nullable: definition.is_null(),
                sql_type: format!("{:?}", definition.get_column_type()),
                input: InputKind::from_column(definition.get_column_type()),
                labels: labels(definition.get_column_type()),
            });
        }
        if columns.is_empty() {
            return Err(ConstructionError::new_err(
                "compiled entities require a nonempty projection",
            ));
        }
        Ok(Self {
            name: name.to_owned(),
            schema,
            table,
            columns,
            primary_key: keys,
            without_overlaps: <E::PrimaryKey as PrimaryKeyTrait>::without_overlaps(),
            unique_keys: entity.unique_keys(),
            relations: super::relations::describe::<E>(),
            entity_type: std::any::type_name::<E>(),
            column_type: std::any::type_name::<E::Column>(),
            model_type: std::any::type_name::<E::Model>(),
            active_type: std::any::type_name::<E::ActiveModel>(),
        })
    }

    pub(crate) fn column(&self, name: &str) -> PyResult<&ColumnInfo> {
        self.columns
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| ConstructionError::new_err("unknown compiled entity column"))
    }

    /// The registration as `describe()` gives it.
    // [spec:pgorm:req:bindings.entities+5]
    pub(crate) fn describe(&self) -> Json {
        json!({
            "name": self.name,
            "schema": self.schema,
            "table": self.table,
            "columns": self.columns.iter().map(ColumnInfo::describe).collect::<Vec<_>>(),
            "primary_key": self.primary_key,
            "primary_key_without_overlaps": self.without_overlaps,
            "unique_keys": self
                .unique_keys
                .iter()
                .map(|key| json!({
                    "name": key.name,
                    "columns": key.columns,
                    "nulls_not_distinct": key.nulls_not_distinct,
                }))
                .collect::<Vec<_>>(),
            "relations": self.relations,
            "rust": {
                "entity": self.entity_type,
                "model": self.model_type,
                "active_model": self.active_type,
                "column": self.column_type,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::PyRange;
    use pgorm::pgorm_query::{ArrayType, Range, RangeType};
    use std::sync::Arc;

    fn key(ty: &ColumnType) -> Option<String> {
        InputKind::from_column(ty).key()
    }

    // [spec:pgorm:req:bindings.entities+5/test]    a range column is described by its range kind,
    // and a registered entity's range field takes a `pgorm.Range` as its value
    #[test]
    fn range_columns_take_their_range_kind() -> PyResult<()> {
        assert_eq!(
            key(&ColumnType::Range(RangeType::Int4)).as_deref(),
            Some("int4range")
        );
        assert_eq!(
            key(&ColumnType::Multirange(RangeType::TimestampTz)).as_deref(),
            Some("tstzmultirange")
        );
        assert_eq!(
            key(&ColumnType::Array(Arc::new(ColumnType::Range(
                RangeType::Date
            ))))
            .as_deref(),
            Some("daterange[]")
        );
        assert_eq!(key(&ColumnType::Money).as_deref(), Some("decimal"));
        assert_eq!(
            key(&ColumnType::Array(Arc::new(ColumnType::Money))).as_deref(),
            Some("decimal[]")
        );

        Python::initialize();
        Python::attach(|py| {
            let range = Py::new(
                py,
                PyRange::bounds(
                    Some(1i32.into_pyobject(py)?.into_any().unbind()),
                    true,
                    None,
                    false,
                ),
            )?;
            let value = InputKind::from_column(&ColumnType::Range(RangeType::Int4))
                .coerce(range.bind(py).as_any())?;
            assert_eq!(value.rust_value(), &Value::from(Range::from(1i32..)));
            let spans = pyo3::types::PyList::new(py, [range])?;
            let array = InputKind::from_column(&ColumnType::Array(Arc::new(ColumnType::Range(
                RangeType::Int4,
            ))))
            .coerce(spans.as_any())?;
            assert!(matches!(
                array.rust_value(),
                Value::Array(ArrayType::Range(RangeType::Int4), Some(_))
            ));
            Ok(())
        })
    }

    // [spec:pgorm:req:bindings.entities+5/test]    a created range column is described by its
    // created kind, and a registered entity's field takes a `pgorm.Range` and reads back tagged
    // with the type
    #[test]
    fn created_range_columns_take_their_created_kind() -> PyResult<()> {
        use pgorm::pgorm_query::Name;

        let column = ColumnType::CreatedRange {
            name: Name::runtime("floatrange"),
            schema: Some(Name::runtime("measure")),
            subtype: Arc::new(ColumnType::Double),
        };
        let input = InputKind::from_column(&column);
        assert_eq!(
            input.key().as_deref(),
            Some("range \"measure\".\"floatrange\" of f64")
        );
        let multirange = ColumnType::CreatedMultirange {
            name: Name::runtime("slot_multirange"),
            schema: None,
            subtype: Arc::new(ColumnType::Integer),
        };
        assert_eq!(
            key(&multirange).as_deref(),
            Some("multirange \"slot_multirange\" of i32")
        );

        Python::initialize();
        Python::attach(|py| {
            let range = Py::new(
                py,
                PyRange::bounds(
                    Some(1.5f64.into_pyobject(py)?.into_any().unbind()),
                    true,
                    None,
                    false,
                ),
            )?;
            let value = input.coerce(range.bind(py).as_any())?;
            assert_eq!(
                value.rust_value(),
                &Value::String(Some(Box::new("[1.5,)".into())))
            );
            let tagged = input.tagged(value.rust_value().clone())?;
            assert_eq!(&tagged, &value);
            let other = InputKind::from_column(&multirange)
                .coerce(pyo3::types::PyString::new(py, "{[1,2)}").as_any())?;
            assert!(input.coerce(Py::new(py, other)?.bind(py).as_any()).is_err());
            Ok(())
        })
    }

    // [spec:pgorm:req:bindings.schema-domains+2/test]    a domain column takes its base type's kind,
    // through a domain over a domain, an array of the domain and a domain over an enum, whose
    // labels it carries, so a registered entity's field binds and reads back as a column of the
    // base would
    #[test]
    fn domain_columns_take_their_base_kind() -> PyResult<()> {
        use pgorm::pgorm_query::Name;

        let domain = |name: &str, base: ColumnType| ColumnType::Domain {
            name: Name::runtime(name),
            schema: Some(Name::runtime("billing")),
            base: Arc::new(base),
        };
        let quantity = domain("quantity", ColumnType::Integer);
        assert_eq!(key(&quantity).as_deref(), Some("i32"));
        assert_eq!(
            key(&domain("small", quantity.clone())).as_deref(),
            Some("i32")
        );
        assert_eq!(
            key(&ColumnType::Array(Arc::new(quantity.clone()))).as_deref(),
            Some("i32[]")
        );
        let mood = ColumnType::Enum {
            name: Name::runtime("mood"),
            schema: None,
            variants: vec![Name::runtime("calm"), Name::runtime("tense")],
        };
        let cheerful = domain("cheerful", mood);
        assert_eq!(key(&cheerful).as_deref(), Some("enum \"mood\""));
        assert_eq!(
            labels(&ColumnType::Array(Arc::new(cheerful))),
            Some(vec!["calm".to_owned(), "tense".to_owned()])
        );
        assert_eq!(labels(&quantity), None);

        Python::initialize();
        Python::attach(|py| {
            let value = InputKind::from_column(&quantity)
                .coerce(7i64.into_pyobject(py)?.into_any().as_any())?;
            assert_eq!(value.rust_value(), &Value::Int(Some(7)));
            Ok(())
        })
    }
}
