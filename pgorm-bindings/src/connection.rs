//! A binding's pool and the connections it lends, each held by the binding's
//! handle in a slot an operation takes it out of.
//!
//! An operation takes a connection out of its slot for its length. Restored,
//! the connection goes back for the next; dropped unrestored, the operation
//! cut off or abandoned, the connection is discarded and its handle closed,
//! because nobody knows the state the server left it in. Closing a pool
//! cancels a token every lent connection, transaction and stream watches, so
//! what runs is cut short and every connection is released.
//!
//! The binding's own data about its pool, Python's event loop, rides along as
//! `O`, which nothing here reads.

use std::{
    fmt,
    future::Future,
    sync::{Arc, Mutex as SyncMutex, PoisonError, Weak},
    time::Duration,
};

use pgorm::{
    DatabaseConnection, DatabasePool,
    metric::{InstrumentedConnection, InstrumentedPool},
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

use crate::{Database, Dialect, Failure, Metrics, Redactions, classify, listen::ListenerState};

/// What a lent connection refuses once it, or its pool, has closed.
pub(crate) const POOL_CLOSED: &str = "pool is closed";
const CONNECTION_CLOSED: &str = "connection is closed";

/// A connection checked out of a pool: a plain one, or one an instrumented
/// pool lent, which reports what runs on it.
// [spec:pgorm:req:bindings.metrics]
#[derive(Debug)]
#[allow(missing_docs)]
pub enum Held {
    Plain(DatabaseConnection),
    Metered(InstrumentedConnection<Metrics>),
}

impl Held {
    /// The connection as the executor statements run on.
    pub fn database(&self) -> Database<'_> {
        match self {
            Self::Plain(connection) => Database::Connection(connection),
            Self::Metered(connection) => Database::InstrumentedConnection(connection),
        }
    }

    /// Close the connection rather than return it to its pool, as
    /// `DatabaseConnection::discard` does.
    pub fn discard(self) {
        match self {
            Self::Plain(connection) => connection.discard(),
            Self::Metered(connection) => connection.into_inner().discard(),
        }
    }
}

/// A pool as a binding holds it, shared by its handle, every instrumented
/// pool wrapping it, and every connection and listener it lent.
pub struct PoolState<O = ()> {
    /// pgorm's pool, which the binding's pool options built.
    pub pool: DatabasePool,
    /// The connection string's secrets, redacted from every failure.
    pub secrets: Redactions,
    /// Fired as the pool closes.
    pub closed: CancellationToken,
    /// The binding's own data about the pool.
    pub owner: O,
    dialect: Dialect,
    acquire_timeout: Duration,
    connections: SyncMutex<Vec<Weak<ConnectionState<O>>>>,
    listeners: SyncMutex<Vec<Weak<ListenerState>>>,
}

impl<O> fmt::Debug for PoolState<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoolState")
            .field("closed", &self.closed.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// What a pool's close leaves to release: every connection it had lent and
/// every listener it had opened.
#[derive(Debug)]
pub struct Closed<O> {
    connections: Vec<Arc<ConnectionState<O>>>,
    listeners: Vec<Arc<ListenerState>>,
}

impl<O: Send + Sync + 'static> Closed<O> {
    /// Finish once every connection and listener is released, an operation
    /// holding one having seen the close and let it go.
    pub async fn released(self) {
        for connection in self.connections {
            connection.released().await;
        }
        for listener in self.listeners {
            listener.release().await;
        }
    }
}

