//! A transaction pgorm-migration's runner holds, lent to the language for
//! one migration's `up`: the runner's closure owns the `DatabaseTransaction`,
//! and a frame serves the language's commands on it, savepoints and COPY
//! included, until `up` returns.

use std::{future::Future, sync::Arc};

use pgorm::DatabaseTransaction;
use tokio::sync::oneshot;

use super::{State, Tx, frame::Frame};
use crate::connection::{ConnectionState, PoolState};

/// How a lent transaction's work ended.
#[derive(Debug)]
pub enum Lent<T> {
    /// The work finished, as it answered; the transaction is as its last
    /// command left it.
    Ran(T),
    /// A command's outcome was lost, so nobody knows the transaction's state.
    Discarded,
}

/// Serve `tx` to the frame `work` is given, for as long as the future it
/// returns runs. The frame closes once that future finishes, so a command
/// the language sends afterwards is refused rather than run in a transaction
/// the runner has moved on from.
// [spec:pgorm:req:bindings.migrations]    a migration's `up` runs in the batch's transaction
pub async fn lend<O, F>(
    pool: Arc<PoolState<O>>,
    tx: &mut DatabaseTransaction<'_>,
    work: impl FnOnce(Arc<State<O>>) -> F,
) -> Lent<F::Output>
where
    O: Send + Sync + 'static,
    F: Future,
{
    let (state, receiver) =
        State::rooted(ConnectionState::unheld(pool), None, Arc::default(), true);
    // Held until the frame has answered it, as an opening transaction's
    // caller holds its own.
    let (opened, _opening) = oneshot::channel();
    let mut frame = Frame::new(state.clone(), receiver, opened);
    let running = work(state.clone());
    let halt = frame.halt.clone();
    let served = halt.run(frame.serve(Tx::Plain(tx)));
    tokio::pin!(running, served);
    let finished = tokio::select! {
        biased;
        finished = &mut running => Some(finished),
        served = &mut served => match served {
            // The handle was dropped while the work still runs, and can send
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
