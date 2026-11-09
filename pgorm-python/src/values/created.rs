//! Range and multirange types a schema created: a kind naming the type and
//! its subtype, and the text form a value of one travels as.
//!
//! The Rust value of such a range is `Value::String` of its text form, the
//! `Value` a `CreatedRange` newtype converts into, because no range
//! tag can carry a name only the schema knows. The kind carries the name,
//! which the cast needs, and the subtype, which converts each bound with that
//! scalar kind's limits and reads the text back.

use pgorm::{
    CreatedRangeCodec,
    pgorm_query::{ArrayType, Multirange, Range, Value},
};
use pyo3::{prelude::*, types::PyString};

use super::{classes::PyMultirange, ranges, types::PyTypeName};
use crate::errors::{ConstructionError, DecodeError};

pub(crate) use pgorm_bindings::CreatedKind;

/// The created range or multirange type `name` names, in `schema`, over the
/// value kind `subtype`, refused for a subtype no created range ranges over.
fn created_kind(
    name: &Bound<'_, PyAny>,
    subtype: &str,
    schema: Option<&Bound<'_, PyAny>>,
    multirange: bool,
) -> PyResult<CreatedKind> {
    let subtype = ArrayType::from_kind_name(subtype)
        .filter(|kind| CreatedRangeCodec::new(kind, multirange).is_some())
        .ok_or_else(|| {
            ConstructionError::new_err(
                "a created range's subtype is one of i16, i32, i64, f32, f64, decimal, \
                 text, date, time, datetime, datetime_utc or uuid",
            )
        })?;
    Ok(CreatedKind {
        name: PyTypeName::new(name, schema)?.0,
        subtype,
        multirange,
    })
}

/// A created range or multirange type as Python reads and writes its
/// values.
pub(crate) trait CreatedKindPy {
    /// The kind as Python names it, a `CreatedRange` or `CreatedMultirange`.
    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>>;
    /// The text form of a Python value of this kind.
    fn text(&self, data: &Bound<'_, PyAny>) -> PyResult<String>;
    /// The Python value of this kind's text form.
    fn read(&self, py: Python<'_>, text: &str) -> PyResult<Py<PyAny>>;
}

/// pgorm's codec of `kind`'s values; a kind is made only over a subtype it
/// has one for.
fn codec(kind: &CreatedKind) -> PyResult<CreatedRangeCodec> {
    kind.codec()
        .ok_or_else(|| ConstructionError::new_err("a created range's subtype has no codec"))
}

/// A `pgorm.Range`'s, or each range of a `pgorm.Multirange`'s, bounds as
/// subtype values.
fn bounds(kind: &CreatedKind, data: &Bound<'_, PyAny>) -> PyResult<Vec<Range<Value>>> {
    if !kind.multirange {
        return Ok(vec![ranges::read_bounds(data, &kind.subtype)?]);
    }
    let multirange = data
        .cast_exact::<PyMultirange>()
        .map_err(|_| ConstructionError::new_err("expected pgorm.Multirange"))?;
    multirange
        .get()
        .ranges
        .iter()
        .map(|range| ranges::read_bounds(range.bind(data.py()).as_any(), &kind.subtype))
        .collect()
}

impl CreatedKindPy for CreatedKind {
    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.multirange {
            Ok(Py::new(py, PyCreatedMultirange(self.clone()))?.into_any())
        } else {
            Ok(Py::new(py, PyCreatedRange(self.clone()))?.into_any())
        }
    }

    /// A `pgorm.Range` (or a `pgorm.Multirange`), each bound converted as
    /// the subtype's kind, or the text form itself, which is read and
    /// written back canonically.
    // [spec:pgorm:req:bindings.created-ranges+5]
    fn text(&self, data: &Bound<'_, PyAny>) -> PyResult<String> {
        let codec = codec(self)?;
        let ranges = if let Ok(text) = data.cast_exact::<PyString>() {
            match codec.ranges(text.to_str()?) {
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
            bounds(self, data)?
        };
        codec
            .text(ranges)
            .ok_or_else(|| ConstructionError::new_err("a bound does not convert to the subtype"))
    }

    // [spec:pgorm:req:bindings.created-ranges+5]
    fn read(&self, py: Python<'_>, text: &str) -> PyResult<Py<PyAny>> {
        let ranges = codec(self)?.ranges(text).ok_or_else(|| {
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
                Ok(Self(created_kind(name, subtype, schema, $multirange)?))
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
