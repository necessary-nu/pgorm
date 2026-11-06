use crate::panics::future_into_py;
use pyo3::{IntoPyObjectExt, prelude::*};

use pgorm_bindings::entity::{Active, Read as Terminal, Select, Write, Written};

use super::PyEntityModel;
use crate::{
    errors::{ConstructionError, InternalError},
    execution::{Executor, listed},
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
    listed(
        py,
        target,
        work,
        !matches!(terminal, Terminal::All),
        "entity",
        |output| match output {
            Output::Models(models) => Some(models),
            _ => None,
        },
        |py, inner| {
            let model = PyEntityModel { inner };
            model.validate(py)?;
            model.into_py_any(py)
        },
    )
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
