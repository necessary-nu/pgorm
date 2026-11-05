use pyo3::prelude::*;

use super::backend::{Active, Write};
use crate::values::PyValue;

#[derive(Clone, Debug, PartialEq, Eq)]
#[pyclass(module = "pgorm", eq, from_py_object)]
pub enum ActiveState {
    NotSet,
    Set,
    Unchanged,
}

#[derive(Clone, Debug)]
#[pyclass(
    name = "ActiveValue",
    module = "pgorm",
    frozen,
    get_all,
    from_py_object
)]
pub struct PyActiveValue {
    state: ActiveState,
    value: Option<PyValue>,
}

#[derive(Clone, Debug)]
#[pyclass(name = "ActiveModel", module = "pgorm", frozen, from_py_object)]
pub struct PyActiveModel {
    pub(crate) inner: Active,
}

// [spec:pgorm:req:bindings.entity-writes+1]
#[pymethods]
impl PyActiveModel {
    #[getter]
    fn entity_name(&self) -> &str {
        &self.inner.info().name
    }

    fn get(&self, column: &Bound<'_, PyAny>) -> PyResult<PyActiveValue> {
        let name = super::column::name(column, self.inner.info())?;
        let (state, value) = match self
            .inner
            .get(&name)
            .map_err(|error| crate::errors::ConstructionError::new_err(error.to_string()))?
        {
            pgorm::ActiveValue::NotSet => (ActiveState::NotSet, None),
            pgorm::ActiveValue::Set(value) => (ActiveState::Set, Some(value)),
            pgorm::ActiveValue::Unchanged(value) => (ActiveState::Unchanged, Some(value)),
        };
        let value = value
            .map(|v| self.inner.info().column(&name)?.input.tagged(v))
            .transpose()?;
        Ok(PyActiveValue { state, value })
    }

    fn set(&self, column: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = super::column::name(column, self.inner.info())?;
        let value = self.inner.info().column(&name)?.input.coerce(value)?;
        Ok(Self {
            inner: self
                .inner
                .set(&name, value.inner)
                .map_err(|error| crate::errors::ConstructionError::new_err(error.to_string()))?,
        })
    }

    fn not_set(&self, column: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = super::column::name(column, self.inner.info())?;
        Ok(Self {
            inner: self
                .inner
                .not_set(&name)
                .map_err(|error| crate::errors::ConstructionError::new_err(error.to_string()))?,
        })
    }

    fn reset(&self, column: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = super::column::name(column, self.inner.info())?;
        Ok(Self {
            inner: self
                .inner
                .reset(&name)
                .map_err(|error| crate::errors::ConstructionError::new_err(error.to_string()))?,
        })
    }

    fn insert<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::io::write(py, connection, self.inner.clone(), Write::Insert)
    }
    fn update<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::io::write(py, connection, self.inner.clone(), Write::Update)
    }
    fn delete<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::io::write(py, connection, self.inner.clone(), Write::Delete)
    }
    fn __repr__(&self) -> String {
        format!("ActiveModel({:?})", self.entity_name())
    }
}
