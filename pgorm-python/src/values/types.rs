use pgorm::pgorm_query::{ArrayType, Name, TypeName, Value};
use pyo3::prelude::*;

use super::created::CreatedKind;
use crate::{UnsupportedCapabilityError, errors::ConstructionError};

/// The refusal of a value, or value kind, of pgorm's that this package has no
/// Python form of. pgorm's value vocabulary grows, and a kind added after the
/// binding was written is reported rather than read as another.
pub(crate) fn unrepresented() -> PyErr {
    UnsupportedCapabilityError::new_err("a kind of pgorm value pgorm-python does not represent")
}

// [spec:pgorm:req:bindings.value-tags+1]
/// An owned, identifier-only PostgreSQL type name, optionally schema qualified.
#[pyclass(name = "TypeName", module = "pgorm", frozen, eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PyTypeName {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub schema: Option<String>,
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
        Ok(Self { name, schema })
    }

    fn __repr__(&self) -> String {
        format!("TypeName({:?}, schema={:?})", self.name, self.schema)
    }
}

impl PyTypeName {
    /// Lower through Rust's qualified identifier constructor, never verbatim SQL.
    pub fn rust_type(&self) -> TypeName {
        let name = TypeName::new(Name::runtime(&self.name));
        match &self.schema {
            Some(schema) => name.schema(Name::runtime(schema)),
            None => name,
        }
    }
}

/// The kind a pgorm value declares itself.
pub fn rust_tag(value: &Value) -> Tag {
    let kind = Tag::Scalar(value.array_type());
    if value.is_array() {
        Tag::Array(Box::new(kind))
    } else {
        kind
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    Scalar(ArrayType),
    Enum(PyTypeName),
    Created(CreatedKind),
    Array(Box<Tag>),
}

impl Tag {
    // [spec:pgorm:req:bindings.argument-errors+1]
    pub fn parse(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(name) = value.extract::<PyRef<'_, PyTypeName>>() {
            Ok(Self::Enum(name.clone()))
        } else if let Some(kind) = super::created::extract(value) {
            Ok(Self::Created(kind))
        } else {
            ArrayType::from_kind_name(value.extract::<&str>()?)
                .map(Self::Scalar)
                .ok_or_else(|| ConstructionError::new_err("unknown scalar value kind"))
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Scalar(kind) => kind.kind_name(),
            Self::Enum(_) => "enum",
            Self::Created(kind) => kind.kind_name(),
            Self::Array(_) => "array",
        }
    }

    pub fn array_type(&self) -> PyResult<ArrayType> {
        match self {
            Self::Scalar(kind) => Ok(kind.clone()),
            Self::Enum(_) | Self::Created(_) => Ok(ArrayType::String),
            Self::Array(_) => Err(ConstructionError::new_err(
                "nested arrays are not supported",
            )),
        }
    }

    pub fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self {
            Self::Enum(name) => Ok(Py::new(py, name.clone())?.into_any()),
            Self::Created(kind) => kind.to_python(py),
            _ => Ok(self.name().into_pyobject(py)?.into_any().unbind()),
        }
    }
}
