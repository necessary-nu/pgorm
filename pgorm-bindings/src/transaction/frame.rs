//! The task that serves a transaction's frames, inside the closures pgorm
//! opens them around.

use std::{future::Future, sync::Arc, time::Duration};

use futures_util::future::BoxFuture;
use pgorm::{TransactionError, TransactionMode, TransactionTrait};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::{Attempted, Command, ENDED, Finished, Flow, Opened, State, Tx};
use crate::{
    Failure,
    connection::{Held, Operation},
};

/// What a transaction ending in a way no arm here names says: its connection
/// is left in a state nobody knows, as a failed `ROLLBACK` leaves it.
pub(super) const UNREPRESENTED: &str =
    "the transaction failed in a way the binding does not represent";

/// How a frame's command loop ended short of discarding it: the error its
/// closure returns, so that pgorm rolls the frame back.
pub(super) enum Exit {
    /// The language rolled the frame back, saying whether its failure is one
    /// to retry; the reply goes once the rollback has.
    RolledBack(Attempted, bool),
    /// The frame's handle was dropped unfinished.
    Abandoned,
}

/// What ends a frame by dropping its transaction's future rather than letting
/// it finish, which rolls back without awaiting the server: `discard`, which
/// the frame cancels as it gives its connection up, and `ending`, which an
/// abandoned frame's closure cancels as it returns, leaving the rollback five
/// seconds before the connection is discarded instead.
#[derive(Clone, Default)]
pub(super) struct Halt {
    discard: CancellationToken,
    ending: CancellationToken,
}

impl Halt {
    /// The future holds owned tokens rather than a borrow of the halt, so a
    /// transaction future whose closure is called for any lifetime stays
    /// `Send` around it.
    pub(super) fn run<T>(
        &self,
        transaction: impl Future<Output = T>,
    ) -> impl Future<Output = Option<T>> {
        let (discard, ending) = (self.discard.clone(), self.ending.clone());
        async move {
            tokio::select! {
                () = discard.cancelled_owned() => None,
                () = async move {
                    ending.cancelled_owned().await;
                    tokio::time::sleep(Duration::from_secs(5)).await;
                } => None,
                ended = transaction => Some(ended),
            }
        }
    }

    /// Give the frame's connection up. The closure never returns, so pgorm
    /// awaits no rollback on a connection in an unknown state, and
    /// [`Halt::run`] drops the transaction instead.
    async fn discard<T>(&self) -> T {
        self.discard.cancel();
        std::future::pending().await
    }
}

/// How a frame ended, for the code that answers it.
pub(super) struct Completion {
    /// Whether the connection is in a state its frame knows.
    pub(super) healthy: bool,
    reply: Option<Attempted>,
    result: Result<(), Failure>,
}

impl Completion {
    fn discarded() -> Self {
        Self {
            healthy: false,
            reply: None,
            result: Err(Failure::Lifecycle(
                "the transaction's connection was discarded".to_owned(),
            )),
        }
    }

    pub(super) fn respond(self) {
        if let Some(reply) = self.reply {
            let _ = reply.send(self.result.map(|()| false));
        }
    }
}

/// How a frame's transaction ended, for the code that opened it.
pub(super) enum Ended {
    /// `BEGIN` or `SAVEPOINT` was refused, the closure never ran, and the
    /// wait for the frame to open is still to be answered.
    Refused(Opened, pgorm::Error),
    Ran(Completion),
}

/// One frame: its commands, and what outlives the closure serving them.
pub(super) struct Frame<O> {
    state: Arc<State<O>>,
    commands: mpsc::UnboundedReceiver<Command<O>>,
    /// Answered from inside the closure as it starts, so a refused opening
    /// finds it unanswered.
    opened: Option<Opened>,
    /// A commit's reply, held outside the closure because the `COMMIT` runs
    /// once the closure has returned and the reply carries its failure.
    pub(super) committed: Option<Attempted>,
    /// Set as the frame opens and dropped with the frame, once its `COMMIT`
    /// or `ROLLBACK` has finished too.
    finished: Option<Finished>,
    pub(super) halt: Halt,
}

