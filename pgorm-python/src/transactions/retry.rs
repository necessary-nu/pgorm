//! A transaction run again from `BEGIN` while it fails with a serialization
//! failure or a deadlock: pgorm's `transaction_with_retry`, whose closure is
//! called once per attempt, each attempt a frame its Python block is served
//! from.
//!
//! Each attempt's block ends with a commit or a rollback request, whose reply
//! waits on pgorm: `Retrying` once pgorm has rolled the attempt back, or seen
//! its `COMMIT` fail retryably, and opened the next; otherwise the outcome of
//! the whole transaction.

use super::{
    State,
    scope::{Exit, Failure, Frame, Halt, Reply, Tx},
    state::{Finished, Root},
    work::Output,
};
use crate::{
    errors::{LifecycleError, database_error},
    runtime::{ConnectionState, Held, Operation},
};
use pgorm::{RetryableError, TransactionError, TransactionMode};
use pyo3::prelude::*;
use std::{fmt, sync::Arc};
use tokio::sync::{Mutex, mpsc, oneshot};

/// The closure's error: whether the block's failure is one to retry.
struct Again(bool);

impl RetryableError for Again {
    fn is_retryable(&self) -> bool {
        self.0
    }
}

/// What a retrying transaction tells Python about its next attempt.
enum Attempt {
    /// An attempt opened, its block to run on this frame.
    Open(Arc<State>),
    /// The first `BEGIN` was refused.
    Refused(PyErr),
}

/// The Python handle's half of a retrying transaction.
pub(crate) struct Retrying {
    attempts: Mutex<mpsc::UnboundedReceiver<Attempt>>,
    root: Arc<Root>,
    connection: Arc<ConnectionState>,
}

impl fmt::Debug for Retrying {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Retrying").finish_non_exhaustive()
    }
}

impl Retrying {
    /// The next attempt's frame, once pgorm has opened it.
    pub(crate) async fn attempt(&self) -> PyResult<Arc<State>> {
        let mut attempts = self.attempts.try_lock().map_err(|_| {
            LifecycleError::new_err("a retrying transaction opens one attempt at a time")
        })?;
        match attempts.recv().await {
            Some(Attempt::Open(state)) => Ok(state),
            Some(Attempt::Refused(error)) => Err(error),
            None => Err(LifecycleError::new_err("transaction is closed")),
        }
    }

    /// End the whole transaction, discarding its connection.
    pub(crate) fn abort(&self) {
        self.root.aborted.cancel();
    }

    pub(crate) fn connection(&self) -> &Arc<ConnectionState> {
        &self.connection
    }
}

/// The reply the last attempt's block is waiting on, whether it asked to
/// commit, and the attempt it ended.
struct Last {
    reply: Reply,
    committed: bool,
    state: Arc<State>,
}

/// What every attempt of one retrying transaction shares.
struct Attempts {
    sender: mpsc::UnboundedSender<Attempt>,
    halt: Halt,
    connection: Arc<ConnectionState>,
    root: Arc<Root>,
    last: Option<Last>,
}

impl Attempts {
    /// One call of pgorm's closure: answer the attempt before it, which pgorm
    /// has ended, with `Retrying`, and serve this attempt's block.
    async fn serve(&mut self, tx: Tx<'_, '_>) -> Result<(), Again> {
        if let Some(previous) = self.last.take() {
            previous.state.released.cancel();
            if previous.reply.send(Ok(Output::Retrying)).is_err() {
                return Err(Again(false));
            }
        }
        let (state, receiver) = State::attempt(self.connection.clone(), self.root.clone());
        let (opened, _open) = oneshot::channel();
        if self.sender.send(Attempt::Open(state.clone())).is_err() {
            return Err(Again(false));
        }
        let mut frame = Frame::halted_by(state.clone(), receiver, opened, self.halt.clone());
        match frame.serve(tx).await {
            Ok(()) => {
                self.last = frame.take_committed().map(|reply| Last {
                    reply,
                    committed: true,
                    state,
                });
                Ok(())
            }
            Err(Exit::RolledBack(reply, retryable)) => {
                self.last = Some(Last {
                    reply,
                    committed: false,
                    state,
                });
                Err(Again(retryable))
            }
            Err(Exit::Abandoned) => Err(Again(false)),
        }
    }
}

