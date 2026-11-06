use super::backend::{CursorPlan, Query};
use crate::panics::future_into_py;
use crate::{
    errors::{ConstructionError, InternalError},
    execution::Executor,
    transactions::work::{Output, Work},
};
use pyo3::{
    IntoPyObjectExt,
    prelude::*,
    types::{PyList, PyTuple},
};

pub(crate) enum Read {
    All,
    Optional,
    Cursor(CursorPlan),
}

// [spec:pgorm:req:bindings.entity-graphs+3]
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
        let work = match terminal {
            Read::Cursor(plan) => Work::Cursor(query, plan),
            _ => Work::Graph(query, optional),
        };
        let Output::Graph(rows) = target.run(work).await? else {
            return Err(InternalError::new_err("unexpected graph result"));
        };
        Python::attach(|py| {
            let rows = rows
                .into_iter()
                .map(|row| {
                    for model in row.iter().flatten() {
                        model.validate(py)?;
                    }
                    if row.len() == 1 {
                        row.into_iter().next().flatten().into_py_any(py)
                    } else {
                        Ok(PyTuple::new(py, row)?.into_any().unbind())
                    }
                })
                .collect::<PyResult<Vec<_>>>()?;
            if optional {
                Ok(rows.into_iter().next().unwrap_or_else(|| py.None()))
            } else {
                Ok(PyList::new(py, rows)?.into_any().unbind())
            }
        })
    })
}
