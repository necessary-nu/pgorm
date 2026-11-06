use std::{fmt, pin::Pin, sync::Arc};

use crate::panics::future_into_py;
use futures_util::TryStreamExt;
use pgorm::{ConnectionTrait, ValueHolder};
use pyo3::prelude::*;
use tokio::sync::Mutex;
use tokio_postgres::{RowStream, types::ToSql};
use tokio_util::sync::CancellationToken;

use super::PyRecord;
use crate::{
    errors::{LifecycleError, database_error},
    runtime::{ConnectionState, Operation},
    statements,
};

struct Active {
    rows: Pin<Box<RowStream>>,
    operation: Operation,
}

/// What a stream makes of each row: a `Record`, or what a read that decodes
/// its own rows, a disjoint one's, makes of it.
pub(crate) type RowDecode = Arc<dyn Fn(tokio_postgres::Row) -> PyResult<Py<PyAny>> + Send + Sync>;

struct StreamState {
    connection: Arc<ConnectionState>,
    decode: Option<RowDecode>,
    active: Arc<Mutex<Option<Active>>>,
    cancelled: CancellationToken,
    finished: CancellationToken,
}

impl StreamState {
    /// Also release idle streams when their connection or pool is closed.
    async fn watch(self: Arc<Self>) {
        tokio::select! {
            _ = self.finished.cancelled() => return,
            _ = self.cancelled.cancelled() => (),
            _ = self.connection.cancelled.cancelled() => (),
            _ = self.connection.pool.cancelled.cancelled() => (),
        }
        drop(self.active.lock().await.take());
        self.finished.cancel();
    }
}

// [spec:pgorm:req:bindings.streams]
// [spec:pgorm:req:bindings.cancellation]
#[pyclass(name = "_Stream", module = "pgorm._native", frozen)]
pub(crate) struct NativeStream {
    state: Arc<StreamState>,
}

impl fmt::Debug for NativeStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeStream")
            .field("closed", &self.closed())
            .finish()
    }
}

#[pymethods]
impl NativeStream {
    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.connection.pool.check_owner(py)?;
        let state = self.state.clone();
        future_into_py(py, async move {
            // Owner closure is an error even when its watcher has released the lease.
            state.connection.ensure_open()?;
            if state.finished.is_cancelled() {
                return Ok(None);
            }
            let mut slot = state.active.clone().try_lock_owned().map_err(|_| {
                LifecycleError::new_err("stream already has an active next operation")
            })?;
            let Some(mut active) = slot.take() else {
                return Ok(None);
            };
            let row = tokio::select! {
                _ = state.cancelled.cancelled() => return Err(LifecycleError::new_err("stream is closed")),
                _ = state.connection.cancelled.cancelled() => return Err(LifecycleError::new_err("connection is closed")),
                _ = state.connection.pool.cancelled.cancelled() => return Err(LifecycleError::new_err("pool is closed")),
                row = active.rows.try_next() => row,
            };
            match row {
                Ok(Some(row)) => {
                    let item = match &state.decode {
                        Some(decode) => decode(row)?,
                        None => {
                            let record = PyRecord::decode(&row)?;
                            Python::attach(|py| Py::new(py, record).map(Py::into_any))?
                        }
                    };
                    *slot = Some(active);
                    Ok(Some(item))
                }
                Ok(None) => {
                    active.operation.restore();
                    state.finished.cancel();
                    Ok(None)
                }
                Err(error) => Err(database_error(
                    pgorm::Error::from(error),
                    &state.connection.pool.secrets,
                )),
            }
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.connection.pool.check_owner(py)?;
        self.state.cancelled.cancel();
        let state = self.state.clone();
        future_into_py(py, async move {
            drop(state.active.lock().await.take());
            state.finished.cancel();
            Ok(())
        })
    }

    fn closed(&self) -> bool {
        self.state.cancelled.is_cancelled() || self.state.finished.is_cancelled()
    }
}

impl Drop for NativeStream {
    fn drop(&mut self) {
        self.state.cancelled.cancel();
        if let Ok(mut slot) = self.state.active.try_lock() {
            drop(slot.take());
        }
    }
}

pub(crate) fn open<'py>(
    py: Python<'py>,
    connection: Arc<ConnectionState>,
    query: &Bound<'_, PyAny>,
    decode: Option<RowDecode>,
) -> PyResult<Bound<'py, PyAny>> {
    connection.pool.check_owner(py)?;
    connection.ensure_open()?;
    let compiled = statements::compile(query)?;
    future_into_py(py, async move {
        let operation = Operation::begin(connection.clone())?;
        let values: Vec<_> = compiled.values.0.into_iter().map(ValueHolder).collect();
        let db = operation.connection()?;
        let rows = tokio::select! {
            _ = connection.cancelled.cancelled() => return Err(LifecycleError::new_err("connection is closed")),
            _ = connection.pool.cancelled.cancelled() => return Err(LifecycleError::new_err("pool is closed")),
            rows = db.query_raw(&compiled.sql, values.iter().map(|v| v as &(dyn ToSql + Sync))) => rows,
        }.map_err(|error| database_error(error, &connection.pool.secrets))?;
        let state = Arc::new(StreamState {
            connection,
            decode,
            active: Arc::new(Mutex::new(Some(Active {
                rows: Box::pin(rows),
                operation,
            }))),
            cancelled: CancellationToken::new(),
            finished: CancellationToken::new(),
        });
        tokio::spawn(state.clone().watch());
        Ok(NativeStream { state })
    })
}
