use super::selected_backend::{Query, Terminal};
use crate::panics::future_into_py;
use crate::{
    errors::InternalError,
    execution::Executor,
    transactions::work::{Output, Work},
};
use pyo3::{
    prelude::*,
    types::{PyList, PyTuple},
};

// [spec:pgorm:req:bindings.pipeline-sources+3]
pub(super) fn read<'py>(
    py: Python<'py>,
    connection: &Bound<'_, PyAny>,
    query: Query,
    terminal: Terminal,
) -> PyResult<Bound<'py, PyAny>> {
    let target = Executor::extract(py, connection)?;
    query.compile(terminal)?;
    future_into_py(py, async move {
        let Output::Graph(rows) = target.run(Work::Sources(query, terminal)).await? else {
            return Err(InternalError::new_err("unexpected selected source result"));
        };
        Python::attach(|py| {
            let rows = rows
                .into_iter()
                .map(|row| {
                    for model in row.iter().flatten() {
                        model.validate(py)?;
                    }
                    // A one-source absent model is (None,), distinct from no result row.
                    Ok(PyTuple::new(py, row)?.into_any().unbind())
                })
                .collect::<PyResult<Vec<_>>>()?;
            match terminal {
                Terminal::All => Ok(PyList::new(py, rows)?.into_any().unbind()),
                _ => Ok(rows.into_iter().next().unwrap_or_else(|| py.None())),
            }
        })
    })
}
