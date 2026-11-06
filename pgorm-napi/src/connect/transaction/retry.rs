//! A transaction run again from `BEGIN` while it fails with a serialization
//! failure or a deadlock: pgorm's `transaction_with_retry`, whose closure is
//! called once per attempt, each attempt a frame the JavaScript callback runs
//! on.
//!
//! Each attempt's callback ends with a commit or a rollback request, whose
//! reply waits on pgorm: `true` once pgorm has rolled the attempt back, or
//! seen its `COMMIT` fail retryably, and opened the next; otherwise the
//! outcome of the whole transaction.
//!
//! What the attempts share is held behind a lock the closure owns a handle
//! to, rather than borrowed by it: a closure pgorm calls once per attempt that
//! borrows from the task around it makes that task's future `Send` for no
//! lifetime the compiler can name, and `tokio::spawn` refuses it.

use std::sync::Arc;

use neon::{prelude::*, types::Finalize};
use pgorm::{RetryableError, TransactionError, TransactionMode};
use tokio::sync::{Mutex, mpsc, oneshot};

use super::{
    Abort, Attempted, ConnectionHandle, ConnectionState, Exit, Failure, Finished, Frame, Halt,
    Held, Operation, Root, State, TransactionHandle, Tx, abort_argument, failure, mode, settle,
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("connectionBeginRetrying", connection_begin_retrying)?;
    cx.export_function("retryingAttempt", retrying_attempt)?;
    cx.export_function("retryingAbort", retrying_abort)?;
    Ok(())
}

/// The closure's error: whether the callback's failure is one to retry.
struct Again(bool);

impl RetryableError for Again {
    fn is_retryable(&self) -> bool {
        self.0
    }
}

/// The reply the last attempt's callback is waiting on, whether it asked to
/// commit, and the attempt it ended.
struct Last {
    reply: Attempted,
    committed: bool,
    state: Arc<State>,
}

/// What every attempt of one retrying transaction shares.
struct Attempts {
    sender: mpsc::UnboundedSender<Result<Arc<State>, Failure>>,
    halt: Halt,
    connection: Arc<ConnectionState>,
    root: Arc<Root>,
    last: Option<Last>,
}

impl Attempts {
    /// One call of pgorm's closure: answer the attempt before it, which pgorm
    /// has ended, with `true`, and serve this attempt's frame.
    async fn serve(&mut self, tx: Tx<'_, '_>) -> Result<(), Again> {
        if let Some(previous) = self.last.take() {
            previous.state.released.cancel();
            if previous.reply.send(Ok(true)).is_err() {
                return Err(Again(false));
            }
        }
        let (state, receiver) = State::attempt(self.connection.clone(), self.root.clone());
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

/// A retrying transaction as JavaScript holds it: the attempts pgorm opens,
/// in turn.
pub(crate) struct RetryingState {
    attempts: Mutex<mpsc::UnboundedReceiver<Result<Arc<State>, Failure>>>,
    root: Arc<Root>,
}

pub(crate) struct RetryingHandle(Arc<RetryingState>);

impl Finalize for RetryingHandle {}

impl Drop for RetryingHandle {
    fn drop(&mut self) {
        self.0.root.aborted.cancel();
    }
}

impl std::fmt::Debug for RetryingHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetryingHandle").finish_non_exhaustive()
    }
}

/// Open a transaction run again from `BEGIN` up to `retries` times while it
/// fails retryably, through pgorm's `transaction_with_retry`.
// [spec:pgorm:req:bindings.transaction-retry]
fn begin(mut operation: Operation, mode: TransactionMode, retries: u32) -> Arc<RetryingState> {
    let connection = operation.state.clone();
    let root = Arc::new(Root {
        aborted: tokio_util::sync::CancellationToken::new(),
        finished: tokio_util::sync::CancellationToken::new(),
    });
    let (sender, receiver) = mpsc::unbounded_channel();
    let shared = Arc::new(Mutex::new(Attempts {
        sender: sender.clone(),
        halt: Halt::default(),
        connection: connection.clone(),
        root: root.clone(),
        last: None,
    }));
    let owner = root.clone();
    tokio::spawn(async move {
        let _finished = Finished(owner.finished.clone());
        let halt = shared.lock().await.halt.clone();
        let attempts = shared.clone();
        let ended = tokio::select! {
            () = owner.aborted.cancelled() => None,
            () = connection.closing() => None,
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
        let secrets = &connection.pool.secrets;
        let (healthy, answer) = match ended {
            None => (false, None),
            Some(Ok(()) | Err(TransactionError::Transaction(_))) => (true, Some(Ok(false))),
            // A COMMIT the aborted transaction answered by rolling back leaves
            // the connection in a known state.
            // [spec:pgorm:req:bindings.transactions+2]
            Some(Err(TransactionError::Connection(
                error @ pgorm::Error::TransactionAborted(_),
            ))) => (true, Some(Err(failure(&error, secrets)))),
            Some(Err(TransactionError::Connection(error))) => match &last {
                // A COMMIT that failed leaves the connection in a state
                // nobody knows; a BEGIN that did, after a rolled-back
                // attempt, leaves it as it was.
                Some(Last { committed, .. }) => (!committed, Some(Err(failure(&error, secrets)))),
                None => {
                    let _ = sender.send(Err(failure(&error, secrets)));
                    (true, None)
                }
            },
            Some(Err(TransactionError::RollbackFailed { rollback, .. })) => {
                (false, Some(Err(failure(&rollback, secrets))))
            }
            Some(Err(_)) => (
                false,
                Some(Err(Failure::Internal(
                    "the transaction failed in a way pgorm-napi does not represent".to_owned(),
                ))),
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
    Arc::new(RetryingState {
        attempts: Mutex::new(receiver),
        root,
    })
}

/// `connectionBeginRetrying(connection, mode, isolation, retries)`: a
/// retrying transaction on `connection`, its first `BEGIN` sent at once
/// and each attempt handed out by `retryingAttempt`.
fn connection_begin_retrying(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let connection = cx.argument::<JsBox<ConnectionHandle>>(0)?.0.clone();
    let mode = mode(&mut cx, 1)?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let retries = cx.argument::<JsNumber>(3)?.value(&mut cx) as u32;
    let operation = match Operation::begin(connection) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move { Ok(begin(operation, mode, retries)) };
    settle::promise(&mut cx, work, |cx, state| {
        Ok(cx.boxed(RetryingHandle(state)).upcast())
    })
}

/// `retryingAttempt(retrying, abort)`: the next attempt's transaction, once
/// pgorm has opened it.
fn retrying_attempt(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = cx.argument::<JsBox<RetryingHandle>>(0)?.0.clone();
    let abort: Abort = abort_argument(&mut cx, 1)?;
    let work = async move {
        let mut attempts = state.attempts.try_lock().map_err(|_| {
            Failure::Lifecycle("a retrying transaction opens one attempt at a time".to_owned())
        })?;
        let attempt = abort
            .guard(async {
                attempts
                    .recv()
                    .await
                    .ok_or_else(|| Failure::Lifecycle("the transaction is closed".to_owned()))
            })
            .await;
        if attempt.is_err() {
            state.root.aborted.cancel();
        }
        attempt?
    };
    settle::promise(&mut cx, work, |cx, attempt| {
        Ok(cx.boxed(TransactionHandle(attempt)).upcast())
    })
}

/// `retryingAbort(retrying)`: end the whole transaction, whichever attempt
/// it is on, discarding its connection.
fn retrying_abort(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let state = cx.argument::<JsBox<RetryingHandle>>(0)?.0.clone();
    state.root.aborted.cancel();
    Ok(cx.undefined())
}
