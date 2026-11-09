use pgorm::pgorm_query::ArrayType;
pub(crate) use pgorm_bindings::{Tag, TypeName};
use pyo3::prelude::*;

use super::created::CreatedKindPy as _;
use crate::{UnsupportedCapabilityError, errors::ConstructionError};

/// The refusal of a value, or value kind, of pgorm's that this package has no
/// Python form of. pgorm's value vocabulary grows, and a kind added after the
/// binding was written is reported rather than read as another.
pub(crate) fn unrepresented() -> PyErr {
    UnsupportedCapabilityError::new_err("a kind of pgorm value pgorm-python does not represent")
}

// [spec:pgorm:req:bindings.value-tags+1]
/// An owned, identifier-only PostgreSQL type name, optionally schema
/// qualified: pgorm-bindings' `TypeName` as Python holds it.
#[pyclass(name = "TypeName", module = "pgorm", frozen, eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PyTypeName(pub TypeName);

impl std::ops::Deref for PyTypeName {
    type Target = TypeName;

    fn deref(&self) -> &TypeName {
        &self.0
    }
}

#[pymethods]
impl PyTypeName {
    #[new]
    #[pyo3(signature = (name, *, schema=None))]
    pub(crate) fn new(
        name: &Bound<'_, PyAny>,
        schema: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let name = name.extract::<String>()?;
        let schema = schema
            .map(|schema| schema.extract::<String>())
            .transpose()?;
        for part in std::iter::once(&name).chain(schema.iter()) {
            if part.is_empty() || part.contains('\0') || part.len() > 63 {
                return Err(ConstructionError::new_err(
                    "type name parts must contain 1–63 UTF-8 bytes without NUL",
                ));
            }
        }
        Ok(Self(TypeName { name, schema }))
    }

    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }

    #[getter]
    fn schema(&self) -> Option<&str> {
        self.0.schema.as_deref()
    }

    fn __repr__(&self) -> String {
        format!("TypeName({:?}, schema={:?})", self.0.name, self.0.schema)
    }
}

/// A kind as Python reads and writes it: parsed from a kind name, a
/// `TypeName` or a created range type, and spelled back as one.
pub(crate) trait TagPy: Sized {
    /// The kind `value` names.
    fn parse(value: &Bound<'_, PyAny>) -> PyResult<Self>;
    /// The `ArrayType` an array of the kind holds.
    fn array_type(&self) -> PyResult<ArrayType>;
    /// The kind as Python names it.
    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>>;
}

impl TagPy for Tag {
    // [spec:pgorm:req:bindings.argument-errors+1]
    fn parse(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(name) = value.extract::<PyRef<'_, PyTypeName>>() {
            Ok(Self::Enum(name.0.clone()))
        } else if let Some(kind) = super::created::extract(value) {
            Ok(Self::Created(kind))
        } else {
            ArrayType::from_kind_name(value.extract::<&str>()?)
                .map(Self::Scalar)
                .ok_or_else(|| ConstructionError::new_err("unknown scalar value kind"))
        }
    }

    fn array_type(&self) -> PyResult<ArrayType> {
        match self {
            Self::Scalar(kind) => Ok(kind.clone()),
            Self::Enum(_) | Self::Created(_) => Ok(ArrayType::String),
            Self::Array(_) => Err(ConstructionError::new_err(
                "nested arrays are not supported",
            )),
        }
    }

    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self {
            Self::Enum(name) => Ok(Py::new(py, PyTypeName(name.clone()))?.into_any()),
            Self::Created(kind) => kind.to_python(py),
            _ => Ok(self.name().into_pyobject(py)?.into_any().unbind()),
        }
    }
}
