//! Transactions and savepoints. pgorm opens a transaction only around a
//! closure, whose `DatabaseTransaction` borrows its connection, and a
//! savepoint's its parent, exclusively; that borrow cannot cross into
//! JavaScript, so a task on the runtime serves each frame's commands inside
//! its closure, nested savepoint frames inside their parent's, and
//! JavaScript's handles send it owned commands. While a savepoint is open its
//! parent's frame is suspended inside it, holding the parent's lock: the
//! parent refuses a statement, a commit, a rollback or another savepoint until
//! the child finishes, and refuses a second request while one runs, rather
//! than queue or race it.

use std::{fmt, future::Future, sync::Arc, time::Duration};

use futures_util::future::BoxFuture;
use neon::{prelude::*, types::Finalize};
use pgorm::{
    CopyColumn, DatabaseTransaction, IsolationLevel, TransactionError, TransactionMode,
    TransactionTrait, metric::InstrumentedTransaction,
};
use tokio::sync::{Mutex, OwnedMutexGuard, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::{
    Abort, ConnectionHandle, ConnectionState, Held, Operation, Outcome, Statement, abort_argument,
    copy::{
        self, CopyInHandle, CopyOutHandle, InLink, InState, OutLink, OutState, Release, Settle,
    },
    job::{Database, Job, Output, taken},
    metrics::Metrics,
};
use crate::{
    errors::{Failure, Redactions, failure},
    settle,
    statements::{CopyFrom, CopyStatement, CopyTo, copy_at},
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("connectionBegin", connection_begin)?;
    cx.export_function("transactionRun", transaction_run)?;
    cx.export_function("transactionBegin", transaction_begin)?;
    cx.export_function("transactionFinish", transaction_finish)?;
    cx.export_function("transactionAbort", transaction_abort)?;
    cx.export_function("transactionWaitClosed", transaction_wait_closed)?;
    cx.export_function("transactionClosed", transaction_closed)?;
    cx.export_function("transactionJob", transaction_job)?;
    cx.export_function("transactionCopyIn", transaction_copy_in)?;
    cx.export_function("transactionCopyOut", transaction_copy_out)?;
    cx.export_function("transactionCopyColumns", transaction_copy_columns)?;
    Ok(())
}

type Finish = oneshot::Sender<Result<(), Failure>>;

enum Command {
    Run {
        statement: Statement,
        reply: oneshot::Sender<Result<Outcome, Failure>>,
    },
    Job {
        job: Job,
        reply: oneshot::Sender<Result<Output, Failure>>,
    },
    /// A COPY's columns, described without starting it.
    Describe {
        copy: Box<CopyStatement>,
        reply: oneshot::Sender<Result<Vec<CopyColumn>, Failure>>,
    },
    Finish {
        commit: bool,
        reply: Finish,
    },
    Begin {
        child: Arc<State>,
        receiver: mpsc::UnboundedReceiver<Command>,
        opened: Finish,
        parent_lock: OwnedMutexGuard<()>,
    },
    /// A COPY, which holds the frame's lock until it ends.
    CopyIn {
        copy: Box<CopyFrom>,
        link: InLink,
        opened: Finish,
        abort: Abort,
        release: Release,
        lock: OwnedMutexGuard<()>,
    },
    CopyOut {
        copy: Box<CopyTo>,
        link: OutLink,
        opened: Finish,
        abort: Abort,
        release: Release,
        lock: OwnedMutexGuard<()>,
    },
}

/// What every frame of one transaction shares: `aborted` ends the whole
/// transaction and discards its connection, `finished` fires once its task
/// has ended.
struct Root {
    aborted: CancellationToken,
    finished: CancellationToken,
}

/// One transaction or savepoint, as its JavaScript handle reaches it.
pub(crate) struct State {
    connection: Arc<ConnectionState>,
    commands: mpsc::UnboundedSender<Command>,
    busy: Arc<Mutex<()>>,
    abandoned: CancellationToken,
    finished: CancellationToken,
    root: Arc<Root>,
    released: CancellationToken,
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State")
            .field("closed", &self.closed())
            .finish_non_exhaustive()
    }
}

