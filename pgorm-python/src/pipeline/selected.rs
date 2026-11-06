use super::PyPipeline;
use crate::{
    UnsupportedCapabilityError,
    entities::{NativeRegistry, reads_py::compiled},
    errors::{ConstructionError, Raise},
    expressions::Compiled,
};
use pgorm_bindings::entity::{Read as Terminal, Selected as Query, SourcesBackend};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyString};
use std::sync::Arc;

/// The terminal `value` names.
pub(super) fn parse_terminal(value: &str) -> PyResult<Terminal> {
    match value {
        "all" => Ok(Terminal::All),
        "one" => Ok(Terminal::One),
        "optional" => Ok(Terminal::Optional),
        _ => Err(PyTypeError::new_err("unknown pipeline terminal")),
    }
}

/// The statement `query`'s terminal sends.
pub(super) fn statement(query: &Query, terminal: Terminal) -> PyResult<Compiled> {
    compiled(
        query
            .compile(terminal)
            .map_err(|error| ConstructionError::new_err(error.to_string()))?,
    )
}

#[derive(Debug, Clone)]
#[pyclass(
    name = "SourceSelection",
    module = "pgorm.pipeline",
    frozen,
    from_py_object
)]
pub struct PySourceSelection {
    pub(crate) factory: Arc<dyn SourcesBackend>,
}

#[pymethods]
impl PySourceSelection {
    #[getter]
    fn name(&self) -> &str {
        &self.factory.info().name
    }
    fn describe<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        py.import("json")?
            .call_method1("loads", (self.factory.info().describe().to_string(),))
    }
}

impl PySourceSelection {
    pub(super) fn select(
        &self,
        pipeline: &PyPipeline,
        qualifiers: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySelectedSources> {
        let qualifiers = match qualifiers.filter(|v| !v.is_none()) {
            Some(value) if !value.is_instance_of::<PyString>() => {
                Some(value.extract::<Vec<String>>().map_err(|_| {
                    PyTypeError::new_err("qualifiers require a sequence of source names")
                })?)
            }
            Some(_) => {
                return Err(PyTypeError::new_err(
                    "qualifiers require a sequence, not one string",
                ));
            }
            None => None,
        };
        let qualifiers = self.factory.info().qualifiers(qualifiers).py()?;
        Ok(PySelectedSources {
            inner: self.factory.select(pipeline.inner.clone(), qualifiers),
        })
    }
}

#[derive(Debug, Clone)]
#[pyclass(
    name = "SelectedSources",
    module = "pgorm.pipeline",
    frozen,
    from_py_object
)]
pub struct PySelectedSources {
    inner: Query,
}

#[pymethods]
impl PySelectedSources {
    #[pyo3(signature=(*, terminal="all"))]
    fn inspect(&self, terminal: &str) -> PyResult<Compiled> {
        statement(&self.inner, parse_terminal(terminal)?)
    }
    fn all<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::selected_io::read(py, connection, self.inner.clone(), Terminal::All)
    }
    fn one<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::selected_io::read(py, connection, self.inner.clone(), Terminal::One)
    }
    fn optional<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::selected_io::read(py, connection, self.inner.clone(), Terminal::Optional)
    }
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "source selections cannot be tested as Python booleans",
        ))
    }
}

#[pyfunction(pass_module)]
pub(crate) fn pipeline_sources(
    module: &Bound<'_, PyModule>,
    name: &str,
) -> PyResult<PySourceSelection> {
    let object = module.getattr("_registry")?;
    let registry = object.extract::<PyRef<'_, NativeRegistry>>()?;
    registry
        .0
        .0
        .source_tuples()
        .get(name)
        .cloned()
        .map(|factory| PySourceSelection { factory })
        .ok_or_else(|| {
            UnsupportedCapabilityError::new_err(
                "source tuple is not registered in this native build",
            )
        })
}
