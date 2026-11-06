//! Closed runtime type constructors over pgorm_query::ColumnType.

use crate::{
    UnsupportedCapabilityError,
    errors::ConstructionError,
    values::{CreatedKindPy, PyTypeName},
};
use pgorm::pgorm_query::{
    ColumnType, IntervalPrecision, IntervalSpec, Name, PgInterval, StringLen,
};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyInt};
use std::sync::Arc;

#[derive(Clone, Debug)]
#[pyclass(name = "DataType", module = "pgorm.schema", frozen, from_py_object)]
pub struct PyDataType {
    pub(crate) inner: ColumnType,
}

fn integer(value: Option<&Bound<'_, PyAny>>, label: &str) -> PyResult<Option<u32>> {
    value
        .map(|value| {
            if !value.is_exact_instance_of::<PyInt>() {
                return Err(PyTypeError::new_err(format!(
                    "{label} requires an exact integer"
                )));
            }
            value
                .extract::<u32>()
                .map_err(|_| ConstructionError::new_err(format!("{label} is outside u32")))
        })
        .transpose()
}

impl PyDataType {
    pub(crate) fn coerce(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(kind) = value.extract::<PyRef<'_, Self>>() {
            return Ok(kind.clone());
        }
        Self::new(value, None, None, None, None)
    }
}

#[pymethods]
impl PyDataType {
    // [spec:pgorm:req:bindings.schema]
    #[new]
    #[pyo3(signature=(kind, *, length=None, precision=None, scale=None, fields=None))]
    fn new(
        kind: &Bound<'_, PyAny>,
        length: Option<&Bound<'_, PyAny>>,
        precision: Option<&Bound<'_, PyAny>>,
        scale: Option<&Bound<'_, PyAny>>,
        fields: Option<&str>,
    ) -> PyResult<Self> {
        let length = integer(length, "length")?;
        let precision = integer(precision, "precision")?;
        let scale = integer(scale, "scale")?;
        if fields.is_some() || kind.extract::<&str>().ok() == Some("interval") {
            if length.is_some() || scale.is_some() {
                return Err(ConstructionError::new_err(
                    "an interval takes fields and a precision, no length or scale",
                ));
            }
            return Ok(Self {
                inner: ColumnType::Interval(interval(kind, fields, precision)?),
            });
        }
        if length == Some(0) {
            return Err(ConstructionError::new_err("type length must be positive"));
        }
        if let Some(created) = crate::values::created_kind(kind) {
            if length.is_some() || precision.is_some() || scale.is_some() {
                return Err(ConstructionError::new_err(
                    "created range types do not accept size modifiers",
                ));
            }
            return Ok(Self {
                inner: created.column_type()?,
            });
        }
        if let Ok(name) = kind.extract::<PyRef<'_, PyTypeName>>() {
            if length.is_some() || precision.is_some() || scale.is_some() {
                return Err(ConstructionError::new_err(
                    "named types do not accept size modifiers",
                ));
            }
            return Ok(Self {
                inner: ColumnType::Enum {
                    name: Name::runtime(&name.name),
                    schema: name.schema.as_ref().map(Name::runtime),
                    variants: Vec::new(),
                },
            });
        }
        let kind = kind.extract::<&str>().map_err(|_| {
            PyTypeError::new_err("DataType requires a supported type name or TypeName")
        })?;
        if length.is_some() && !matches!(kind, "char" | "varchar" | "bit" | "varbit" | "vector") {
            return Err(ConstructionError::new_err(
                "this type does not accept length",
            ));
        }
        if (precision.is_some() || scale.is_some()) && kind != "numeric" {
            return Err(ConstructionError::new_err(
                "precision and scale require numeric",
            ));
        }
        let inner = match kind {
            // SQL's `char` without a length is `char(1)`; `Char(None)` is the
            // unbounded `bpchar`.
            "char" => ColumnType::Char(Some(length.unwrap_or(1))),
            "varchar" => ColumnType::String(length.map_or(StringLen::None, StringLen::N)),
            "text" => ColumnType::Text,
            "smallint" => ColumnType::SmallInteger,
            "integer" => ColumnType::Integer,
            "bigint" => ColumnType::BigInteger,
            "real" => ColumnType::Float,
            "double" => ColumnType::Double,
            "numeric" => {
                let size = match (precision, scale) {
                    (None, None) => None,
                    (Some(p @ 1..=1000), s) if s.unwrap_or(0) <= 1000 => Some((p, s.unwrap_or(0))),
                    _ => {
                        return Err(ConstructionError::new_err(
                            "numeric requires precision 1–1000 and optional scale 0–1000",
                        ));
                    }
                };
                ColumnType::Decimal(size)
            }
            "boolean" => ColumnType::Boolean,
            "date" => ColumnType::Date,
            "time" => ColumnType::Time,
            "timestamp" => ColumnType::Timestamp,
            "timestamptz" => ColumnType::TimestampWithTimeZone,
            "bytea" => ColumnType::Bytea,
            // SQL's `bit` without a length is `bit(1)`; `Bit(None)` is the
            // bit type with no length limit.
            "bit" => ColumnType::Bit(Some(length.unwrap_or(1))),
            "varbit" => ColumnType::VarBit(
                length.ok_or_else(|| ConstructionError::new_err("varbit requires length"))?,
            ),
            "money" => ColumnType::Money,
            "json" => ColumnType::Json,
            "jsonb" => ColumnType::JsonBinary,
            "uuid" => ColumnType::Uuid,
            "vector" => ColumnType::Vector(length),
            "cidr" => ColumnType::Cidr,
            "inet" => ColumnType::Inet,
            "macaddr" => ColumnType::MacAddr,
            "ltree" => ColumnType::LTree,
            kind => crate::values::range_column_type(kind).ok_or_else(|| {
                UnsupportedCapabilityError::new_err("unsupported DDL column type")
            })?,
        };
        Ok(Self { inner })
    }

    /// A domain over `base`: a column of it is written as the domain, read
    /// as its base and, a domain over an enum, compared as the enum.
    // [spec:pgorm:req:bindings.model-casts]
    #[staticmethod]
    fn domain(name: &Bound<'_, PyAny>, base: &Bound<'_, PyAny>) -> PyResult<Self> {
        let (name, schema) = match name.extract::<PyRef<'_, PyTypeName>>() {
            Ok(named) => (named.name.clone(), named.schema.clone()),
            Err(_) => (
                name.extract::<String>().map_err(|_| {
                    PyTypeError::new_err("a domain is named by a string or a TypeName")
                })?,
                None,
            ),
        };
        Ok(Self {
            inner: ColumnType::Domain {
                name: Name::runtime(name),
                schema: schema.map(Name::runtime),
                base: Arc::new(Self::coerce(base)?.inner),
            },
        })
    }

    fn array(&self) -> Self {
        Self {
            inner: ColumnType::Array(Arc::new(self.inner.clone())),
        }
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "data types cannot be tested as Python booleans",
        ))
    }
    fn __repr__(&self) -> String {
        format!("DataType({:?})", self.inner)
    }
}