/// Open a transaction run again from `BEGIN` up to `retries` times while it
/// fails retryably, through pgorm's `transaction_with_retry`.
///
/// What the attempts share is held behind a lock the closure owns a handle
/// to, rather than borrowed by it: a closure pgorm calls once per attempt
/// that borrows from the task around it makes that task's future `Send` for
/// no lifetime the compiler can name, and `tokio::spawn` refuses it.
// [spec:pgorm:req:bindings.transaction-retry]
pub(crate) fn begin(
    connection: Arc<ConnectionState>,
    mode: TransactionMode,
    retries: u32,
) -> PyResult<Arc<Retrying>> {
    let mut operation = Operation::begin(connection.clone())?;
    let root = Root::new();
    let (sender, receiver) = mpsc::unbounded_channel();
    let shared = Arc::new(Mutex::new(Attempts {
        sender: sender.clone(),
        halt: Halt::default(),
        connection: connection.clone(),
        root: root.clone(),
        last: None,
    }));
    let (owner, served) = (root.clone(), connection.clone());
    tokio::spawn(async move {
        let _finished = Finished(owner.finished.clone());
        let halt = shared.lock().await.halt.clone();
        let attempts = shared.clone();
        let ended = tokio::select! {
            _ = owner.aborted.cancelled() => None,
            _ = served.cancelled.cancelled() => None,
            _ = served.pool.cancelled.cancelled() => None,
            ended = async {
                let db = match operation.connection_mut() {
                    Ok(db) => db,
                    Err(error) => {
                        let _ = sender.send(Attempt::Refused(error));
                        return None;
                    }
                };
                match db {
                    Held::Plain(db) => {
                        halt.run(db.transaction_with_retry(mode, retries, async move |tx| {
                            attempts.lock().await.serve(Tx::Plain(tx)).await
                        }))
                        .await
                    }
                    Held::Metered(db) => {
                        halt.run(db.transaction_with_retry(mode, retries, async move |tx| {
                            attempts.lock().await.serve(Tx::Metered(tx)).await
                        }))
                        .await
                    }
                }
            } => ended,
        };
        let last = shared.lock().await.last.take();
        let (healthy, answer) = match ended {
            None => (false, None),
            Some(Ok(())) | Some(Err(TransactionError::Transaction(_))) => {
                (true, Some(Ok(Output::Finished)))
            }
            // A COMMIT the aborted transaction answered by rolling back leaves
            // the connection in a known state.
            // [spec:pgorm:req:bindings.transactions+2]
            Some(Err(TransactionError::Connection(
                error @ pgorm::Error::TransactionAborted(_),
            ))) => (true, Some(Err(Failure::Pgorm(error)))),
            Some(Err(TransactionError::Connection(error))) => match &last {
                // A COMMIT that failed leaves the connection in a state
                // nobody knows; a BEGIN that did, after a rolled-back
                // attempt, leaves it as it was.
                Some(Last { committed, .. }) => (!committed, Some(Err(Failure::Pgorm(error)))),
                None => {
                    let refused = database_error(error, &served.pool.secrets);
                    let _ = sender.send(Attempt::Refused(refused));
                    (true, None)
                }
            },
            Some(Err(TransactionError::RollbackFailed { rollback, .. })) => {
                (false, Some(Err(Failure::Pgorm(rollback))))
            }
            Some(Err(_)) => (false, Some(Err(Failure::Unrepresented))),
        };
        if healthy {
            operation.restore();
        } else {
            drop(operation);
        }
        if let Some(last) = last {
            if let Some(answer) = answer {
                let _ = last.reply.send(answer);
            }
            last.state.released.cancel();
        }
    });
    Ok(Arc::new(Retrying {
        attempts: Mutex::new(receiver),
        root,
        connection,
    }))
}
