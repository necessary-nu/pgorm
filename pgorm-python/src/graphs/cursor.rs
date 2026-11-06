use crate::{entities::reads_py::CursorState, errors::ConstructionError, expressions::Compiled};
use pyo3::{prelude::*, types::PyTuple};

/// A registered graph's keyset cursor, `SelectGraph::cursor_by` or
/// `cursor_by_on`, every decoded source's primary key installed after its
/// order columns as a tiebreak.
#[derive(Clone, Debug)]
#[pyclass(name = "EntityGraphCursor", module = "pgorm", frozen, from_py_object)]
pub struct PyEntityGraphCursor {
    pub(crate) state: CursorState,
}

// [spec:pgorm:req:bindings.entity-graphs+4]
#[pymethods]
impl PyEntityGraphCursor {
    /// Rows short of the one whose order-column values these are.
    #[pyo3(signature = (*values))]
    fn before(&self, values: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            state: self.state.bound(true, values, false)?,
        })
    }
    #[pyo3(signature = (*values))]
    fn after(&self, values: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            state: self.state.bound(false, values, false)?,
        })
    }
    /// Rows short of the one whose order-column values, or whole key, these
    /// are: the order columns, the ordered source's other key columns, then
    /// each other source's.
    #[pyo3(signature = (*values))]
    fn before_with(&self, values: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            state: self.state.bound(true, values, true)?,
        })
    }
    #[pyo3(signature = (*values))]
    fn after_with(&self, values: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            state: self.state.bound(false, values, true)?,
        })
    }
    fn first(&self, count: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            state: self.state.window(false, count)?,
        })
    }
    fn last(&self, count: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            state: self.state.window(true, count)?,
        })
    }
    fn asc(&self) -> Self {
        Self {
            state: self.state.direction(false),
        }
    }
    fn desc(&self) -> Self {
        Self {
            state: self.state.direction(true),
        }
    }
    /// The statement `all` sends, `Cursor::statement`.
    fn inspect(&self) -> PyResult<Compiled> {
        self.state.inspect()
    }
    fn all<'py>(
        &self,
        py: Python<'py>,
        connection: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.state.all(py, connection)
    }
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "graph cursors cannot be tested as Python booleans",
        ))
    }
}