/// `interval`, `interval(p)` or `interval <fields>`: the fields spelled in
/// snake_case, a precision only where the last field is seconds.
// [spec:pgorm:req:bindings.model-casts]
fn interval(
    kind: &Bound<'_, PyAny>,
    fields: Option<&str>,
    precision: Option<u32>,
) -> PyResult<IntervalSpec> {
    if kind.extract::<&str>().ok() != Some("interval") {
        return Err(ConstructionError::new_err("fields are an interval's"));
    }
    let precision = precision
        .map(|digits| {
            u8::try_from(digits)
                .ok()
                .and_then(IntervalPrecision::new)
                .ok_or_else(|| ConstructionError::new_err("an interval's precision is 0 to 6"))
        })
        .transpose()?;
    let Some(fields) = fields else {
        return Ok(IntervalSpec::Any(precision));
    };
    let Ok(named) = PgInterval::try_from(fields.replace('_', " ").as_str()) else {
        return Err(PyTypeError::new_err(
            "an interval's fields are year, month, day, hour, minute, second, year_to_month, \
             day_to_hour, day_to_minute, day_to_second, hour_to_minute, hour_to_second or \
             minute_to_second",
        ));
    };
    named
        .with_precision(precision)
        .map(IntervalSpec::Fields)
        .ok_or_else(|| {
            ConstructionError::new_err("only an interval ending in seconds takes a precision")
        })
}