impl<O: Send + Sync + 'static> PoolState<O> {
    /// A pool lending its connections within `acquire_timeout`, its
    /// refusals worded for `dialect`.
    pub fn new(
        pool: DatabasePool,
        secrets: Redactions,
        owner: O,
        acquire_timeout: Duration,
        dialect: Dialect,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            secrets,
            closed: CancellationToken::new(),
            owner,
            dialect,
            acquire_timeout,
            connections: SyncMutex::new(Vec::new()),
            listeners: SyncMutex::new(Vec::new()),
        })
    }

    /// The dialect the pool's refusals are worded for.
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// `error` classified for this pool's binding, its secrets redacted.
    pub fn failure(&self, error: &pgorm::Error) -> Failure {
        classify(error, &self.secrets, self.dialect)
    }

    /// Refuse once the pool has closed.
    pub fn ensure_open(&self) -> Result<(), Failure> {
        if self.closed.is_cancelled() {
            Err(Failure::Lifecycle(POOL_CLOSED.to_owned()))
        } else {
            Ok(())
        }
    }

    /// A connection, within the acquire timeout, unless the pool closes
    /// first. Through `metered`, an instrumented pool's, the acquisition is
    /// reported, and the connection reports what runs on it.
    pub async fn acquire(
        &self,
        metered: Option<&InstrumentedPool<Metrics>>,
    ) -> Result<Held, Failure> {
        self.ensure_open()?;
        let acquire = async {
            match metered {
                Some(metered) => metered.get().await.map(Held::Metered),
                None => self.pool.get().await.map(Held::Plain),
            }
        };
        let connection = tokio::select! {
            () = self.closed.cancelled() => {
                return Err(Failure::Lifecycle(POOL_CLOSED.to_owned()));
            }
            acquired = tokio::time::timeout(self.acquire_timeout, acquire) => {
                self.ensure_open()?;
                acquired
                .map_err(|_| Failure::Timeout(format!(
                    "no connection became free within the pool's {}",
                    self.dialect.option("acquire_timeout")
                )))?
                .map_err(|error| self.failure(&error))?
            }
        };
        if self.closed.is_cancelled() {
            connection.discard();
            return Err(Failure::Lifecycle(POOL_CLOSED.to_owned()));
        }
        Ok(connection)
    }

    /// A connection for its borrower alone, recorded so that closing the
    /// pool closes it.
    // [spec:pgorm:req:bindings.connections+3]
    pub async fn lend(
        self: &Arc<Self>,
        metered: Option<&InstrumentedPool<Metrics>>,
    ) -> Result<Arc<ConnectionState<O>>, Failure> {
        let connection = self.acquire(metered).await?;
        let lent = Arc::new(ConnectionState {
            pool: self.clone(),
            closed: CancellationToken::new(),
            slot: Arc::new(Mutex::new(Some(connection))),
        });
        let mut registry = self
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.closed.is_cancelled() {
            drop(registry);
            lent.close_now();
            return Err(Failure::Lifecycle(POOL_CLOSED.to_owned()));
        }
        registry.retain(|item| item.strong_count() > 0);
        registry.push(Arc::downgrade(&lent));
        Ok(lent)
    }

    /// Run `work` on a connection the pool lends for it alone, returned to
    /// the pool once `work` finishes. Cut off, by `cancel` or by the pool
    /// closing, the connection is discarded rather than returned, its state
    /// unknown.
    pub async fn run<T, W>(
        &self,
        metered: Option<&InstrumentedPool<Metrics>>,
        cancel: &CancellationToken,
        work: W,
    ) -> Result<T, Failure>
    where
        W: AsyncFnOnce(&mut Held) -> Result<T, Failure>,
    {
        let mut connection = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled),
            acquired = self.acquire(metered) => acquired?,
        };
        let outcome = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(Failure::Cancelled),
            () = self.closed.cancelled() => Err(Failure::Lifecycle(POOL_CLOSED.to_owned())),
            outcome = work(&mut connection) => {
                drop(connection);
                return outcome;
            }
        };
        connection.discard();
        outcome
    }

    /// Record `listener` so that closing the pool closes it; one opened as
    /// the pool closed is closed here instead.
    // [spec:pgorm:req:bindings.listen+3]
    pub fn register_listener(&self, listener: &Arc<ListenerState>) -> Result<(), Failure> {
        let mut registry = self
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.closed.is_cancelled() {
            drop(registry);
            listener.close_now();
            return Err(Failure::Lifecycle(POOL_CLOSED.to_owned()));
        }
        registry.retain(|item| item.strong_count() > 0);
        registry.push(Arc::downgrade(listener));
        Ok(())
    }

    /// Refuse new work, close the idle connections and the listeners, and
    /// hand back what is left to release. Every lent connection, its
    /// operations, transactions and streams watch the pool's token, so
    /// closing it cuts short what runs.
    // [spec:pgorm:req:bindings.connections+3]
    pub fn close_now(&self) -> Closed<O> {
        self.closed.cancel();
        self.pool.close();
        let connections: Vec<_> = std::mem::take(
            &mut *self
                .connections
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
        .iter()
        .filter_map(Weak::upgrade)
        .collect();
        for connection in &connections {
            connection.close_now();
        }
        let listeners: Vec<_> = std::mem::take(
            &mut *self
                .listeners
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
        .iter()
        .filter_map(Weak::upgrade)
        .collect();
        for listener in &listeners {
            listener.close_now();
        }
        Closed {
            connections,
            listeners,
        }
    }
}

/// One connection a pool lent, in its slot while no operation holds it.
pub struct ConnectionState<O = ()> {
    /// The pool that lent it, whose close closes it.
    pub pool: Arc<PoolState<O>>,
    /// Fired as the connection closes.
    pub closed: CancellationToken,
    slot: Arc<Mutex<Option<Held>>>,
}

impl<O> fmt::Debug for ConnectionState<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionState")
            .field("closed", &self.closed.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl<O: Send + Sync + 'static> ConnectionState<O> {
    /// The state of no connection the pool lent: what a transaction
    /// pgorm-migration's runner holds and lends is served over, the runner
    /// keeping the connection.
    pub fn unheld(pool: Arc<PoolState<O>>) -> Arc<Self> {
        Arc::new(Self {
            pool,
            closed: CancellationToken::new(),
            slot: Arc::new(Mutex::new(None)),
        })
    }

    /// Close, returning an idle connection to the pool; one an operation
    /// holds is discarded when that operation sees the close.
    pub fn close_now(&self) {
        self.closed.cancel();
        if let Ok(mut slot) = self.slot.try_lock() {
            drop(slot.take());
        }
    }

    /// Close, discarding the connection rather than returning it: one an
    /// operation holds is discarded by that operation once it sees the close.
    // [spec:pgorm:req:bindings.connections+3]
    pub fn discard_now(&self) {
        self.closed.cancel();
        if let Ok(mut slot) = self.slot.try_lock()
            && let Some(connection) = slot.take()
        {
            connection.discard();
        }
    }

    /// The handle over the connection was dropped: an idle connection goes
    /// back to the pool, and one an operation holds goes back when that
    /// operation's last reference to it is gone, so dropping a handle never
    /// cuts work short.
    pub fn abandon(&self) {
        if let Ok(mut slot) = self.slot.try_lock()
            && let Some(connection) = slot.take()
        {
            self.closed.cancel();
            drop(connection);
        }
    }

    /// Finish once no operation holds the connection, returning it to the
    /// pool if it is still in its slot.
    pub async fn released(&self) {
        drop(self.slot.lock().await.take());
    }

    /// Finish once no operation holds the connection, discarding it if it is
    /// still in its slot.
    pub async fn discarded(&self) {
        if let Some(connection) = self.slot.lock().await.take() {
            connection.discard();
        }
    }

    /// Whether the connection or its pool has closed.
    pub fn is_closed(&self) -> bool {
        self.closed.is_cancelled() || self.pool.closed.is_cancelled()
    }

    /// Refuse once the connection or its pool has closed.
    pub fn ensure_open(&self) -> Result<(), Failure> {
        self.pool.ensure_open()?;
        if self.closed.is_cancelled() {
            Err(Failure::Lifecycle(CONNECTION_CLOSED.to_owned()))
        } else {
            Ok(())
        }
    }

    /// Resolves once the connection or its pool closes.
    pub async fn closing(&self) {
        tokio::select! {
            () = self.closed.cancelled() => {}
            () = self.pool.closed.cancelled() => {}
        }
    }

    /// Run `work` on the connection `operation` took from its slot, failing
    /// when the connection or its pool closes or `cancel` fires, the
    /// connection then discarded.
    pub async fn run<T, W>(
        mut operation: Operation<O>,
        cancel: &CancellationToken,
        work: W,
    ) -> Result<T, Failure>
    where
        W: AsyncFnOnce(&mut Held) -> Result<T, Failure>,
    {
        let state = operation.state.clone();
        let outcome = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled),
            () = state.closed.cancelled() => {
                return Err(Failure::Lifecycle(CONNECTION_CLOSED.to_owned()));
            }
            () = state.pool.closed.cancelled() => {
                return Err(Failure::Lifecycle(POOL_CLOSED.to_owned()));
            }
            outcome = work(operation.connection_mut()?) => outcome,
        };
        operation.restore();
        outcome
    }
}

