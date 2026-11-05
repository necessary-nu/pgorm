//! A table read through `TABLESAMPLE`, over Rust's `TableSample`.

use pgorm::pgorm_query::{FromItem, NamedTable, SampleMethod, TableSample};
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyFloat, PyInt},
};

use crate::errors::ConstructionError;

/// How a sample draws its rows: each row on its own (`Bernoulli`) or whole
/// pages (`System`), PostgreSQL's two built-in methods.
#[pyclass(name = "SampleMethod", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PySampleMethod {
    Bernoulli,
    System,
}

#[pymethods]
impl PySampleMethod {
    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "sample methods cannot be tested as Python booleans",
        ))
    }
}

impl PySampleMethod {
    fn rust(&self) -> SampleMethod {
        match self {
            Self::Bernoulli => SampleMethod::Bernoulli,
            Self::System => SampleMethod::System,
        }
    }
}

/// An `int` or `float` as the double Rust binds. An `int` a double cannot
/// hold exactly is refused rather than rounded, and a `bool` is not a
/// number here.
fn double(value: &Bound<'_, PyAny>, what: &str) -> PyResult<f64> {
    if value.is_exact_instance_of::<PyFloat>() {
        return value.extract();
    }
    if !value.is_exact_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err(format!(
            "a sample {what} requires an int or float"
        )));
    }
    let exact = value
        .extract::<f64>()
        .ok()
        .filter(|double| value.eq(*double).unwrap_or(false));
    exact.ok_or_else(|| {
        ConstructionError::new_err(format!(
            "a sample {what} must be exactly representable as a double"
        ))
    })
}

// [spec:pgorm:req:bindings.select+4]
/// `table TABLESAMPLE method (percentage) [REPEATABLE (seed)]`, both numbers
/// bound; a percentage outside 0 to 100 is the server's refusal (`2202H`).
pub(super) fn sample(
    table: &NamedTable,
    method: &PySampleMethod,
    percentage: &Bound<'_, PyAny>,
    repeatable: Option<&Bound<'_, PyAny>>,
) -> PyResult<FromItem> {
    let mut sample = TableSample::new(
        table.clone(),
        method.rust(),
        double(percentage, "percentage")?,
    );
    if let Some(seed) = repeatable {
        sample = sample.repeatable(double(seed, "seed")?);
    }
    Ok(sample.into())
}
