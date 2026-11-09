use crate::panics::future_into_py;
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyList, PyTuple},
};
use tokio_postgres::Row;

use super::PyRecord;
use crate::{
    errors::{DecodeError, InternalError},
    execution::{Executor, Target},
    statements,
    transactions::work::{Output, Work},
};

// [spec:pgorm:req:bindings.results+2]
pub(crate) fn execute<'py>(
    py: Python<'py>,
    target: Target,
    query: &Bound<'_, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    target.check(py)?;
    let compiled = statements::compile(query)?;
    future_into_py(py, async move {
        match target.run(Work::Execute(compiled)).await? {
            Output::Count(count) => Ok(count),
            _ => Err(InternalError::new_err("unexpected execution result")),
        }
    })
}

/// All three materializing terminals use the same completed Rust query result.
/// Cardinality checks never turn database/decode failures into a missing row.
pub(crate) fn fetch<'py>(
    py: Python<'py>,
    executor: Executor,
    query: &Bound<'_, PyAny>,
    mode: &str,
) -> PyResult<Bound<'py, PyAny>> {
    fetched(py, executor, query, mode, PyRecord::decode, |py, record| {
        Ok(Py::new(py, record)?.into_any())
    })
}

/// [`fetch`] with each row a tuple of its values in column order, read by
/// position as Rust's `into_tuple` reads one, so a result naming one column
/// twice is read rather than refused.
// [spec:pgorm:req:bindings.pipeline-models+1]
pub(crate) fn fetch_tuples<'py>(
    py: Python<'py>,
    executor: Executor,
    query: &Bound<'_, PyAny>,
    mode: &str,
) -> PyResult<Bound<'py, PyAny>> {
    fetched(
        py,
        executor,
        query,
        mode,
        |row| {
            (0..row.len())
                .map(|index| super::decode::value(super::decode::Field::Row(row, index)))
                .collect::<PyResult<Vec<_>>>()
        },
        |py, values| {
            let values = values
                .iter()
                .map(|value| value.to_python(py))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyTuple::new(py, values)?.into_any().unbind())
        },
    )
}

fn fetched<'py, T: Send + 'static>(
    py: Python<'py>,
    executor: Executor,
    query: &Bound<'_, PyAny>,
    mode: &str,
    decode: fn(&Row) -> PyResult<T>,
    into_py: fn(Python<'_>, T) -> PyResult<Py<PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    executor.check(py)?;
    let compiled = statements::compile(query)?;
    let mode = match mode {
        "all" => 0,
        "one" => 1,
        "optional" => 2,
        _ => {
            return Err(PyTypeError::new_err("unknown result terminal"));
        }
    };
    future_into_py(py, async move {
        let Output::Rows(rows) = executor.run(Work::Fetch(compiled)).await? else {
            return Err(InternalError::new_err("unexpected query result"));
        };
        // [spec:pgorm:req:bindings.row-count]
        if (mode == 1 && rows.len() != 1) || (mode == 2 && rows.len() > 1) {
            return Err(DecodeError::new_err(format!(
                "expected {} row, received {}",
                if mode == 1 {
                    "exactly one"
                } else {
                    "at most one"
                },
                rows.len()
            )));
        }
        let decoded = rows.iter().map(decode).collect::<PyResult<Vec<_>>>()?;
        Python::attach(|py| match mode {
            0 => {
                let items = decoded
                    .into_iter()
                    .map(|item| into_py(py, item))
                    .collect::<PyResult<Vec<_>>>()?;
                Ok(PyList::new(py, items)?.into_any().unbind())
            }
            _ => match decoded.into_iter().next() {
                Some(item) => into_py(py, item),
                None => Ok(py.None()),
            },
        })
    })
}