impl<O: Send + Sync + 'static> Frame<O> {
    pub(super) fn new(
        state: Arc<State<O>>,
        commands: mpsc::UnboundedReceiver<Command<O>>,
        opened: Opened,
    ) -> Self {
        Self::halted_by(state, commands, opened, Halt::default())
    }

    /// A frame given up through `halt`, which every attempt of a retrying
    /// transaction shares.
    pub(super) fn halted_by(
        state: Arc<State<O>>,
        commands: mpsc::UnboundedReceiver<Command<O>>,
        opened: Opened,
        halt: Halt,
    ) -> Self {
        Self {
            state,
            commands,
            opened: Some(opened),
            committed: None,
            finished: None,
            halt,
        }
    }

    fn failure(&self, error: &pgorm::Error) -> Failure {
        self.state.connection.pool.failure(error)
    }

    /// Serve the frame's commands inside its transaction, a savepoint's frame
    /// nested inside while it is open: `Ok` once the language commits, an
    /// `Exit` once it rolls back or the handle is dropped unfinished.
    // [spec:pgorm:req:bindings.transactions+2]
    pub(super) fn serve<'a>(&'a mut self, mut tx: Tx<'a, '_>) -> BoxFuture<'a, Result<(), Exit>> {
        Box::pin(async move {
            self.finished = Some(Finished(self.state.finished.clone()));
            if self
                .opened
                .take()
                .is_some_and(|opened| opened.send(Ok(())).is_err())
            {
                return self.halt.discard().await;
            }
            loop {
                let command = tokio::select! {
                    () = self.state.abandoned.cancelled() => None,
                    command = self.commands.recv() => command,
                };
                let Some(command) = command else {
                    self.halt.ending.cancel();
                    return Err(Exit::Abandoned);
                };
                match command {
                    Command::Work(job) => {
                        let flow = tokio::select! {
                            () = self.state.abandoned.cancelled() => Flow::Lost,
                            flow = job(&mut tx) => flow,
                        };
                        if flow == Flow::Lost {
                            return self.halt.discard().await;
                        }
                    }
                    Command::Finish {
                        commit: true,
                        reply,
                        ..
                    } => {
                        self.committed = Some(reply);
                        return Ok(());
                    }
                    Command::Finish {
                        commit: false,
                        retryable,
                        reply,
                    } => return Err(Exit::RolledBack(reply, retryable)),
                    Command::Begin {
                        child,
                        receiver,
                        opened,
                        parent_lock,
                    } => {
                        let mut frame = Frame::new(child.clone(), receiver, opened);
                        let halt = frame.halt.clone();
                        let ended = match &mut tx {
                            Tx::Plain(tx) => {
                                halt.run(tx.transaction(async |savepoint| {
                                    frame.serve(Tx::Plain(savepoint)).await
                                }))
                                .await
                            }
                            Tx::Metered(tx) => {
                                halt.run(tx.transaction(async |savepoint| {
                                    frame.serve(Tx::Metered(savepoint)).await
                                }))
                                .await
                            }
                        };
                        let completion = match frame.end(ended) {
                            Ended::Ran(completion) => completion,
                            Ended::Refused(opened, error) => {
                                if opened.send(Err(self.failure(&error))).is_err() {
                                    return self.halt.discard().await;
                                }
                                continue;
                            }
                        };
                        drop(frame);
                        let healthy = completion.healthy;
                        drop(parent_lock);
                        completion.respond();
                        child.released.cancel();
                        if !healthy {
                            return self.halt.discard().await;
                        }
                    }
                }
            }
        })
    }

    /// What the frame's transaction ended in: `None` is a future [`Halt`]
    /// dropped. A frame whose `COMMIT` or `ROLLBACK` failed has left its
    /// connection in a state nobody knows, so it is not healthy, and the reply
    /// carries the failure.
    pub(super) fn end(&mut self, ended: Option<Result<(), TransactionError<Exit>>>) -> Ended {
        let Some(ended) = ended else {
            return Ended::Ran(Completion::discarded());
        };
        let completion = |healthy, reply, result| Completion {
            healthy,
            reply,
            result,
        };
        Ended::Ran(match ended {
            Ok(()) => completion(true, self.committed.take(), Ok(())),
            // A `COMMIT` an aborted transaction answered by rolling back, or a
            // `RELEASE` it refused and pgorm rolled back to: the commit fails,
            // and the connection is in a known state.
            // [spec:pgorm:req:bindings.transactions+2]
            Err(TransactionError::Connection(error @ pgorm::Error::TransactionAborted(_))) => {
                completion(true, self.committed.take(), Err(self.failure(&error)))
            }
            // An aborted transaction whose ROLLBACK then failed leaves the
            // connection unknown, and fails with both errors as a block's
            // failed rollback does.
            // [spec:pgorm:req:bindings.rollback-failed]
            Err(TransactionError::Connection(error @ pgorm::Error::RollbackFailed { .. })) => {
                completion(false, self.committed.take(), Err(self.failure(&error)))
            }
            Err(TransactionError::Connection(error)) => {
                if let Some(opened) = self.opened.take() {
                    return Ended::Refused(opened, error);
                }
                completion(false, self.committed.take(), Err(self.failure(&error)))
            }
            Err(TransactionError::Transaction(Exit::RolledBack(reply, _))) => {
                completion(true, Some(reply), Ok(()))
            }
            Err(TransactionError::Transaction(Exit::Abandoned)) => completion(true, None, Ok(())),
            Err(TransactionError::RollbackFailed { cause, rollback }) => completion(
                false,
                match cause {
                    Exit::RolledBack(reply, _) => Some(reply),
                    Exit::Abandoned => None,
                },
                Err(self.failure(&rollback)),
            ),
            Err(_) => completion(
                false,
                self.committed.take(),
                Err(Failure::Internal(UNREPRESENTED.to_owned())),
            ),
        })
    }
}

