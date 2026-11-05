//! Range and multirange types a schema created: a kind naming the type and
//! its subtype, and the text form a value of one travels as.
//!
//! The Rust value of such a range is `Value::String` of its text form, the
//! `Value` a `CreatedRange` newtype converts into, because no range
//! tag can carry a name only the schema knows. The kind carries the name,
//! which the cast needs, and the subtype, which converts each bound with that
//! scalar kind's limits and reads the text back.

use std::ops;

use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use pgorm::pgorm_query::{
    ArrayType, ColumnType, Multirange, Range, RangeSubtype, StringLen, Value,
};
use pyo3::{prelude::*, types::PyString};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::{classes::PyMultirange, ranges, types::PyTypeName};
use crate::errors::{ConstructionError, DecodeError};

/// The value kinds a created range can range over: `RangeSubtype`'s twelve.
const SUBTYPES: [ArrayType; 12] = [
    ArrayType::SmallInt,
    ArrayType::Int,
    ArrayType::BigInt,
    ArrayType::Float,
    ArrayType::Double,
    ArrayType::Decimal,
    ArrayType::String,
    ArrayType::Date,
    ArrayType::Time,
    ArrayType::DateTime,
    ArrayType::DateTimeWithTimeZone,
    ArrayType::Uuid,
];

/// Run a generic function at the Rust type of a subtype's value kind.
macro_rules! dispatch {
    ($subtype:expr, $f:ident, $($arg:expr),*) => {
        match $subtype {
            ArrayType::SmallInt => $f::<i16>($($arg),*),
            ArrayType::Int => $f::<i32>($($arg),*),
            ArrayType::BigInt => $f::<i64>($($arg),*),
            ArrayType::Float => $f::<f32>($($arg),*),
            ArrayType::Double => $f::<f64>($($arg),*),
            ArrayType::Decimal => $f::<Decimal>($($arg),*),
            ArrayType::Date => $f::<Date>($($arg),*),
            ArrayType::Time => $f::<Time>($($arg),*),
            ArrayType::DateTime => $f::<DateTime>($($arg),*),
            ArrayType::DateTimeWithTimeZone => $f::<Timestamp>($($arg),*),
            ArrayType::Uuid => $f::<Uuid>($($arg),*),
            _ => $f::<String>($($arg),*),
        }
    };
}

/// A created range or multirange type: its name, its subtype's value kind,
/// and which of the two it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedKind {
    pub(crate) name: PyTypeName,
    pub(crate) subtype: ArrayType,
    pub(crate) multirange: bool,
}

impl CreatedKind {
    fn new(
        name: &Bound<'_, PyAny>,
        subtype: &str,
        schema: Option<&Bound<'_, PyAny>>,
        multirange: bool,
    ) -> PyResult<Self> {
        let subtype = ArrayType::from_kind_name(subtype)
            .filter(|kind| SUBTYPES.contains(kind))
            .ok_or_else(|| {
                ConstructionError::new_err(
                    "a created range's subtype is one of i16, i32, i64, f32, f64, decimal, \
                     text, date, time, datetime, datetime_utc or uuid",
                )
            })?;
        Ok(Self {
            name: PyTypeName::new(name, schema)?,
            subtype,
            multirange,
        })
    }

    /// The kind a `ColumnType::CreatedRange` or `CreatedMultirange` column
    /// holds, when its subtype is one a created range can range over.
    pub(crate) fn from_column(ty: &ColumnType) -> Option<Self> {
        let (name, schema, subtype, multirange) = match ty {
            ColumnType::CreatedRange {
                name,
                schema,
                subtype,
            } => (name, schema, subtype, false),
            ColumnType::CreatedMultirange {
                name,
                schema,
                subtype,
            } => (name, schema, subtype, true),
            _ => return None,
        };
        let subtype = match subtype.as_ref() {
            ColumnType::SmallInteger => ArrayType::SmallInt,
            ColumnType::Integer => ArrayType::Int,
            ColumnType::BigInteger => ArrayType::BigInt,
            ColumnType::Float => ArrayType::Float,
            ColumnType::Double => ArrayType::Double,
            ColumnType::Decimal(_) => ArrayType::Decimal,
            ColumnType::Text | ColumnType::String(_) | ColumnType::Char(_) => ArrayType::String,
            ColumnType::Date => ArrayType::Date,
            ColumnType::Time => ArrayType::Time,
            ColumnType::Timestamp => ArrayType::DateTime,
            ColumnType::TimestampWithTimeZone => ArrayType::DateTimeWithTimeZone,
            ColumnType::Uuid => ArrayType::Uuid,
            _ => return None,
        };
        Some(Self {
            name: PyTypeName {
                name: name.to_string(),
                schema: schema.as_ref().map(|schema| schema.to_string()),
            },
            subtype,
            multirange,
        })
    }

