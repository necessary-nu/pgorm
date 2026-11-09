use std::{fmt, sync::Arc};

use crate::panics::future_into_py;
use pgorm::ValueHolder;
use pyo3::prelude::*;
use tokio_util::sync::CancellationToken;

use super::PyRecord;
use crate::{
    errors::Raise,
    runtime::{ConnectionState, Loop, Operation, Owned},
    statements,
    transactions::State,
};

/// What a stream makes of each row: a `Record`, or what a read that decodes
/// its own rows, a disjoint one's, makes of it.
pub(crate) type RowDecode = Arc<dyn Fn(tokio_postgres::Row) -> PyResult<Py<PyAny>> + Send + Sync>;

// [spec:pgorm:req:bindings.streams+1]
// [spec:pgorm:req:bindings.cancellation]
#[pyclass(name = "_Stream", module = "pgorm._native", frozen)]
pub(crate) struct NativeStream {
    state: Arc<pgorm_bindings::StreamState<Loop>>,
    decode: Option<RowDecode>,
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
        self.state.connection().pool.check_owner(py)?;
        let (state, decode) = (self.state.clone(), self.decode.clone());
        future_into_py(py, async move {
            let row = state
                .next(&CancellationToken::new(), |row| match &decode {
                    Some(decode) => decode(row),
                    None => {
                        let record = PyRecord::decode(&row)?;
                        Python::attach(|py| Py::new(py, record).map(Py::into_any))
                    }
                })
                .await
                .py()?;
            row.transpose()
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.connection().pool.check_owner(py)?;
        let state = self.state.clone();
        future_into_py(py, async move {
            state.close().await;
            Ok(())
        })
    }

    fn closed(&self) -> bool {
        self.state.closed()
    }
}

impl Drop for NativeStream {
    fn drop(&mut self) {
        self.state.close_now();
    }
}

/// What decodes a stream's rows in place of a `Record`: a disjoint read's
/// codec or a registered read's selector, as `decoder` names one.
pub(crate) fn row_decoder(decoder: Option<&Bound<'_, PyAny>>) -> PyResult<Option<RowDecode>> {
    let Some(decoder) = decoder.filter(|decoder| !decoder.is_none()) else {
        return Ok(None);
    };
    if let Ok(disjoint) = decoder.extract::<PyRef<'_, crate::disjoint::NativeDisjoint>>() {
        return Ok(Some(disjoint.row_decode()));
    }
    Ok(Some(
        decoder
            .extract::<PyRef<'_, crate::entities::reads::NativeRowDecoder>>()?
            .row_decode(),
    ))
}

/// What a stream reads from.
pub(crate) enum Source {
    Connection(Arc<ConnectionState>),
    Transaction(Arc<State>),
}

impl Source {
    fn connection(&self) -> &Arc<ConnectionState> {
        match self {
            Self::Connection(connection) => connection,
            Self::Transaction(state) => state.connection(),
        }
    }
}

// [spec:pgorm:req:bindings.streams+1]
pub(crate) fn open<'py>(
    py: Python<'py>,
    source: Source,
    query: &Bound<'_, PyAny>,
    decode: Option<RowDecode>,
) -> PyResult<Bound<'py, PyAny>> {
    let connection = source.connection().clone();
    connection.pool.check_owner(py)?;
    match &source {
        Source::Connection(connection) => connection.ensure_open().py()?,
        Source::Transaction(state) => state.check_owner(py)?,
    }
    let compiled = statements::compile(query)?;
    future_into_py(py, async move {
        let values: Vec<_> = compiled.values.0.into_iter().map(ValueHolder).collect();
        let never = CancellationToken::new();
        let state = match source {
            Source::Connection(connection) => {
                let operation = Operation::begin(connection).py()?;
                pgorm_bindings::StreamState::on_connection(operation, compiled.sql, values, &never)
                    .await
            }
            Source::Transaction(state) => {
                let lock = state.lock().py()?;
                pgorm_bindings::StreamState::in_transaction(
                    &state,
                    lock,
                    compiled.sql,
                    values,
                    &never,
                )
                .await
            }
        }
        .py()?;
        Ok(NativeStream { state, decode })
    })
}