/// A connection taken out of its slot for one operation. Restored, it goes
/// back; dropped unrestored, the operation aborted or cut off, it is
/// discarded and its handle closed, because its state is unknown.
// [spec:pgorm:req:bindings.connections+3]
// [spec:pgorm:req:bindings.cancellation]
pub struct Operation<O = ()> {
    slot: OwnedMutexGuard<Option<Held>>,
    connection: Option<Held>,
    /// The connection's state, which outlives the operation.
    pub state: Arc<ConnectionState<O>>,
}

impl<O> fmt::Debug for Operation<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Operation").finish_non_exhaustive()
    }
}

impl<O: Send + Sync + 'static> Operation<O> {
    /// Take the connection, refusing rather than waiting while another
    /// operation, a transaction or a stream holds it. Taken as the language
    /// makes the call, so operations are refused in the order it made them.
    pub fn begin(state: Arc<ConnectionState<O>>) -> Result<Self, Failure> {
        state.ensure_open()?;
        let mut slot = state.slot.clone().try_lock_owned().map_err(|_| {
            Failure::Lifecycle(
                "the connection is busy: an operation, a transaction or a stream holds it"
                    .to_owned(),
            )
        })?;
        let connection = slot
            .take()
            .ok_or_else(|| Failure::Lifecycle(CONNECTION_CLOSED.to_owned()))?;
        Ok(Self {
            slot,
            connection: Some(connection),
            state,
        })
    }

    /// The connection as the executor statements run on.
    pub fn connection(&self) -> Result<Database<'_>, Failure> {
        self.connection
            .as_ref()
            .map(Held::database)
            .ok_or_else(|| Failure::Internal("the operation has no connection".to_owned()))
    }

    /// The connection, for a transaction or a COPY to borrow.
    pub fn connection_mut(&mut self) -> Result<&mut Held, Failure> {
        self.connection
            .as_mut()
            .ok_or_else(|| Failure::Internal("the operation has no connection".to_owned()))
    }

    /// Put the connection back for the next operation, or return it to the
    /// pool if its handle closed meanwhile.
    pub fn restore(mut self) {
        if self.state.ensure_open().is_ok() {
            *self.slot = self.connection.take();
        }
    }
}

impl<O> Drop for Operation<O> {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            self.state.closed.cancel();
            connection.discard();
        }
    }
}

/// `future`, unless `cancel` fires first.
pub async fn cancellable<T>(
    cancel: &CancellationToken,
    future: impl Future<Output = Result<T, Failure>>,
) -> Result<T, Failure> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(Failure::Cancelled),
        outcome = future => outcome,
    }
}
