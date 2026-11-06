//! A transaction pgorm-migration's runner holds, lent to Python for one
//! migration's `up`: the runner's closure owns the `DatabaseTransaction`, and
//! a frame serves Python's commands on it, savepoints and COPY included, until
//! `up` returns.

use super::{
    NativeTransaction,
    scope::{Frame, Tx},
    state::State,
};
use crate::runtime::{ConnectionState, PoolState};
use pgorm::DatabaseTransaction;
use pyo3::prelude::*;
use std::{future::Future, sync::Arc};
use tokio::sync::{Mutex, oneshot};
use tokio_util::sync::CancellationToken;

/// How a lent transaction's work ended.
pub(crate) enum Lent<T> {
    /// The work finished, as it answered; the transaction is as its last
    /// command left it.
    Ran(PyResult<T>),
    /// A command's outcome was lost, so nobody knows the transaction's state.
    Discarded,
}

/// Serve `tx` to the handle `work` is given, for as long as the future it
/// returns runs. The handle closes once that future finishes, so a command
/// Python sends afterwards is refused rather than run in a transaction the
/// runner has moved on from.
// [spec:pgorm:req:bindings.migrations]    a migration's `up` runs in the batch's transaction
pub(crate) async fn lend<F, T>(
    pool: Arc<PoolState>,
    tx: &mut DatabaseTransaction<'_>,
    work: impl FnOnce(NativeTransaction) -> PyResult<F>,
) -> Lent<T>
where
    F: Future<Output = PyResult<T>>,
{
    let connection = Arc::new(ConnectionState {
        pool,
        cancelled: CancellationToken::new(),
        slot: Arc::new(Mutex::new(None)),
    });
    let (state, receiver) = State::lent(connection);
    // Held until the frame has answered it, as an opening transaction's
    // caller holds its own.
    let (opened, _opening) = oneshot::channel();
    let mut frame = Frame::new(state.clone(), receiver, opened);
    let running = match work(NativeTransaction {
        state: state.clone(),
    }) {
        Ok(running) => running,
        Err(error) => return Lent::Ran(Err(error)),
    };
    let halt = frame.halt.clone();
    let served = halt.run(frame.serve(Tx::Plain(tx)));
    tokio::pin!(running, served);
    let finished = tokio::select! {
        biased;
        finished = &mut running => Some(finished),
        served = &mut served => match served {
            // The handle was dropped while `up` still runs, and can send
            // nothing more.
            Some(_) => None,
            None => return Lent::Discarded,
        },
    };
    let finished = match finished {
        Some(finished) => finished,
        None => running.await,
    };
    state.finished.cancel();
    Lent::Ran(finished)
}