/// Cancels a token when dropped, however the frame holding it ends.
struct Finished(CancellationToken);

impl Drop for Finished {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// A request whose outcome is unknown once abandoned: dropped before it
/// completes — its caller aborted — it ends the whole transaction, whose
/// connection is then discarded.
// [spec:pgorm:req:bindings.cancellation]
struct Pending {
    aborted: CancellationToken,
    complete: bool,
}

impl Pending {
    fn new(aborted: CancellationToken) -> Self {
        Self {
            aborted,
            complete: false,
        }
    }

    fn complete(&mut self) {
        self.complete = true;
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.complete {
            self.aborted.cancel();
        }
    }
}

struct Completion {
    healthy: bool,
    reply: Option<Finish>,
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

    fn respond(self) {
        if let Some(reply) = self.reply {
            let _ = reply.send(self.result);
        }
    }
}

impl State {
    fn channel(
        connection: Arc<ConnectionState>,
        parent: Option<&Arc<Self>>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Command>) {
        let (commands, receiver) = mpsc::unbounded_channel();
        let root = parent.map_or_else(
            || {
                Arc::new(Root {
                    aborted: CancellationToken::new(),
                    finished: CancellationToken::new(),
                })
            },
            |parent| parent.root.clone(),
        );
        let abandoned = parent.map_or_else(CancellationToken::new, |parent| {
            parent.abandoned.child_token()
        });
        (
            Arc::new(Self {
                connection,
                commands,
                busy: Arc::new(Mutex::new(())),
                abandoned,
                finished: CancellationToken::new(),
                root,
                released: CancellationToken::new(),
            }),
            receiver,
        )
    }

    fn closed(&self) -> bool {
        self.finished.is_cancelled()
            || self.abandoned.is_cancelled()
            || self.root.aborted.is_cancelled()
            || self.root.finished.is_cancelled()
            || self.connection.ensure_open().is_err()
    }

    fn ensure_open(&self) -> Result<(), Failure> {
        if self.closed() {
            Err(Failure::Lifecycle("the transaction is closed".to_owned()))
        } else {
            Ok(())
        }
    }

    /// The frame's lock, refusing rather than waiting when a request or an
    /// open savepoint holds it. Taken on the JavaScript thread as each request
    /// is made, so requests are refused in the order JavaScript made them.
    // [spec:pgorm:req:bindings.transactions+1]
    fn lock(&self) -> Result<OwnedMutexGuard<()>, Failure> {
        self.ensure_open()?;
        self.busy.clone().try_lock_owned().map_err(|_| {
            Failure::Lifecycle(
                "the transaction is busy: a statement runs on it, or a savepoint of it is open"
                    .to_owned(),
            )
        })
    }

    fn send(&self, command: Command) -> Result<(), Failure> {
        self.commands
            .send(command)
            .map_err(|_| Failure::Lifecycle("the transaction has ended".to_owned()))
    }

    /// Send the request `command` makes of the reply it is given, and wait
    /// for that reply, holding the lock taken as JavaScript started it: a
    /// statement, a registered entity's job, or a COPY's description.
    async fn request<T>(
        &self,
        _lock: OwnedMutexGuard<()>,
        command: impl FnOnce(oneshot::Sender<Result<T, Failure>>) -> Command,
        abort: &Abort,
    ) -> Result<T, Failure> {
        let (reply, receive) = oneshot::channel();
        let mut pending = Pending::new(self.root.aborted.clone());
        self.send(command(reply))?;
        let outcome = abort
            .guard(async {
                receive
                    .await
                    .map_err(|_| Failure::Lifecycle("the transaction has ended".to_owned()))
            })
            .await?;
        pending.complete();
        outcome
    }

    async fn finish(&self, _lock: OwnedMutexGuard<()>, commit: bool) -> Result<(), Failure> {
        let (reply, receive) = oneshot::channel();
        let mut pending = Pending::new(self.root.aborted.clone());
        self.send(Command::Finish { commit, reply })?;
        let outcome = receive
            .await
            .map_err(|_| Failure::Lifecycle("the transaction has ended".to_owned()))?;
        pending.complete();
        outcome?;
        self.released.cancelled().await;
        Ok(())
    }

