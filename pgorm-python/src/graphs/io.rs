use super::backend::Query;
use crate::{
    errors::{ConstructionError, InternalError},
    execution::Executor,
    transactions::work::{Output, Work},
};
use pyo3::{prelude::*, types::PyList};
use pyo3_async_runtimes::tokio::future_into_py;

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
    future_into_py(py, async move {
        let work = Work::Graph(query, optional);
        let Output::Graph(rows) = target.run(work).await? else {
            return Err(InternalError::new_err("unexpected graph result"));
        };
        Python::attach(|py| {
            let rows = rows
                .into_iter()
                .map(|row| crate::entities::reads::row_object(py, row))
                .collect::<PyResult<Vec<_>>>()?;
            if optional {
                Ok(rows.into_iter().next().unwrap_or_else(|| py.None()))
            } else {
                Ok(PyList::new(py, rows)?.into_any().unbind())
            }
        })
    })
}
