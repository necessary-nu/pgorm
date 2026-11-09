//! A transaction run again from `BEGIN` while it fails with a serialization
//! failure or a deadlock: pgorm's `transaction_with_retry`, whose closure is
//! called once per attempt, each attempt a frame the language's block runs
//! on.
//!
//! Each attempt's block ends with a commit or a rollback request, whose reply
//! waits on pgorm: `true` once pgorm has rolled the attempt back, or seen its
//! `COMMIT` fail retryably, and opened the next; otherwise the outcome of the
//! whole transaction.
//!
//! What the attempts share is held behind a lock the closure owns a handle
//! to, rather than borrowed by it: a closure pgorm calls once per attempt that
//! borrows from the task around it makes that task's future `Send` for no
//! lifetime the compiler can name, and `tokio::spawn` refuses it.

use std::{fmt, sync::Arc};

use pgorm::{RetryableError, TransactionError, TransactionMode};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::{
    Attempted, Finished, Root, State, Tx,
    frame::{Exit, Frame, Halt, UNREPRESENTED},
};
use crate::{
    Failure,
    connection::{ConnectionState, Held, Operation},
};

/// The closure's error: whether the block's failure is one to retry.
struct Again(bool);

impl RetryableError for Again {
    fn is_retryable(&self) -> bool {
        self.0
    }
}

/// The reply the last attempt's block is waiting on, whether it asked to
/// commit, and the attempt it ended.
struct Last<O> {
    reply: Attempted,
    committed: bool,
    state: Arc<State<O>>,
}

/// What every attempt of one retrying transaction shares.
struct Attempts<O> {
    sender: mpsc::UnboundedSender<Result<Arc<State<O>>, Failure>>,
    halt: Halt,
    connection: Arc<ConnectionState<O>>,
    root: Arc<Root>,
    last: Option<Last<O>>,
}

impl<O: Send + Sync + 'static> Attempts<O> {
    /// One call of pgorm's closure: answer the attempt before it, which pgorm
    /// has ended, with `true`, and serve this attempt's frame.
    async fn serve(&mut self, tx: Tx<'_, '_>) -> Result<(), Again> {
        if let Some(previous) = self.last.take() {
            previous.state.released.cancel();
            if previous.reply.send(Ok(true)).is_err() {
                return Err(Again(false));
            }
        }
        let (state, receiver) =
            State::rooted(self.connection.clone(), None, self.root.clone(), false);
        let (opened, _open) = oneshot::channel();
        if self.sender.send(Ok(state.clone())).is_err() {
            return Err(Again(false));
        }
        let mut frame = Frame::halted_by(state.clone(), receiver, opened, self.halt.clone());
        match frame.serve(tx).await {
            Ok(()) => {
                self.last = frame.committed.take().map(|reply| Last {
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

/// A retrying transaction as the language's handle holds it: the attempts
/// pgorm opens, in turn.
pub struct Retrying<O = ()> {
    attempts: Mutex<mpsc::UnboundedReceiver<Result<Arc<State<O>>, Failure>>>,
    root: Arc<Root>,
    connection: Arc<ConnectionState<O>>,
}

impl<O> fmt::Debug for Retrying<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Retrying").finish_non_exhaustive()
    }
}

impl<O: Send + Sync + 'static> Retrying<O> {
    /// Open a transaction run again from `BEGIN` up to `retries` times while
    /// it fails retryably, through pgorm's `transaction_with_retry`, on the
    /// connection `operation` took.
    // [spec:pgorm:req:bindings.transaction-retry]
    pub fn begin(mut operation: Operation<O>, mode: TransactionMode, retries: u32) -> Arc<Self> {
        let connection = operation.state.clone();
        let root = Arc::new(Root::default());
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
                () = owner.aborted.cancelled() => None,
                () = served.closing() => None,
                ended = async {
                    let db = match operation.connection_mut() {
                        Ok(db) => db,
                        Err(error) => {
                            let _ = sender.send(Err(error));
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
            let failure = |error: &pgorm::Error| served.pool.failure(error);
            let (healthy, answer) = match ended {
                None => (false, None),
                Some(Ok(()) | Err(TransactionError::Transaction(_))) => (true, Some(Ok(false))),
                // A COMMIT the aborted transaction answered by rolling back
                // leaves the connection in a known state.
                // [spec:pgorm:req:bindings.transactions+2]
                Some(Err(TransactionError::Connection(
                    error @ pgorm::Error::TransactionAborted(_),
                ))) => (true, Some(Err(failure(&error)))),
                Some(Err(TransactionError::Connection(error))) => match &last {
                    // A COMMIT that failed leaves the connection in a state
                    // nobody knows; a BEGIN that did, after a rolled-back
                    // attempt, leaves it as it was.
                    Some(Last { committed, .. }) => (!committed, Some(Err(failure(&error)))),
                    None => {
                        let _ = sender.send(Err(failure(&error)));
                        (true, None)
                    }
                },
                Some(Err(TransactionError::RollbackFailed { rollback, .. })) => {
                    (false, Some(Err(failure(&rollback))))
                }
                Some(Err(_)) => (
                    false,
                    Some(Err(Failure::Internal(UNREPRESENTED.to_owned()))),
                ),
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
        Arc::new(Self {
            attempts: Mutex::new(receiver),
            root,
            connection,
        })
    }

    /// The next attempt's frame, once pgorm has opened it.
    pub async fn attempt(&self, cancel: &CancellationToken) -> Result<Arc<State<O>>, Failure> {
        let mut attempts = self.attempts.try_lock().map_err(|_| {
            Failure::Lifecycle("a retrying transaction opens one attempt at a time".to_owned())
        })?;
        let attempt = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(Failure::Cancelled),
            attempt = attempts.recv() => attempt
                .ok_or_else(|| Failure::Lifecycle("the transaction is closed".to_owned())),
        };
        if attempt.is_err() {
            self.abort();
        }
        attempt?
    }

    /// End the whole transaction, whichever attempt it is on, discarding its
    /// connection.
    pub fn abort(&self) {
        self.root.aborted.cancel();
    }

    /// The connection the transaction holds.
    pub fn connection(&self) -> &Arc<ConnectionState<O>> {
        &self.connection
    }
}
