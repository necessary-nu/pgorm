use crate::panics::future_into_py;
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyList};

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
        let records = rows
            .into_iter()
            .map(|row| PyRecord::decode(&row))
            .collect::<PyResult<Vec<_>>>()?;
        Python::attach(|py| match mode {
            0 => Ok(PyList::new(py, records)?.into_any().unbind()),
            _ => match records.into_iter().next() {
                Some(record) => Ok(Py::new(py, record)?.into_any()),
                None => Ok(py.None()),
            },
        })
    })
}
