use super::{State, work::Output};
use crate::{
    errors::{ConstructionError, InternalError},
    execution::{Executor, Target},
    results,
};
use pgorm::{IsolationLevel, TransactionMode};
use pyo3::{exceptions::PyTypeError, prelude::*};
use pyo3_async_runtimes::tokio::future_into_py;
use std::{fmt, sync::Arc};

pub(crate) fn mode(mode: &str, isolation: Option<&str>) -> PyResult<TransactionMode> {
    let isolation = isolation
        .map(|level| match level {
            "read_uncommitted" => Ok(IsolationLevel::ReadUncommitted),
            "read_committed" => Ok(IsolationLevel::ReadCommitted),
            "repeatable_read" => Ok(IsolationLevel::RepeatableRead),
            "serializable" => Ok(IsolationLevel::Serializable),
            _ => Err(PyTypeError::new_err(
                "unsupported transaction isolation level",
            )),
        })
        .transpose()?;
    match (mode, isolation) {
        ("default", None) => Ok(TransactionMode::Default),
        ("read_write", isolation) => Ok(TransactionMode::ReadWrite { isolation }),
        ("read_only", isolation) => Ok(TransactionMode::ReadOnly { isolation }),
        ("deferrable", None) => Ok(TransactionMode::DeferrableSnapshot),
        ("default" | "deferrable", Some(_)) => Err(ConstructionError::new_err(
            "default and deferrable modes take no isolation override; choose read_write or read_only",
        )),
        _ => Err(PyTypeError::new_err(
            "a transaction's mode is default, read_write, read_only or deferrable",
        )),
    }
}

#[pyclass(name = "_Transaction", module = "pgorm._native", frozen)]
pub(crate) struct NativeTransaction {
    pub(crate) state: Arc<State>,
}

impl fmt::Debug for NativeTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeTransaction")
            .field("closed", &self.state.closed())
            .finish()
    }
}

#[pymethods]
impl NativeTransaction {
    fn abort<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.connection.pool.check_owner(py)?;
        self.state.root.aborted.cancel();
        let state = self.state.clone();
        future_into_py(py, async move {
            state.root.finished.cancelled().await;
            Ok(())
        })
    }

    fn owner_matches(&self, parent: &Bound<'_, PyAny>) -> bool {
        if let Some(expected) = &self.state.parent {
            let Ok(parent) = parent.extract::<PyRef<'_, NativeTransaction>>() else {
                return false;
            };
            expected
                .upgrade()
                .is_some_and(|expected| Arc::ptr_eq(&expected, &parent.state))
        } else {
            let Ok(parent) = parent.extract::<PyRef<'_, crate::runtime::NativeConnection>>() else {
                return false;
            };
            Arc::ptr_eq(&self.state.connection, &parent.state)
        }
    }

    fn wait_closed<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.connection.pool.check_owner(py)?;
        let state = self.state.clone();
        future_into_py(py, async move {
            tokio::select! {
                _ = state.released.cancelled() => (),
                _ = state.root.finished.cancelled() => (),
            }
            Ok(())
        })
    }

    fn begin<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.check(py)?;
        let state = self.state.clone();
        future_into_py(py, async move {
            Ok(Self {
                state: state.begin().await?,
            })
        })
    }

    fn finish<'py>(&self, py: Python<'py>, commit: bool) -> PyResult<Bound<'py, PyAny>> {
        self.state.check(py)?;
        let state = self.state.clone();
        future_into_py(py, async move {
            match state.request(None, commit).await? {
                Output::Finished => Ok(()),
                _ => Err(InternalError::new_err("unexpected transaction completion")),
            }
        })
    }

    fn closed(&self, py: Python<'_>) -> PyResult<bool> {
        self.state.connection.pool.check_owner(py)?;
        Ok(self.state.closed())
    }

    fn execute<'py>(
        &self,
        py: Python<'py>,
        query: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        results::execute(py, Target::Transaction(self.state.clone()), query)
    }

    fn fetch<'py>(
        &self,
        py: Python<'py>,
        query: &Bound<'_, PyAny>,
        mode: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        results::fetch(
            py,
            Executor::Held(Target::Transaction(self.state.clone())),
            query,
            mode,
        )
    }

    fn notify<'py>(
        &self,
        py: Python<'py>,
        channel: &Bound<'_, PyAny>,
        payload: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        crate::listen::notify(
            py,
            Target::Transaction(self.state.clone()),
            channel,
            payload,
        )
    }
}

impl Drop for NativeTransaction {
    fn drop(&mut self) {
        self.state.abandoned.cancel();
    }
}
