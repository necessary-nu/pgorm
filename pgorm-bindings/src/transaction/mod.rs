//! Transactions and savepoints as a binding's handles reach them.
//!
//! pgorm opens a transaction only around a closure, whose
//! `DatabaseTransaction` borrows its connection, and a savepoint's its parent,
//! exclusively. That borrow cannot cross into the language, so a task on the
//! runtime serves each frame's commands inside its closure, nested savepoint
//! frames inside their parent's, and the language's handles send it owned
//! [`Job`]s. While a savepoint is open its parent's frame is suspended inside
//! it, holding the parent's lock: the parent refuses a statement, a commit, a
//! rollback or another savepoint until the child finishes, and refuses a
//! second request while one runs, rather than queue or race it.

mod frame;
mod lend;
mod retry;

use std::{
    fmt,
    sync::{Arc, Weak},
};

use futures_util::future::BoxFuture;
use pgorm::{DatabaseTransaction, metric::InstrumentedTransaction};
use tokio::sync::{Mutex, OwnedMutexGuard, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub use frame::begin;
pub use lend::{Lent, lend};
pub use retry::Retrying;

use crate::{Database, Failure, Metrics, connection::ConnectionState};

const ENDED: &str = "transaction owner has closed";

/// The transaction a frame serves: a plain one, or pgorm's metrics wrapper
/// around one on an instrumented pool's connection, whose savepoints are
/// instrumented too.
// [spec:pgorm:req:bindings.metrics]
#[derive(Debug)]
#[allow(missing_docs)]
pub enum Tx<'a, 't> {
    Plain(&'a mut DatabaseTransaction<'t>),
    Metered(&'a mut InstrumentedTransaction<'t, Metrics>),
}

impl Tx<'_, '_> {
    /// The transaction as the executor statements run on.
    pub fn database(&self) -> Database<'_> {
        match self {
            Self::Plain(tx) => Database::Transaction(tx),
            Self::Metered(tx) => Database::InstrumentedTransaction(tx),
        }
    }
}

/// Whether a frame can go on serving once a job has run in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// The transaction is in a state its frame knows.
    Kept,
    /// The job's effect is unknown, its caller gone or its COPY's connection
    /// discarded, so the frame gives its connection up.
    Lost,
}

/// Work a handle sends its frame: called once, with the frame's
/// transaction, on the runtime. It answers its caller itself.
pub type Job = Box<dyn for<'b> FnOnce(&'b mut Tx<'_, '_>) -> BoxFuture<'b, Flow> + Send>;

/// The reply a frame's end waits on: `true` once pgorm has opened the next
/// attempt of a retrying transaction, `false` once the transaction has ended.
type Attempted = oneshot::Sender<Result<bool, Failure>>;

/// The wait for a frame to open.
type Opened = oneshot::Sender<Result<(), Failure>>;

enum Command<O> {
    Work(Job),
    Finish {
        commit: bool,
        /// For a rollback: whether the failure that ended the frame is one a
        /// retrying transaction runs again after.
        retryable: bool,
        reply: Attempted,
    },
    Begin {
        child: Arc<State<O>>,
        receiver: mpsc::UnboundedReceiver<Command<O>>,
        opened: Opened,
        parent_lock: OwnedMutexGuard<()>,
    },
}

/// What every frame of one transaction shares: `aborted` ends the whole
/// transaction and discards its connection, `finished` fires once its task
/// has ended.
#[derive(Debug, Default)]
struct Root {
    aborted: CancellationToken,
    finished: CancellationToken,
}

/// One transaction or savepoint, as its handle reaches it.
pub struct State<O = ()> {
    connection: Arc<ConnectionState<O>>,
    commands: mpsc::UnboundedSender<Command<O>>,
    busy: Arc<Mutex<()>>,
    abandoned: CancellationToken,
    finished: CancellationToken,
    root: Arc<Root>,
    released: CancellationToken,
    parent: Option<Weak<State<O>>>,
    /// Whether pgorm-migration's runner opened this transaction and lends
    /// it, so no connection of the binding's holds it.
    lent: bool,
}

impl<O> fmt::Debug for State<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State")
            .field("finished", &self.finished.is_cancelled())
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
/// completes, its caller gone, it ends the whole transaction, whose
/// connection is then discarded.
// [spec:pgorm:req:bindings.cancellation]
#[derive(Debug)]
pub struct Pending {
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

