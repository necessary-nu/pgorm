//! Borrowed execution context: all calls preserve the selected native executor.

mod target;
pub(crate) use target::{Executor, Target};

use pyo3::{prelude::*, types::PyList};

use crate::{
    errors::InternalError,
    metrics::Metrics,
    panics::future_into_py,
    transactions::work::{Output, Work},
};

/// What an operation runs on: a connection or a transaction, plain or, from
/// an instrumented pool, pgorm's metrics wrapper around one.
pub(crate) type Database<'a> = pgorm::DatabaseRef<'a, Metrics>;

/// Run `work` on `target` and give the rows `rows` takes from its output as
/// Python reads them, each made by `each`: a list, or with `single` the first
/// row or `None`. An output of another kind is the binding's own failure, a
/// `what` result it did not expect.
pub(crate) fn listed<'py, T: Send + 'static>(
    py: Python<'py>,
    target: Executor,
    work: Work,
    single: bool,
    what: &'static str,
    rows: fn(Output) -> Option<Vec<T>>,
    each: fn(Python<'_>, T) -> PyResult<Py<PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    future_into_py(py, async move {
        let Some(rows) = rows(target.run(work).await?) else {
            return Err(InternalError::new_err(format!("unexpected {what} result")));
        };
        Python::attach(|py| {
            let mut rows = rows
                .into_iter()
                .map(|row| each(py, row))
                .collect::<PyResult<Vec<_>>>()?;
            if single {
                return Ok(if rows.is_empty() {
                    py.None()
                } else {
                    rows.swap_remove(0)
                });
            }
            Ok(PyList::new(py, rows)?.into_any().unbind())
        })
    })
}
