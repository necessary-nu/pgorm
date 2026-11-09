use pyo3::{exceptions::PyKeyError, prelude::*, types::PyTuple};

use pgorm_bindings::entity::Model;

use super::{PyActiveModel, input};
use crate::{errors::Raise, values::PyValue};

#[derive(Clone, Debug)]
#[pyclass(name = "EntityModel", module = "pgorm", frozen, from_py_object)]
pub struct PyEntityModel {
    pub(crate) inner: Model,
}

impl PyEntityModel {
    pub(crate) fn validate(&self, py: Python<'_>) -> PyResult<()> {
        for column in &self.inner.info().columns {
            self.tagged(&column.name)?.to_python(py)?;
        }
        Ok(())
    }
}

#[pymethods]
impl PyEntityModel {
    #[getter]
    fn entity_name(&self) -> &str {
        &self.inner.info().name
    }

    fn tagged(&self, name: &str) -> PyResult<PyValue> {
        let column = self
            .inner
            .info()
            .column(name)
            .map_err(|_| PyKeyError::new_err(name.to_owned()))?;
        input::tagged(column, self.inner.get(name).py()?)
    }

    fn __getitem__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        self.tagged(name)?.to_python(py)
    }
    fn __len__(&self) -> usize {
        self.inner.info().columns.len()
    }

    fn keys<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            self.inner.info().columns.iter().map(|c| c.name.as_str()),
        )
    }

    fn values<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let values = self
            .inner
            .info()
            .columns
            .iter()
            .map(|c| self.__getitem__(py, &c.name))
            .collect::<PyResult<Vec<_>>>()?;
        PyTuple::new(py, values)
    }

    fn items<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let values = self
            .inner
            .info()
            .columns
            .iter()
            .map(|c| Ok((c.name.clone(), self.__getitem__(py, &c.name)?)))
            .collect::<PyResult<Vec<_>>>()?;
        PyTuple::new(py, values)
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.keys(py)?.call_method0("__iter__")
    }

    fn with_value(&self, column: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = super::column::name(column, self.inner.info())?;
        let value = input::coerce(self.inner.info().column(&name).py()?, value)?;
        Ok(Self {
            inner: self.inner.set(&name, value.inner).py()?,
        })
    }

    #[pyo3(name = "into_active")]
    fn to_active(&self) -> PyActiveModel {
        PyActiveModel {
            inner: self.inner.active(),
        }
    }
    fn __repr__(&self) -> String {
        format!(
            "EntityModel({:?}, fields={})",
            self.entity_name(),
            self.__len__()
        )
    }
}