    /// The column type a schema declares for this kind.
    pub(crate) fn column_type(&self) -> ColumnType {
        let subtype = std::sync::Arc::new(match self.subtype {
            ArrayType::SmallInt => ColumnType::SmallInteger,
            ArrayType::Int => ColumnType::Integer,
            ArrayType::BigInt => ColumnType::BigInteger,
            ArrayType::Float => ColumnType::Float,
            ArrayType::Double => ColumnType::Double,
            ArrayType::Decimal => ColumnType::Decimal(None),
            ArrayType::Date => ColumnType::Date,
            ArrayType::Time => ColumnType::Time,
            ArrayType::DateTime => ColumnType::Timestamp,
            ArrayType::DateTimeWithTimeZone => ColumnType::TimestampWithTimeZone,
            ArrayType::Uuid => ColumnType::Uuid,
            _ => ColumnType::String(StringLen::None),
        });
        let rust = self.name.rust_type();
        let (name, schema) = (rust.name, rust.schema);
        if self.multirange {
            ColumnType::CreatedMultirange {
                name,
                schema,
                subtype,
            }
        } else {
            ColumnType::CreatedRange {
                name,
                schema,
                subtype,
            }
        }
    }

    pub(crate) fn kind_name(&self) -> &'static str {
        if self.multirange {
            "created_multirange"
        } else {
            "created_range"
        }
    }

    pub(crate) fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.multirange {
            Ok(Py::new(py, PyCreatedMultirange(self.clone()))?.into_any())
        } else {
            Ok(Py::new(py, PyCreatedRange(self.clone()))?.into_any())
        }
    }

    /// The text form of a Python value of this kind: a `pgorm.Range` (or a
    /// `pgorm.Multirange`), each bound converted as the subtype's kind, or the
    /// text form itself, which is read and written back canonically.
    // [spec:pgorm:req:bindings.created-ranges+3]
    pub(crate) fn text(&self, data: &Bound<'_, PyAny>) -> PyResult<String> {
        let ranges = if let Ok(text) = data.cast_exact::<PyString>() {
            match dispatch!(self.subtype, parsed, text.to_str()?, self.multirange) {
                Some(ranges) => ranges,
                None => {
                    return Err(ConstructionError::new_err(format!(
                        "text is no {} over {}",
                        self.kind_name(),
                        self.subtype.kind_name()
                    )));
                }
            }
        } else {
            self.bounds(data)?
        };
        dispatch!(self.subtype, written, ranges, self.multirange)
            .ok_or_else(|| ConstructionError::new_err("a bound does not convert to the subtype"))
    }

    /// A `pgorm.Range`'s, or each range of a `pgorm.Multirange`'s, bounds as
    /// subtype values.
    fn bounds(&self, data: &Bound<'_, PyAny>) -> PyResult<Vec<Range<Value>>> {
        if !self.multirange {
            return Ok(vec![ranges::read_bounds(data, &self.subtype)?]);
        }
        let multirange = data
            .cast_exact::<PyMultirange>()
            .map_err(|_| ConstructionError::new_err("expected pgorm.Multirange"))?;
        multirange
            .get()
            .ranges
            .iter()
            .map(|range| ranges::read_bounds(range.bind(data.py()).as_any(), &self.subtype))
            .collect()
    }

    /// The Python value of this kind's text form.
    // [spec:pgorm:req:bindings.created-ranges+3]
    pub(crate) fn read(&self, py: Python<'_>, text: &str) -> PyResult<Py<PyAny>> {
        let ranges = dispatch!(self.subtype, parsed, text, self.multirange).ok_or_else(|| {
            DecodeError::new_err(format!(
                "text is no value of {} {}",
                self.kind_name(),
                self.name.name
            ))
        })?;
        if self.multirange {
            ranges::multirange_to_python(py, &Multirange::from(ranges))
        } else {
            let range = ranges.into_iter().next().unwrap_or(Range::Empty);
            Ok(Py::new(py, ranges::write(py, &range)?)?.into_any())
        }
    }
}

