use pgorm::pgorm_query::{Values, inject_parameters};
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyList, PyTuple},
};

use super::common;
use crate::{
    errors::{ConstructionError, Raise},
    expressions::Compiled,
    values::PyValue,
};

// [spec:pgorm:req:python.raw]
/// Explicit application SQL text with separately owned Rust parameters.
#[pyclass(name = "RawSQL", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct RawSQL {
    #[pyo3(get)]
    pub template: String,
    pub values: Values,
}

#[pymethods]
impl RawSQL {
    #[new]
    #[pyo3(signature = (template, values=None))]
    pub(crate) fn new(template: String, values: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        if template.contains('\0') {
            return Err(ConstructionError::new_err("SQL text cannot contain NUL"));
        }
        let values = match values {
            None => Vec::new(),
            Some(values) => {
                if !values.is_exact_instance_of::<PyList>()
                    && !values.is_exact_instance_of::<PyTuple>()
                {
                    return Err(PyTypeError::new_err(
                        "raw parameters require a list or tuple",
                    ));
                }
                values
                    .try_iter()?
                    .map(|value| PyValue::coerce(&value?).map(|value| value.rust_value().clone()))
                    .collect::<PyResult<_>>()?
            }
        };
        pgorm_bindings::parameter_count(values.len()).py()?;
        Ok(Self {
            template,
            values: Values(values),
        })
    }

    pub fn inspect(&self) -> PyResult<Compiled> {
        common::compiled((self.template.clone(), self.values.clone()))
    }

    /// Inline parameters with pgorm's PostgreSQL lexer and Rust renderer.
    fn inline_sql(&self) -> PyResult<String> {
        inject_parameters(&self.template, self.values.0.iter().cloned())
            .map_err(|error| ConstructionError::new_err(error.to_string()))
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL statements cannot be tested as Python booleans",
        ))
    }
    fn __repr__(&self) -> String {
        format!("RawSQL(parameters={})", self.values.0.len())
    }
}