    /// What ends a COPY in this frame from outside it.
    fn copy_stop(&self, abort: Abort) -> copy::Stop {
        copy::Stop::frame(abort, &self.connection, &self.abandoned, &self.root.aborted)
    }

    /// Free the frame once a COPY in it has ended: its lock first, so that a
    /// statement JavaScript runs as the COPY settles finds it free. A COPY
    /// whose connection is discarded ends the whole transaction, nothing in
    /// it committed, as an aborted statement does, and the frame is told so.
    // [spec:pgorm:req:bindings.copy]
    fn after_copy(&self, settle: Settle, lock: OwnedMutexGuard<()>, release: Release) -> bool {
        drop(lock);
        let discarded = settle == Settle::Discard;
        if discarded {
            self.root.aborted.cancel();
        }
        drop(release);
        discarded
    }

    /// A savepoint, which holds this frame's lock until it finishes.
    async fn begin(
        self: &Arc<Self>,
        parent_lock: OwnedMutexGuard<()>,
        abort: &Abort,
    ) -> Result<Arc<Self>, Failure> {
        let (child, receiver) = Self::channel(self.connection.clone(), Some(self));
        let (opened, receive) = oneshot::channel();
        let mut pending = Pending::new(self.root.aborted.clone());
        self.send(Command::Begin {
            child: child.clone(),
            receiver,
            opened,
            parent_lock,
        })?;
        abort
            .guard(async {
                receive
                    .await
                    .map_err(|_| Failure::Lifecycle("the transaction has ended".to_owned()))?
            })
            .await?;
        pending.complete();
        Ok(child)
    }
}

/// How a frame's command loop ended short of discarding it: the error its
/// closure returns, so that pgorm rolls the frame back.
enum Exit {
    /// JavaScript rolled the frame back; the reply goes once the rollback has.
    RolledBack(Finish),
    /// The frame's handle was collected unfinished.
    Abandoned,
}

/// What ends a frame by dropping its transaction's future rather than letting
/// it finish, which rolls back without awaiting the server: `discard`, which
/// the frame cancels as it gives its connection up, and `ending`, which an
/// abandoned frame's closure cancels as it returns, leaving the rollback five
/// seconds before the connection is discarded instead.
#[derive(Clone, Default)]
struct Halt {
    discard: CancellationToken,
    ending: CancellationToken,
}

impl Halt {
    async fn run<T>(&self, transaction: impl Future<Output = T>) -> Option<T> {
        tokio::select! {
            () = self.discard.cancelled() => None,
            () = async {
                self.ending.cancelled().await;
                tokio::time::sleep(Duration::from_secs(5)).await;
            } => None,
            ended = transaction => Some(ended),
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

/// How a frame's transaction ended, for the code that opened it.
enum Ended {
    /// `BEGIN` or `SAVEPOINT` was refused, the closure never ran, and the
    /// wait for the frame to open is still to be answered.
    Refused(Finish, pgorm::Error),
    Ran(Completion),
}

/// The transaction a frame serves: a plain one, or pgorm's metrics wrapper
/// around one on an instrumented pool's connection, whose savepoints are
/// instrumented too.
// [spec:pgorm:req:bindings.metrics]
enum Tx<'a, 't> {
    Plain(&'a mut DatabaseTransaction<'t>),
    Metered(&'a mut InstrumentedTransaction<'t, Metrics>),
}

impl Tx<'_, '_> {
    fn database(&self) -> Database<'_> {
        match self {
            Self::Plain(tx) => Database::Transaction(tx),
            Self::Metered(tx) => Database::InstrumentedTransaction(tx),
        }
    }

    /// A COPY's columns, described in this transaction.
    async fn described(
        &mut self,
        copy: &CopyStatement,
        secrets: &Redactions,
    ) -> Result<Vec<CopyColumn>, Failure> {
        match self {
            Self::Plain(tx) => copy::described(&mut **tx, copy, secrets).await,
            Self::Metered(tx) => copy::described(&mut **tx, copy, secrets).await,
        }
    }
}

/// One frame: its commands, and what outlives the closure serving them.
struct Frame {
    state: Arc<State>,
    commands: mpsc::UnboundedReceiver<Command>,
    secrets: Redactions,
    /// Answered from inside the closure as it starts, so a refused opening
    /// finds it unanswered.
    opened: Option<Finish>,
    /// A commit's reply, held outside the closure because the `COMMIT` runs
    /// once the closure has returned and the reply carries its failure.
    committed: Option<Finish>,
    /// Set as the frame opens and dropped with the frame, once its `COMMIT`
    /// or `ROLLBACK` has finished too.
    finished: Option<Finished>,
    halt: Halt,
}

impl Frame {
    fn new(state: Arc<State>, commands: mpsc::UnboundedReceiver<Command>, opened: Finish) -> Self {
        let secrets = state.connection.pool.secrets.clone();
        Self {
            state,
            commands,
            secrets,
            opened: Some(opened),
            committed: None,
            finished: None,
            halt: Halt::default(),
        }
    }

    /// Serve the frame's commands inside its transaction, a savepoint's frame
    /// nested inside while it is open: `Ok` once JavaScript commits, an `Exit`
    /// once it rolls back or the handle is collected unfinished.
    // [spec:pgorm:req:bindings.transactions+1]
    fn serve<'a>(&'a mut self, mut tx: Tx<'a, '_>) -> BoxFuture<'a, Result<(), Exit>> {
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
                    Command::Run { statement, reply } => {
                        let db = tx.database();
                        let run = statement.run(&db, &self.secrets);
                        if !self.answer(run, reply).await {
                            return self.halt.discard().await;
                        }
                    }
                    Command::Job { job, reply } => {
                        let run = job(tx.database(), &self.secrets);
                        if !self.answer(run, reply).await {
                            return self.halt.discard().await;
                        }
                    }
                    Command::Describe { copy, reply } => {
                        let described = tx.described(&copy, &self.secrets);
                        if !self.answer(described, reply).await {
                            return self.halt.discard().await;
                        }
                    }
                    Command::Finish {
                        commit: true,
                        reply,
                    } => {
                        self.committed = Some(reply);
                        return Ok(());
                    }
                    Command::Finish {
                        commit: false,
                        reply,
                    } => return Err(Exit::RolledBack(reply)),
                    Command::CopyIn {
                        copy,
                        link,
                        opened,
                        abort,
                        release,
                        lock,
                    } => {
                        let stop = self.state.copy_stop(abort);
                        let settle = match &mut tx {
                            Tx::Plain(tx) => {
                                copy::copy_in(&mut **tx, &copy, link, opened, &stop, &self.secrets)
                                    .await
                            }
                            Tx::Metered(tx) => {
                                copy::copy_in(&mut **tx, &copy, link, opened, &stop, &self.secrets)
                                    .await
                            }
                        };
                        if self.state.after_copy(settle, lock, release) {
                            return self.halt.discard().await;
                        }
                    }
                    Command::CopyOut {
                        copy,
                        link,
                        opened,
                        abort,
                        release,
                        lock,
                    } => {
                        let stop = self.state.copy_stop(abort);
                        let settle = match &mut tx {
                            Tx::Plain(tx) => {
                                copy::copy_out(&mut **tx, &copy, link, opened, &stop, &self.secrets)
                                    .await
                            }
                            Tx::Metered(tx) => {
                                copy::copy_out(&mut **tx, &copy, link, opened, &stop, &self.secrets)
                                    .await
                            }
                        };
                        if self.state.after_copy(settle, lock, release) {
                            return self.halt.discard().await;
                        }
                    }
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
                                if opened.send(Err(failure(&error, &self.secrets))).is_err() {
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

    /// Run a request and send its outcome on `reply`: `false` when the frame
    /// is to be given up, its handle collected while the request ran or its
    /// caller gone, the request's effect on the transaction then unknown.
    async fn answer<T>(&self, request: impl Future<Output = T>, reply: oneshot::Sender<T>) -> bool {
        let outcome = tokio::select! {
            () = self.state.abandoned.cancelled() => return false,
            outcome = request => outcome,
        };
        reply.send(outcome).is_ok()
    }

    /// What the frame's transaction ended in: `None` is a future [`Halt`]
    /// dropped. A frame whose `COMMIT` or `ROLLBACK` failed has left its
    /// connection in a state nobody knows, so it is not healthy, and the reply
    /// carries the failure.
    fn end(&mut self, ended: Option<Result<(), TransactionError<Exit>>>) -> Ended {
        let Some(ended) = ended else {
            return Ended::Ran(Completion::discarded());
        };
        Ended::Ran(match ended {
            Ok(()) => Completion {
                healthy: true,
                reply: self.committed.take(),
                result: Ok(()),
            },
            // A `COMMIT` an aborted transaction answered by rolling back, or a
            // `RELEASE` it refused and pgorm rolled back to: the commit fails,
            // and the connection is in a known state.
            // [spec:pgorm:req:bindings.transactions+1]
            Err(TransactionError::Connection(error @ pgorm::Error::TransactionAborted(_))) => {
                Completion {
                    healthy: true,
                    reply: self.committed.take(),
                    result: Err(failure(&error, &self.secrets)),
                }
            }
            // An aborted transaction whose ROLLBACK then failed: the rollback's
            // failure is what leaves the connection unknown.
            Err(TransactionError::Connection(pgorm::Error::RollbackFailed {
                rollback, ..
            })) => Completion {
                healthy: false,
                reply: self.committed.take(),
                result: Err(failure(&rollback, &self.secrets)),
            },
            Err(TransactionError::Connection(error)) => {
                if let Some(opened) = self.opened.take() {
                    return Ended::Refused(opened, error);
                }
                Completion {
                    healthy: false,
                    reply: self.committed.take(),
                    result: Err(failure(&error, &self.secrets)),
                }
            }
            Err(TransactionError::Transaction(Exit::RolledBack(reply))) => Completion {
                healthy: true,
                reply: Some(reply),
                result: Ok(()),
            },
            Err(TransactionError::Transaction(Exit::Abandoned)) => Completion {
                healthy: true,
                reply: None,
                result: Ok(()),
            },
            Err(TransactionError::RollbackFailed { cause, rollback }) => Completion {
                healthy: false,
                reply: match cause {
                    Exit::RolledBack(reply) => Some(reply),
                    Exit::Abandoned => None,
                },
                result: Err(failure(&rollback, &self.secrets)),
            },
            // A failure with no arm here leaves the connection in a state
            // nobody knows, as a failed ROLLBACK does.
            Err(_) => Completion {
                healthy: false,
                reply: self.committed.take(),
                result: Err(Failure::Internal(
                    "the transaction failed in a way pgorm-napi does not represent".to_owned(),
                )),
            },
        })
    }
}

/// Open a transaction on `connection`, which it holds until it finishes: a
/// task takes the connection out of its slot and serves the transaction from
/// inside the closure pgorm opens it around.
// [spec:pgorm:req:bindings.transactions+1]
async fn begin(
    mut operation: Operation,
    mode: TransactionMode,
    abort: Abort,
) -> Result<Arc<State>, Failure> {
    let connection = operation.state.clone();
    let (state, receiver) = State::channel(connection, None);
    let (opened, receive) = oneshot::channel::<Result<(), Failure>>();
    let mut pending = Pending::new(state.root.aborted.clone());
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
                        let _ = opened.send(Err(failure(&error, &frame.secrets)));
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
    abort
        .guard(async {
            receive
                .await
                .map_err(|_| Failure::Lifecycle("the transaction ended as it began".to_owned()))?
        })
        .await?;
    pending.complete();
    Ok(state)
}

/// How a transaction lent to JavaScript ended for the work it was lent to.
pub(crate) enum Lent<T> {
    /// The work finished, as it answered; the transaction is as its last
    /// command left it.
    Ran(T),
    /// A command's outcome was lost, so nobody knows the transaction's state.
    Discarded,
}

/// Serve `tx`, a transaction pgorm-migration's runner holds, to the handle
/// `work` is given, for as long as the future it returns runs. The handle
/// closes once that future finishes, so a command JavaScript sends with it
/// afterwards is refused rather than run in a transaction the runner has
/// moved on from.
// [spec:pgorm:req:bindings.migrations]    a migration's `up` runs in the batch's transaction
pub(crate) async fn lend<F>(
    pool: Arc<super::PoolState>,
    tx: &mut DatabaseTransaction<'_>,
    work: impl FnOnce(TransactionHandle) -> F,
) -> Lent<F::Output>
where
    F: Future,
{
    let connection = Arc::new(ConnectionState {
        pool,
        closed: CancellationToken::new(),
        slot: Arc::new(Mutex::new(None)),
    });
    let (state, receiver) = State::channel(connection, None);
    // Held until the frame has answered it, as an opening transaction's
    // caller holds its own.
    let (opened, _opening) = oneshot::channel();
    let mut frame = Frame::new(state.clone(), receiver, opened);
    let running = work(TransactionHandle(state.clone()));
    let halt = frame.halt.clone();
    let served = halt.run(frame.serve(Tx::Plain(tx)));
    tokio::pin!(running, served);
    let finished = tokio::select! {
        biased;
        finished = &mut running => Some(finished),
        served = &mut served => match served {
            // The handle was collected while the work still runs, and can
            // send nothing more.
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

/// The JavaScript `Transaction`'s native half. Collected unfinished, it rolls
/// back as a backstop; `commit`, `rollback` and `close` are what end it.
#[derive(Debug)]
pub(crate) struct TransactionHandle(Arc<State>);

impl Finalize for TransactionHandle {}

impl Drop for TransactionHandle {
    fn drop(&mut self) {
        self.0.abandoned.cancel();
    }
}

fn state(cx: &mut FunctionContext) -> NeonResult<Arc<State>> {
    Ok(cx.argument::<JsBox<TransactionHandle>>(0)?.0.clone())
}

/// A transaction's mode and isolation level, named as JavaScript names them.
fn mode(cx: &mut FunctionContext, at: usize) -> NeonResult<TransactionMode> {
    let mode = cx.argument::<JsString>(at)?.value(cx);
    let isolation = cx.argument::<JsValue>(at + 1)?;
    let isolation = if isolation.is_a::<JsNull, _>(cx) || isolation.is_a::<JsUndefined, _>(cx) {
        None
    } else {
        let name = isolation.downcast_or_throw::<JsString, _>(cx)?.value(cx);
        Some(match name.as_str() {
            "readUncommitted" => IsolationLevel::ReadUncommitted,
            "readCommitted" => IsolationLevel::ReadCommitted,
            "repeatableRead" => IsolationLevel::RepeatableRead,
            "serializable" => IsolationLevel::Serializable,
            _ => {
                return cx.throw_type_error(
                    "isolation is \"readUncommitted\", \"readCommitted\", \"repeatableRead\" or \
                     \"serializable\"",
                );
            }
        })
    };
    match (mode.as_str(), isolation) {
        ("default", None) => Ok(TransactionMode::Default),
        ("readWrite", isolation) => Ok(TransactionMode::ReadWrite { isolation }),
        ("readOnly", isolation) => Ok(TransactionMode::ReadOnly { isolation }),
        ("deferrable", None) => Ok(TransactionMode::DeferrableSnapshot),
        ("default" | "deferrable", Some(_)) => crate::values::read::refuse(
            cx,
            "the default and deferrable modes take no isolation level: choose readWrite or readOnly",
        ),
        _ => cx
            .throw_type_error("mode is \"default\", \"readWrite\", \"readOnly\" or \"deferrable\""),
    }
}

/// `connectionBegin(connection, mode, isolation, abort)`.
fn connection_begin(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let connection = cx.argument::<JsBox<ConnectionHandle>>(0)?.0.clone();
    let mode = mode(&mut cx, 1)?;
    let abort = abort_argument(&mut cx, 3)?;
    let operation = match Operation::begin(connection) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    settle::promise(&mut cx, begin(operation, mode, abort), |cx, state| {
        Ok(cx.boxed(TransactionHandle(state)).upcast())
    })
}

/// `transactionRun(transaction, terminal, sql, params, tagged, abort)`.
fn transaction_run(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let statement = Statement::read(&mut cx, 1)?;
    let tagged = cx.argument::<JsBoolean>(4)?.value(&mut cx);
    let abort = abort_argument(&mut cx, 5)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move {
        let run = |reply| Command::Run { statement, reply };
        state.request(lock, run, &abort).await
    };
    settle::promise(&mut cx, work, move |cx, outcome| {
        outcome.into_js(cx, tagged)
    })
}

/// `transactionBegin(transaction, abort)`: a savepoint.
fn transaction_begin(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let abort = abort_argument(&mut cx, 1)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move { state.begin(lock, &abort).await };
    settle::promise(&mut cx, work, |cx, child| {
        Ok(cx.boxed(TransactionHandle(child)).upcast())
    })
}

/// `transactionFinish(transaction, commit)`: commit or roll back, resolving
/// once the parent — the connection, or the savepoint's transaction — is
/// free again.
fn transaction_finish(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let commit = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move { state.finish(lock, commit).await };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

/// `transactionAbort(transaction)`: end the whole transaction now,
/// discarding its connection, and resolve once its task has ended.
fn transaction_abort(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    state.root.aborted.cancel();
    let work = async move {
        state.root.finished.cancelled().await;
        Ok(())
    };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

/// `transactionWaitClosed(transaction)`: resolve once the frame has
/// released its parent.
fn transaction_wait_closed(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let work = async move {
        tokio::select! {
            () = state.released.cancelled() => {}
            () = state.root.finished.cancelled() => {}
        }
        Ok(())
    };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

fn transaction_closed(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let closed = state(&mut cx)?.closed();
    Ok(cx.boolean(closed))
}

/// `transactionJob(transaction, job, abort)`: a registered entity's job in
/// the transaction, holding its lock as a statement does.
// [spec:pgorm:req:bindings.entities+6]
// [spec:pgorm:req:bindings.registered-pools]
fn transaction_job(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let job = match taken(&mut cx, 1)? {
        Ok(job) => job,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let abort = abort_argument(&mut cx, 2)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move {
        let run = |reply| Command::Job { job, reply };
        state.request(lock, run, &abort).await
    };
    settle::promise(&mut cx, work, |cx, output| output.into_js(cx))
}

/// `transactionCopyColumns(transaction, copy, abort)`: a COPY's columns,
/// described in the transaction, holding its lock as a statement does; a
/// refused description fails the transaction as a refused statement does.
// [spec:pgorm:req:bindings.copy-rows+2]
fn transaction_copy_columns(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let copy = Box::new(copy_at(&mut cx, 1)?);
    let abort = abort_argument(&mut cx, 2)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move {
        let describe = |reply| Command::Describe { copy, reply };
        state.request(lock, describe, &abort).await
    };
    settle::promise(&mut cx, work, copy::columns_to_js)
}

/// `transactionCopyIn(transaction, copy, abort, rows)`: a COPY FROM in the
/// transaction, holding its lock as a statement does until the COPY ends.
// [spec:pgorm:req:bindings.copy]
fn transaction_copy_in(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let (copy, rows) = copy::copy_from_rows_at(&mut cx, 1, 3)?;
    let copy = Box::new(copy);
    let abort = abort_argument(&mut cx, 2)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let (shared, link, release) = InState::channel(rows);
    let (opened, receive) = oneshot::channel();
    let command = Command::CopyIn {
        copy,
        link,
        opened,
        abort,
        release,
        lock,
    };
    copy::opened(&mut cx, shared, state.send(command), receive, CopyInHandle)
}

/// `transactionCopyOut(transaction, copy, abort, rows)`: a COPY TO in the
/// transaction, which refuses other work until the stream ends or closes.
// [spec:pgorm:req:bindings.copy]
fn transaction_copy_out(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let (copy, rows) = copy::copy_to_rows_at(&mut cx, 1, 3)?;
    let copy = Box::new(copy);
    let abort = abort_argument(&mut cx, 2)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let (shared, link, release) = OutState::channel(rows);
    let (opened, receive) = oneshot::channel();
    let command = Command::CopyOut {
        copy,
        link,
        opened,
        abort,
        release,
        lock,
    };
    copy::opened(&mut cx, shared, state.send(command), receive, CopyOutHandle)
}