    /// The request finished with a known outcome.
    pub fn complete(&mut self) {
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

/// A transaction a stream or a COPY in progress holds: its frame's lock, and
/// the request whose outcome is unknown until the stream's last row or the
/// COPY's end. Dropped without `complete`, the whole transaction is
/// discarded.
#[derive(Debug)]
pub struct Lease {
    _lock: OwnedMutexGuard<()>,
    pending: Pending,
}

impl Lease {
    /// What the lease held finished with a known outcome.
    pub fn complete(mut self) {
        self.pending.complete();
    }
}

impl<O: Send + Sync + 'static> State<O> {
    fn channel(
        connection: Arc<ConnectionState<O>>,
        parent: Option<&Arc<Self>>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Command<O>>) {
        let root = parent.map_or_else(Arc::default, |parent| parent.root.clone());
        Self::rooted(connection, parent, root, false)
    }

    fn rooted(
        connection: Arc<ConnectionState<O>>,
        parent: Option<&Arc<Self>>,
        root: Arc<Root>,
        lent: bool,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Command<O>>) {
        let (commands, receiver) = mpsc::unbounded_channel();
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
                parent: parent.map(Arc::downgrade),
                lent,
            }),
            receiver,
        )
    }

    /// The connection the transaction holds.
    pub fn connection(&self) -> &Arc<ConnectionState<O>> {
        &self.connection
    }

    /// The transaction a savepoint is of, or `None` for a transaction.
    pub fn parent(&self) -> Option<Arc<Self>> {
        self.parent.as_ref().and_then(Weak::upgrade)
    }

    /// Whether this is a transaction pgorm-migration's runner lent, which no
    /// connection of the binding's opened.
    pub fn is_lent(&self) -> bool {
        self.lent
    }

    /// Whether this frame is a savepoint's rather than a transaction's.
    pub fn is_savepoint(&self) -> bool {
        self.parent.is_some()
    }

    /// Whether this frame belongs to a savepoint opened in `parent`.
    pub fn is_child_of(&self, parent: &Arc<Self>) -> bool {
        self.parent().is_some_and(|own| Arc::ptr_eq(&own, parent))
    }

    /// Whether the frame can take no more requests: ended, abandoned, its
    /// transaction aborted or its connection closed.
    pub fn closed(&self) -> bool {
        self.finished.is_cancelled()
            || self.abandoned.is_cancelled()
            || self.root.aborted.is_cancelled()
            || self.root.finished.is_cancelled()
            || self.connection.is_closed()
    }

    /// Refuse once the frame is closed.
    pub fn ensure_open(&self) -> Result<(), Failure> {
        if self.closed() {
            Err(Failure::Lifecycle("transaction is closed".to_owned()))
        } else {
            Ok(())
        }
    }

    /// The frame's lock, refused rather than waited for when a request or an
    /// open savepoint holds it. Taken as the language makes each request, so
    /// requests are refused in the order it made them.
    // [spec:pgorm:req:bindings.transactions+2]
    pub fn lock(&self) -> Result<OwnedMutexGuard<()>, Failure> {
        self.ensure_open()?;
        self.busy.clone().try_lock_owned().map_err(|_| {
            Failure::Lifecycle(
                "the transaction is busy: a statement runs on it, or a savepoint of it is open"
                    .to_owned(),
            )
        })
    }

    fn send(&self, command: Command<O>) -> Result<(), Failure> {
        self.commands
            .send(command)
            .map_err(|_| Failure::Lifecycle(ENDED.to_owned()))
    }

    /// Send `job` to the frame, the lock held for it however the job holds
    /// it: what the job answers, it answers itself.
    pub fn send_job(&self, job: Job) -> Result<(), Failure> {
        self.send(Command::Work(job))
    }

    /// A request whose outcome is unknown until the caller completes it:
    /// dropped before, the whole transaction is discarded.
    pub fn pending(&self) -> Pending {
        Pending::new(self.root.aborted.clone())
    }

    /// A lease over `lock` and `pending`, for a stream or a COPY that holds
    /// the frame until it ends.
    pub fn lease(lock: OwnedMutexGuard<()>, pending: Pending) -> Lease {
        Lease {
            _lock: lock,
            pending,
        }
    }

    /// Run `run` on the frame's transaction and wait for what it gives,
    /// holding the lock taken as the request was made. A failure it gives
    /// leaves the transaction as any failed statement does.
    pub async fn request<T, F>(
        &self,
        lock: OwnedMutexGuard<()>,
        run: F,
        cancel: &CancellationToken,
    ) -> Result<T, Failure>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(Database<'a>) -> BoxFuture<'a, Result<T, Failure>> + Send + 'static,
    {
        self.request_on(lock, move |tx| run(tx.database()), cancel)
            .await
    }

    /// [`State::request`] of what needs the transaction itself, a COPY's
    /// description.
    pub async fn request_on<T, F>(
        &self,
        lock: OwnedMutexGuard<()>,
        run: F,
        cancel: &CancellationToken,
    ) -> Result<T, Failure>
    where
        T: Send + 'static,
        F: for<'b> FnOnce(&'b mut Tx<'_, '_>) -> BoxFuture<'b, Result<T, Failure>> + Send + 'static,
    {
        let (outcome, mut pending) = self.exchange_on(run, cancel).await?;
        drop(lock);
        pending.complete();
        outcome
    }

    /// Run `run` on the frame's transaction and wait for what it gives, which
    /// comes with the request still pending: until the caller completes it,
    /// the transaction is discarded if the caller is dropped.
    pub async fn exchange<T, F>(
        &self,
        run: F,
        cancel: &CancellationToken,
    ) -> Result<(T, Pending), Failure>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(Database<'a>) -> BoxFuture<'a, T> + Send + 'static,
    {
        self.exchange_on(move |tx| run(tx.database()), cancel).await
    }

    async fn exchange_on<T, F>(
        &self,
        run: F,
        cancel: &CancellationToken,
    ) -> Result<(T, Pending), Failure>
    where
        T: Send + 'static,
        F: for<'b> FnOnce(&'b mut Tx<'_, '_>) -> BoxFuture<'b, T> + Send + 'static,
    {
        let (reply, receive) = oneshot::channel();
        let pending = self.pending();
        self.send_job(Box::new(move |tx| {
            Box::pin(async move {
                let outcome = run(tx).await;
                match reply.send(outcome) {
                    Ok(()) => Flow::Kept,
                    Err(_) => Flow::Lost,
                }
            })
        }))?;
        let outcome = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled),
            received = receive => received.map_err(|_| Failure::Lifecycle(ENDED.to_owned()))?,
        };
        Ok((outcome, pending))
    }

    /// Commit or roll back the frame, finishing once its end has released
    /// its parent.
    pub async fn finish(&self, lock: OwnedMutexGuard<()>, commit: bool) -> Result<(), Failure> {
        self.finish_attempt(lock, commit, false).await.map(|_| ())
    }

    /// End one attempt of a retrying transaction, `retryable` saying whether
    /// the failure that ended it is one to run it again after: whether pgorm
    /// opened the next attempt.
    // [spec:pgorm:req:bindings.transaction-retry]
    pub async fn finish_attempt(
        &self,
        _lock: OwnedMutexGuard<()>,
        commit: bool,
        retryable: bool,
    ) -> Result<bool, Failure> {
        let (reply, receive) = oneshot::channel();
        let mut pending = self.pending();
        self.send(Command::Finish {
            commit,
            retryable,
            reply,
        })?;
        let outcome = receive
            .await
            .map_err(|_| Failure::Lifecycle(ENDED.to_owned()))?;
        pending.complete();
        let again = outcome?;
        if !again {
            self.released.cancelled().await;
        }
        Ok(again)
    }

    /// A savepoint, which holds this frame's lock until it finishes.
    pub async fn begin(
        self: &Arc<Self>,
        parent_lock: OwnedMutexGuard<()>,
        cancel: &CancellationToken,
    ) -> Result<Arc<Self>, Failure> {
        let (child, receiver) = Self::channel(self.connection.clone(), Some(self));
        let (opened, receive) = oneshot::channel();
        let mut pending = self.pending();
        self.send(Command::Begin {
            child: child.clone(),
            receiver,
            opened,
            parent_lock,
        })?;
        tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled),
            opened = receive => opened.map_err(|_| Failure::Lifecycle(ENDED.to_owned()))??,
        }
        pending.complete();
        Ok(child)
    }

    /// The handle over the frame was dropped unfinished: the frame rolls
    /// back as a backstop.
    pub fn abandon(&self) {
        self.abandoned.cancel();
    }

    /// End the whole transaction, discarding its connection, whichever
    /// frame of it is serving.
    pub fn abort(&self) {
        self.root.aborted.cancel();
    }

    /// Finish once the transaction's task has ended.
    pub async fn finished(&self) {
        self.root.finished.cancelled().await;
    }

    /// Finish once the frame has released its parent, or the transaction's
    /// task has ended.
    pub async fn wait_closed(&self) {
        tokio::select! {
            () = self.released.cancelled() => {}
            () = self.root.finished.cancelled() => {}
        }
    }

    /// Whether a COPY in this frame, ended with its connection discarded,
    /// ended the whole transaction, which a discarded connection does, as an
    /// aborted statement does.
    pub fn copy_ended(&self, discarded: bool) -> Flow {
        if discarded {
            self.root.aborted.cancel();
            Flow::Lost
        } else {
            Flow::Kept
        }
    }

    /// A token that fires once this frame or its transaction has ended, by
    /// any route.
    pub fn ended(&self) -> CancellationToken {
        let ended = CancellationToken::new();
        let fire = ended.clone();
        let (aborted, finished, abandoned, own) = (
            self.root.aborted.clone(),
            self.root.finished.clone(),
            self.abandoned.clone(),
            self.finished.clone(),
        );
        tokio::spawn(async move {
            tokio::select! {
                () = aborted.cancelled() => {}
                () = finished.cancelled() => {}
                () = abandoned.cancelled() => {}
                () = own.cancelled() => {}
            }
            fire.cancel();
        });
        ended
    }

    /// The tokens that end a COPY in this frame from outside it: the frame
    /// abandoned, or its transaction aborted.
    pub fn stops(&self) -> [CancellationToken; 2] {
        [self.abandoned.clone(), self.root.aborted.clone()]
    }
}
