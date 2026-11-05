use crate::{
    errors::{LifecycleError, database_error},
    metrics::Metrics,
    runtime::{ConnectionState, NativeConnection, NativePool, Operation, PoolState},
    transactions::{
        NativeTransaction, State,
        work::{Output, Work},
    },
};
use pgorm::metric::InstrumentedPool;
use pyo3::{exceptions::PyTypeError, prelude::*};
use std::sync::Arc;

pub(crate) enum Target {
    Connection(Arc<ConnectionState>),
    Transaction(Arc<State>),
}

/// The native handle behind a Python `Pool`, `Connection` or `Transaction`.
fn native<'py>(value: &Bound<'py, PyAny>) -> Bound<'py, PyAny> {
    value.getattr("_native").unwrap_or_else(|_| value.clone())
}

impl Target {
    pub(crate) fn extract(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Self::held(py, &native(value))?.ok_or_else(|| {
            PyTypeError::new_err("execution requires an acquired pgorm Connection or Transaction")
        })
    }

    /// The connection or transaction `native` is, checked usable from here.
    fn held(py: Python<'_>, native: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        let target = if let Ok(connection) = native.extract::<PyRef<'_, NativeConnection>>() {
            Self::Connection(connection.state.clone())
        } else if let Ok(transaction) = native.extract::<PyRef<'_, NativeTransaction>>() {
            Self::Transaction(transaction.state.clone())
        } else {
            return Ok(None);
        };
        target.check(py)?;
        Ok(Some(target))
    }

    pub(crate) fn check(&self, py: Python<'_>) -> PyResult<()> {
        match self {
            Self::Connection(state) => {
                state.pool.check_owner(py)?;
                state.ensure_open()
            }
            Self::Transaction(state) => state.check(py),
        }
    }

    pub(crate) async fn run(self, work: Work) -> PyResult<Output> {
        match self {
            Self::Transaction(state) => state.run(work).await,
            Self::Connection(state) => {
                let operation = Operation::begin(state.clone())?;
                let result = tokio::select! {
                    _ = state.cancelled.cancelled() => return Err(LifecycleError::new_err("connection is closed")),
                    _ = state.pool.cancelled.cancelled() => return Err(LifecycleError::new_err("pool is closed")),
                    result = work.run(operation.connection()?) => result,
                };
                operation.restore();
                result.map_err(|error| database_error(error, &state.pool.secrets))
            }
        }
    }
}

/// Where an operation on a registration runs: on a connection a pool lends it
/// alone, or on the connection or transaction its caller holds.
pub(crate) enum Executor {
    /// A pool, and its pgorm wrapper when it is instrumented.
    Pool(Arc<PoolState>, Option<InstrumentedPool<Metrics>>),
    Held(Target),
}

impl Executor {
    // [spec:pgorm:req:bindings.registered-pools]
    pub(crate) fn extract(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let native = native(value);
        if let Ok(pool) = native.extract::<PyRef<'_, NativePool>>() {
            let executor = Self::Pool(pool.state.clone(), pool.metered.clone());
            executor.check(py)?;
            return Ok(executor);
        }
        Target::held(py, &native)?.map(Self::Held).ok_or_else(|| {
            PyTypeError::new_err(
                "a registration or a pipeline runs on a Pool, a Connection or a Transaction",
            )
        })
    }

    /// Whether what this runs on is still usable from here.
    pub(crate) fn check(&self, py: Python<'_>) -> PyResult<()> {
        match self {
            Self::Pool(pool, _) => {
                pool.check_owner(py)?;
                pool.ensure_open()
            }
            Self::Held(target) => target.check(py),
        }
    }

    /// Run `work`. A pool's connection goes back to it once the work has
    /// finished, and is discarded if the work is cut off, as a statement's is.
    pub(crate) async fn run(self, work: Work) -> PyResult<Output> {
        match self {
            Self::Held(target) => target.run(work).await,
            Self::Pool(pool, metered) => {
                Target::Connection(pool.lend(metered).await?)
                    .run(work)
                    .await
            }
        }
    }
}
