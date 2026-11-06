use super::backend::Query;
use crate::{
    errors::ConstructionError,
    execution::{Executor, listed},
    transactions::work::Work,
};
use pyo3::prelude::*;

pub(crate) enum Read {
    All,
    Optional,
}

// [spec:pgorm:req:bindings.entity-graphs+4]
pub(crate) fn read<'py>(
    py: Python<'py>,
    connection: &Bound<'_, PyAny>,
    query: Query,
    terminal: Read,
) -> PyResult<Bound<'py, PyAny>> {
    let target = Executor::extract(py, connection)?;
    let optional = matches!(terminal, Read::Optional);
    if query.compile(optional).1.0.len() > 65535 {
        return Err(ConstructionError::new_err(
            "PostgreSQL supports at most 65535 query parameters",
        ));
    }
    listed(
        py,
        target,
        Work::Graph(query, optional),
        optional,
        "graph",
        crate::entities::reads_py::graph_rows,
        crate::entities::reads::row_object,
    )
}
