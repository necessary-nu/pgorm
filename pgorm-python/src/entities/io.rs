use pyo3::{IntoPyObjectExt, prelude::*, types::PyList};
use pyo3_async_runtimes::tokio::future_into_py;

use super::{
    PyEntityModel,
    backend::{Active, Select, Terminal, Write, Written},
};
use crate::{
    errors::{ConstructionError, InternalError},
    execution::Executor,
    transactions::work::{Output, Work},
};

pub(crate) fn select<'py>(
    py: Python<'py>,
    connection: &Bound<'_, PyAny>,
    query: Select,
    terminal: Terminal,
) -> PyResult<Bound<'py, PyAny>> {
    if query.compile(terminal).1.0.len() > 65535 {
        return Err(ConstructionError::new_err(
            "PostgreSQL supports at most 65535 query parameters",
        ));
    }
    models(py, connection, Work::Entity(query, terminal), terminal)
}

/// The models `work`, a read of a registration's, gives on `connection`: a
/// list for `all`, and one or `None` otherwise.
pub(crate) fn models<'py>(
    py: Python<'py>,
    connection: &Bound<'_, PyAny>,
    work: Work,
    terminal: Terminal,
) -> PyResult<Bound<'py, PyAny>> {
    let target = Executor::extract(py, connection)?;
    future_into_py(py, async move {
        let Output::Models(models) = target.run(work).await? else {
            return Err(InternalError::new_err("unexpected entity result"));
        };
        Python::attach(|py| {
            let models: Vec<_> = models
                .into_iter()
                .map(|inner| PyEntityModel { inner })
                .collect();
            for model in &models {
                model.validate(py)?;
            }
            if matches!(terminal, Terminal::All) {
                Ok(PyList::new(py, models)?.into_any().unbind())
            } else {
                match models.into_iter().next() {
                    Some(model) => model.into_py_any(py),
                    None => Ok(py.None()),
                }
            }
        })
    })
}

pub(crate) fn write<'py>(
    py: Python<'py>,
    connection: &Bound<'_, PyAny>,
    active: Active,
    write: Write,
) -> PyResult<Bound<'py, PyAny>> {
    let target = Executor::extract(py, connection)?;
    future_into_py(py, async move {
        let Output::Written(result) = target.run(Work::Write(active, write)).await? else {
            return Err(InternalError::new_err("unexpected model write result"));
        };
        Python::attach(|py| match result {
            Written::Count(count) => count.into_py_any(py),
            Written::Model(inner) => {
                let model = PyEntityModel { inner };
                model.validate(py)?;
                model.into_py_any(py)
            }
        })
    })
}