/// Open a transaction on the connection `operation` took, which it holds
/// until the transaction finishes: a task takes the connection and serves
/// the transaction from inside the closure pgorm opens it around.
// [spec:pgorm:req:bindings.transactions+2]
pub async fn begin<O: Send + Sync + 'static>(
    mut operation: Operation<O>,
    mode: TransactionMode,
    cancel: &CancellationToken,
) -> Result<Arc<State<O>>, Failure> {
    let connection = operation.state.clone();
    let (state, receiver) = State::channel(connection, None);
    let (opened, receive) = oneshot::channel::<Result<(), Failure>>();
    let mut pending = state.pending();
    let owner = state.clone();
    tokio::spawn(async move {
        let _finished = Finished(owner.root.finished.clone());
        let served = tokio::select! {
            () = owner.root.aborted.cancelled() => None,
            () = owner.connection.closing() => None,
            served = async {
                let db = match operation.connection_mut() {
                    Ok(db) => db,
                    Err(error) => {
                        let _ = opened.send(Err(error));
                        return None;
                    }
                };
                let mut frame = Frame::new(owner.clone(), receiver, opened);
                let halt = frame.halt.clone();
                let ended = match db {
                    Held::Plain(db) => halt
                        .run(db.transaction_with(mode, async |tx| frame.serve(Tx::Plain(tx)).await))
                        .await,
                    Held::Metered(db) => halt
                        .run(db.transaction_with(mode, async |tx| frame.serve(Tx::Metered(tx)).await))
                        .await,
                };
                match frame.end(ended) {
                    Ended::Ran(completion) => Some(completion),
                    Ended::Refused(opened, error) => {
                        let _ = opened.send(Err(frame.failure(&error)));
                        None
                    }
                }
            } => served,
        };
        match served {
            Some(completion) => {
                if completion.healthy {
                    operation.restore();
                } else {
                    drop(operation);
                }
                completion.respond();
                owner.released.cancel();
            }
            None => drop(operation),
        }
    });
    tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(Failure::Cancelled),
        opened = receive => opened.map_err(|_| Failure::Lifecycle(ENDED.to_owned()))??,
    }
    pending.complete();
    Ok(state)
}
