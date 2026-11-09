use pyo3::prelude::*;

use pgorm_bindings::entity::{Active, Write};

use super::input;
use crate::{errors::Raise, values::PyValue};

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
        let (state, value) = match self.inner.get(&name).py()? {
            pgorm::ActiveValue::NotSet => (ActiveState::NotSet, None),
            pgorm::ActiveValue::Set(value) => (ActiveState::Set, Some(value)),
            pgorm::ActiveValue::Unchanged(value) => (ActiveState::Unchanged, Some(value)),
        };
        let value = value
            .map(|v| input::tagged(self.inner.info().column(&name).py()?, v))
            .transpose()?;
        Ok(PyActiveValue { state, value })
    }

    fn set(&self, column: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = super::column::name(column, self.inner.info())?;
        let value = input::coerce(self.inner.info().column(&name).py()?, value)?;
        Ok(Self {
            inner: self.inner.set(&name, value.inner).py()?,
        })
    }

    fn not_set(&self, column: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = super::column::name(column, self.inner.info())?;
        Ok(Self {
            inner: self.inner.not_set(&name).py()?,
        })
    }

    // [spec:pgorm:req:bindings.entity-active-changes]
    fn set_if_not_equals(
        &self,
        column: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        let name = super::column::name(column, self.inner.info())?;
        let value = input::coerce(self.inner.info().column(&name).py()?, value)?;
        Ok(Self {
            inner: self.inner.set_if_not_equals(&name, value.inner).py()?,
        })
    }

    // [spec:pgorm:req:bindings.entity-active-changes]
    fn is_changed(&self) -> bool {
        self.inner.is_changed()
    }

    fn reset(&self, column: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = super::column::name(column, self.inner.info())?;
        Ok(Self {
            inner: self.inner.reset(&name).py()?,
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
