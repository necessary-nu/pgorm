//! Closed runtime type constructors over pgorm_query::ColumnType.

use crate::{UnsupportedCapabilityError, errors::ConstructionError, values::PyTypeName};
use pgorm::pgorm_query::{ColumnType, IntervalPrecision, IntervalSpec, Name, PgInterval};
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
        let refused =
            |refusal: pgorm_bindings::Refusal| ConstructionError::new_err(refusal.into_message());
        let sized = length.is_some() || precision.is_some() || scale.is_some();
        if let Some(created) = crate::values::created_kind(kind) {
            if sized {
                return Err(refused(pgorm_bindings::sized_named_type()));
            }
            return Ok(Self {
                inner: pgorm_bindings::created_column(&created).map_err(refused)?,
            });
        }
        if let Ok(name) = kind.extract::<PyRef<'_, PyTypeName>>() {
            if sized {
                return Err(refused(pgorm_bindings::sized_named_type()));
            }
            return Ok(Self {
                inner: pgorm_bindings::enum_column(&name),
            });
        }
        let kind = kind.extract::<&str>().map_err(|_| {
            PyTypeError::new_err("DataType requires a supported type name or TypeName")
        })?;
        let inner = pgorm_bindings::builtin_column(kind, length, precision, scale)
            .map_err(refused)?
            .ok_or_else(|| {
                UnsupportedCapabilityError::new_err(
                    pgorm_bindings::unknown_type(kind).into_message(),
                )
            })?;
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
