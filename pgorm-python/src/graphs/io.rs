use crate::{
    errors::Raise,
    execution::{Executor, listed},
    transactions::work::Work,
};
use pgorm_bindings::entity::GraphQuery as Query;
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
    pgorm_bindings::parameter_count(query.compile(optional).1.0.len()).py()?;
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
