use pgorm::pgorm_query::Values;
use pyo3::prelude::*;

use crate::{errors::ConstructionError, values::PyValue};

/// A Rust-built SQL statement and its independently owned, ordered parameters.
#[pyclass(name = "Compiled", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct Compiled {
    #[pyo3(get)]
    pub sql: String,
    pub values: Values,
}

#[pymethods]
impl Compiled {
    #[getter]
    fn params(&self) -> Vec<PyValue> {
        self.values
            .0
            .iter()
            .cloned()
            .map(PyValue::from_rust)
            .collect()
    }

    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "compiled SQL cannot be tested as Python booleans",
        ))
    }
    fn __repr__(&self) -> String {
        format!("Compiled(parameters={})", self.values.0.len())
    }
}
