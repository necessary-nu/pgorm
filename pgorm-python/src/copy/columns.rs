//! A binary COPY's columns: `CopyColumn`, pgorm's own under a Python class,
//! and the description of a COPY's columns before it starts.

use pyo3::{exceptions::PyTypeError, prelude::*};
use pyo3_async_runtimes::tokio::future_into_py;

use super::{Described, PyCopyFrom, PyCopyTo, describe};
use crate::{
    config::DIALECT,
    errors::{ConstructionError, LifecycleError, database_error},
    execution::Target,
    runtime::{Held, Operation},
    values::PyTypeName,
};
/// One column of a binary COPY: its name, its type and, for `money`, the
/// places its currency is written to. A described column carries whatever
/// type the server gave it; one a program builds names a built-in type.
// [spec:pgorm:req:bindings.copy-rows+2]
#[derive(Debug, Clone)]
#[pyclass(name = "CopyColumn", module = "pgorm", frozen, eq, from_py_object)]
pub(crate) struct PyCopyColumn {
    pub(crate) inner: pgorm::CopyColumn,
}

impl PartialEq for PyCopyColumn {
    fn eq(&self, other: &Self) -> bool {
        self.inner.name() == other.inner.name()
            && self.inner.type_() == other.inner.type_()
            && self.inner.money_places() == other.inner.money_places()
    }
}

#[pymethods]
impl PyCopyColumn {
    #[new]
    #[pyo3(signature = (name, r#type, *, money_places=None))]
    fn new(name: String, r#type: &Bound<'_, PyAny>, money_places: Option<u32>) -> PyResult<Self> {
        let type_name = if let Ok(text) = r#type.extract::<String>() {
            text
        } else if let Ok(named) = r#type.extract::<PyRef<'_, PyTypeName>>() {
            if named
                .schema
                .as_deref()
                .is_some_and(|schema| schema != "pg_catalog")
            {
                return Err(ConstructionError::new_err(format!(
                    "{} is no built-in PostgreSQL type; describe the COPY's columns to learn \
                     another type",
                    named.name
                )));
            }
            named.name.clone()
        } else {
            return Err(PyTypeError::new_err(
                "a COPY column's type is a built-in type's name or a TypeName",
            ));
        };
        pgorm_bindings::copy_column(name, &type_name, money_places, DIALECT)
            .map(|inner| Self { inner })
            .map_err(|refused| ConstructionError::new_err(refused.into_message()))
    }

    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// The column's type, schema-qualified as the server names it.
    #[getter]
    fn r#type(&self) -> PyTypeName {
        PyTypeName {
            name: self.inner.type_().name().to_owned(),
            schema: Some(self.inner.type_().schema().to_owned()),
        }
    }

    #[getter]
    fn money_places(&self) -> Option<u32> {
        self.inner.money_places()
    }

    fn __repr__(&self) -> String {
        format!(
            "CopyColumn({:?}, {:?})",
            self.inner.name(),
            self.inner.type_().name()
        )
    }
}

/// The description on a connection, plain or an instrumented pool's.
async fn describe_held(
    held: &mut Held,
    statement: &Described,
) -> Result<Vec<pgorm::CopyColumn>, pgorm::Error> {
    match held {
        Held::Plain(connection) => describe(connection, statement).await,
        Held::Metered(connection) => describe(connection, statement).await,
    }
}

/// `copy_in_columns` and, with `out`, `copy_out_columns`: the columns
/// `statement` writes or reads, described without starting the COPY, on a
/// connection or a transaction. A description the server refuses is its
/// database error, and leaves a connection usable and a transaction aborted
/// as a failed statement does.
// [spec:pgorm:req:bindings.copy-rows+2]
#[pyfunction]
#[pyo3(name = "_copy_columns")]
pub(crate) fn copy_columns<'py>(
    py: Python<'py>,
    target: &Bound<'_, PyAny>,
    statement: &Bound<'_, PyAny>,
    out: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let target = Target::extract(py, target)?;
    let statement = if out {
        Described::To(
            statement
                .extract::<PyRef<'_, PyCopyTo>>()
                .map_err(|_| PyTypeError::new_err("copy_out_columns describes a CopyTo statement"))?
                .inner
                .clone(),
        )
    } else {
        Described::From(
            statement
                .extract::<PyRef<'_, PyCopyFrom>>()
                .map_err(|_| {
                    PyTypeError::new_err("copy_in_columns describes a CopyFrom statement")
                })?
                .inner
                .clone(),
        )
    };
    future_into_py(py, async move {
        let columns = match target {
            Target::Connection(state) => {
                let mut operation = Operation::begin(state.clone())?;
                let described = tokio::select! {
                    _ = state.cancelled.cancelled() => return Err(LifecycleError::new_err("connection is closed")),
                    _ = state.pool.cancelled.cancelled() => return Err(LifecycleError::new_err("pool is closed")),
                    described = describe_held(operation.connection_mut()?, &statement) => described,
                };
                operation.restore();
                described.map_err(|error| database_error(error, &state.pool.secrets))?
            }
            Target::Transaction(state) => state.describe(statement).await?,
            Target::Hook(_) => {
                return Err(PyTypeError::new_err(
                    "a COPY runs on a Connection or a Transaction",
                ));
            }
        };
        Ok(columns
            .into_iter()
            .map(|inner| PyCopyColumn { inner })
            .collect::<Vec<_>>())
    })
}
