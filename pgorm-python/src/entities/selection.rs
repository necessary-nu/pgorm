//! A registered read's relations, for a query to join, and its projection
//! onto some of its columns, read as records.

use std::sync::Arc;

use pyo3::{exceptions::PyTypeError, prelude::*};

use pgorm_bindings::entity::{EntityBackend, Read as Terminal, Select};

use super::reads_py::{compiled, stream};
use crate::{
    errors::{ConstructionError, Raise},
    execution::Executor,
    expressions::Compiled,
};

/// A relation a registered entity's `Relation` enum declares, by its
/// variant's name, for a query to join.
#[pyclass(name = "EntityRelation", module = "pgorm", frozen, skip_from_py_object)]
#[derive(Clone, Debug)]
pub struct PyEntityRelation {
    pub(crate) entity: Arc<dyn EntityBackend>,
    pub(crate) name: String,
}

#[pymethods]
impl PyEntityRelation {
    #[getter]
    fn entity_name(&self) -> &str {
        &self.entity.info().name
    }

    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    fn __repr__(&self) -> String {
        format!("EntityRelation({:?}, {:?})", self.entity_name(), self.name)
    }
}

/// A registered `Select<E>` projected onto some of its columns, through
/// `select_only` and `columns`: its rows are records of those columns, each
/// read through the column's `select_as`, rather than models.
// [spec:pgorm:req:bindings.entity-reads+4]
#[pyclass(
    name = "EntitySelection",
    module = "pgorm",
    frozen,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
pub struct PyEntitySelection {
    pub(crate) select: Select,
    pub(crate) columns: Vec<String>,
}

impl PyEntitySelection {
    pub(crate) fn statement(&self, terminal: Terminal) -> PyResult<Compiled> {
        compiled(self.select.projected(&self.columns, terminal).py()?.build())
    }

    fn fetch<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
        terminal: Terminal,
        mode: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let statement = Bound::new(py, self.statement(terminal)?)?.into_any();
        crate::results::fetch(py, Executor::extract(py, connection)?, &statement, mode)
    }
}

#[pymethods]
impl PyEntitySelection {
    #[getter]
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }

    #[pyo3(signature = (*, terminal="all"))]
    fn inspect(&self, terminal: &str) -> PyResult<Compiled> {
        self.statement(match terminal {
            "all" => Terminal::All,
            "one" => Terminal::One,
            "optional" => Terminal::Optional,
            _ => return Err(PyTypeError::new_err("unknown entity selection terminal")),
        })
    }

    fn all<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.fetch(py, connection, Terminal::All, "all")
    }

    /// The first row of the selection narrowed to one, a `DecodeError` when
    /// there is none.
    fn one<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.fetch(py, connection, Terminal::One, "one")
    }

    fn optional<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.fetch(py, connection, Terminal::Optional, "optional")
    }

    /// The rows as records, one per pull, over a connection the stream holds
    /// until it ends.
    fn stream<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        stream(py, connection, self.statement(Terminal::All)?, None)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity selections cannot be tested as Python booleans",
        ))
    }
}