/// A range of subtype values as a range of `T`, or `None` if a bound is not
/// a `T`.
fn typed<T: RangeSubtype>(range: Range<Value>) -> Option<Range<T>> {
    let bound = |bound: ops::Bound<Value>| -> Option<ops::Bound<T>> {
        Some(match bound {
            ops::Bound::Included(value) => ops::Bound::Included(T::try_from(value).ok()?),
            ops::Bound::Excluded(value) => ops::Bound::Excluded(T::try_from(value).ok()?),
            ops::Bound::Unbounded => ops::Bound::Unbounded,
        })
    };
    Some(match range {
        Range::Empty => Range::Empty,
        Range::Bounds { lower, upper } => Range::new(bound(lower)?, bound(upper)?),
    })
}

/// The text form Rust's `Display` writes for these ranges, as a range or a
/// multirange.
fn written<T: RangeSubtype>(ranges: Vec<Range<Value>>, multirange: bool) -> Option<String> {
    let ranges = ranges
        .into_iter()
        .map(typed::<T>)
        .collect::<Option<Vec<_>>>()?;
    if multirange {
        Some(Multirange::from(ranges).to_string())
    } else {
        ranges.into_iter().next().map(|range| range.to_string())
    }
}

/// The ranges a text form holds, read with `T`'s own `FromStr`.
fn parsed<T: RangeSubtype>(text: &str, multirange: bool) -> Option<Vec<Range<Value>>> {
    let ranges: Vec<Range<T>> = if multirange {
        text.parse::<Multirange<T>>().ok()?.into_iter().collect()
    } else {
        vec![text.parse::<Range<T>>().ok()?]
    };
    Some(
        ranges
            .into_iter()
            .map(|range| range.map(Into::into))
            .collect(),
    )
}

/// A range type a schema created, named in full, and the value kind of its
/// subtype: `CreatedRange("floatrange", "f64", schema="measure")`.
#[pyclass(name = "CreatedRange", module = "pgorm", frozen, eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PyCreatedRange(pub(crate) CreatedKind);

/// The multirange PostgreSQL creates beside such a range, named by its own
/// name: `CreatedMultirange("floatmultirange", "f64")`.
#[pyclass(
    name = "CreatedMultirange",
    module = "pgorm",
    frozen,
    eq,
    from_py_object
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PyCreatedMultirange(pub(crate) CreatedKind);

macro_rules! created_kind_methods {
    ($class:ident, $multirange:literal, $label:literal) => {
        #[pymethods]
        impl $class {
            #[new]
            #[pyo3(signature = (name, subtype, *, schema=None))]
            fn new(
                name: &Bound<'_, PyAny>,
                subtype: &str,
                schema: Option<&Bound<'_, PyAny>>,
            ) -> PyResult<Self> {
                Ok(Self(CreatedKind::new(name, subtype, schema, $multirange)?))
            }

            #[getter]
            fn name(&self) -> String {
                self.0.name.name.clone()
            }

            #[getter]
            fn schema(&self) -> Option<String> {
                self.0.name.schema.clone()
            }

            #[getter]
            fn subtype(&self) -> &'static str {
                self.0.subtype.kind_name()
            }

            fn __hash__(&self) -> u64 {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                (&self.0.name.name, &self.0.name.schema, self.subtype()).hash(&mut hasher);
                hasher.finish()
            }

            fn __repr__(&self) -> String {
                let schema = match &self.0.name.schema {
                    Some(schema) => format!(", schema={schema:?}"),
                    None => String::new(),
                };
                format!(
                    concat!($label, "({:?}, {:?}{})"),
                    self.0.name.name,
                    self.subtype(),
                    schema
                )
            }
        }
    };
}

created_kind_methods!(PyCreatedRange, false, "CreatedRange");
created_kind_methods!(PyCreatedMultirange, true, "CreatedMultirange");

/// The created kind `value` is, if it is a `CreatedRange` or
/// `CreatedMultirange`.
pub(crate) fn extract(value: &Bound<'_, PyAny>) -> Option<CreatedKind> {
    if let Ok(kind) = value.extract::<PyRef<'_, PyCreatedRange>>() {
        Some(kind.0.clone())
    } else {
        value
            .extract::<PyRef<'_, PyCreatedMultirange>>()
            .ok()
            .map(|kind| kind.0.clone())
    }
}
