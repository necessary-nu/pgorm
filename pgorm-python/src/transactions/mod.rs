//! Transaction handles over pgorm-bindings' frames: a task on the runtime
//! serves each borrowed transaction and savepoint inside the closure pgorm
//! opens it around, and Python's handles send it owned work.
mod capabilities;
mod copy;
mod native;
pub(crate) mod work;

use crate::{
    errors::{Raise, raised},
    runtime::{ConnectionState, Loop, Operation, Owned},
};
pub(crate) use capabilities::operations as capabilities;
pub(crate) use copy::{copy_in, copy_out, describe_in};
pub(crate) use native::{NativeRetrying, NativeTransaction, mode};
use pgorm::TransactionMode;
pub(crate) use pgorm_bindings::transaction::Lease;
use pgorm_bindings::transaction::Retrying;
use pyo3::prelude::*;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use work::{Output, Work};

/// One transaction or savepoint, as its Python handle reaches it.
pub(crate) type State = pgorm_bindings::transaction::State<Loop>;

impl Owned for State {
    /// Refuse from another event loop, and once the frame is closed.
    fn check_owner(&self, py: Python<'_>) -> PyResult<()> {
        self.connection().pool.check_owner(py)?;
        self.ensure_open().py()
    }
}

// [spec:pgorm:req:bindings.transactions+2]
pub(crate) async fn begin(
    connection: Arc<ConnectionState>,
    mode: TransactionMode,
) -> PyResult<Arc<State>> {
    let operation = Operation::begin(connection).py()?;
    pgorm_bindings::transaction::begin(operation, mode, &CancellationToken::new())
        .await
        .py()
}

// [spec:pgorm:req:bindings.transaction-retry]
pub(crate) fn begin_retrying(
    connection: Arc<ConnectionState>,
    mode: TransactionMode,
    retries: u32,
) -> PyResult<Arc<Retrying<Loop>>> {
    let operation = Operation::begin(connection).py()?;
    Ok(Retrying::begin(operation, mode, retries))
}

/// Run `work` in the frame, its lock taken for it.
pub(crate) async fn run(state: &Arc<State>, work: Work) -> PyResult<Output> {
    let lock = state.lock().py()?;
    let pool = state.connection().pool.clone();
    state
        .request(
            lock,
            move |db| {
                Box::pin(async move { work.run(db).await.map_err(|error| pool.failure(&error)) })
            },
            &CancellationToken::new(),
        )
        .await
        .map_err(raised)
}
