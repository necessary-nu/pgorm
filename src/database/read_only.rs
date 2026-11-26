//! Connections that may only read: the pool whose sessions the server holds
//! read-only, its connections and transactions, and the views a connection
//! that may write lends as one that may not.

use std::{convert::identity, fmt, sync::Arc};

use pgorm_pool::Status;

use super::{
    connect_with,
    connection::RunsText,
    connection::private::Text,
    drive,
    sealed::{OpenReadOnly, ReadOnlyMode},
    simple_manager_config,
};
use crate::{
    Config, Connection, DatabaseConnection, DatabasePool, DatabaseTransaction, Error,
    IsolationLevel, MakeTlsConnect, ManagerConfig, NoTls, PoolBuilder, Socket, TlsConnect,
    TransactionError,
};

/// Implement [`Connection`] for a type by handing each statement to the
/// connection it wraps as a read: the wrapper's own bound has admitted the
/// statement, and the wrapper may run no more than the connection beneath.
macro_rules! forward_connection {
    ([$($generics:tt)*] $ty:ty, |$this:ident| $inner:expr) => {
        #[async_trait::async_trait]
        impl<$($generics)*> $crate::Connection for $ty {
            async fn execute<T>(
                &self,
                statement: &T,
                params: &[&(dyn ::pgorm_driver::types::ToSql + Sync)],
            ) -> Result<u64, $crate::Error>
            where
                T: ?Sized + $crate::SqlText + Sync,
                T::Kind: $crate::RunsOn<Self>,
            {
                let $this = self;
                $inner
                    .execute(&$crate::database::Kinded::read(statement), params)
                    .await
            }

            async fn execute_raw<T, P, I>(
                &self,
                statement: &T,
                params: I,
            ) -> Result<u64, $crate::Error>
            where
                T: ?Sized + $crate::SqlText + Sync,
                T::Kind: $crate::RunsOn<Self>,
                P: ::pgorm_driver::types::BorrowToSql,
                I: IntoIterator<Item = P> + Send,
                I::IntoIter: ExactSizeIterator,
            {
                let $this = self;
                $inner
                    .execute_raw(&$crate::database::Kinded::read(statement), params)
                    .await
            }

            async fn query_one<T>(
                &self,
                statement: &T,
                params: &[&(dyn ::pgorm_driver::types::ToSql + Sync)],
            ) -> Result<$crate::Row, $crate::Error>
            where
                T: ?Sized + $crate::SqlText + Sync,
                T::Kind: $crate::RunsOn<Self>,
            {
                let $this = self;
                $inner
                    .query_one(&$crate::database::Kinded::read(statement), params)
                    .await
            }

            async fn query_opt<T>(
                &self,
                statement: &T,
                params: &[&(dyn ::pgorm_driver::types::ToSql + Sync)],
            ) -> Result<Option<$crate::Row>, $crate::Error>
            where
                T: ?Sized + $crate::SqlText + Sync,
                T::Kind: $crate::RunsOn<Self>,
            {
                let $this = self;
                $inner
                    .query_opt(&$crate::database::Kinded::read(statement), params)
                    .await
            }

            async fn query_all<T>(
                &self,
                statement: &T,
                params: &[&(dyn ::pgorm_driver::types::ToSql + Sync)],
            ) -> Result<Vec<$crate::Row>, $crate::Error>
            where
                T: ?Sized + $crate::SqlText + Sync,
                T::Kind: $crate::RunsOn<Self>,
            {
                let $this = self;
                $inner
                    .query_all(&$crate::database::Kinded::read(statement), params)
                    .await
            }

            async fn query_raw<T, P, I>(
                &self,
                statement: &T,
                params: I,
            ) -> Result<$crate::RowStream, $crate::Error>
            where
                T: ?Sized + $crate::SqlText + Sync,
                T::Kind: $crate::RunsOn<Self>,
                P: ::pgorm_driver::types::BorrowToSql,
                I: IntoIterator<Item = P> + Send,
                I::IntoIter: ExactSizeIterator,
            {
                let $this = self;
                $inner
                    .query_raw(&$crate::database::Kinded::read(statement), params)
                    .await
            }
        }
    };
}

/// The session setting every connection of a [`ReadOnlyPool`] starts with,
/// as a startup option so that `DISCARD ALL` and `RESET ALL`, which return a
/// setting to its session default, return it to `on`.
const READ_ONLY_OPTION: &str = "-c default_transaction_read_only=on";

/// `config`, made to open sessions that may only read: on PostgreSQL every
/// transaction defaults to `READ ONLY`, the option appended after any the
/// caller gave so it is the one that holds; a native Turso file opens
/// read-only, and `:memory:` is refused.
// [spec:pgorm:req:read-only.pool]
// [spec:pgorm:req:read-only.turso]
fn read_only_config(mut config: Config) -> Result<Config, Error> {
    if config.is_turso() {
        config.turso_read_only()?;
        return Ok(config);
    }
    let options = match config.get_options() {
        Some(options) if !options.trim().is_empty() => format!("{options} {READ_ONLY_OPTION}"),
        _ => READ_ONLY_OPTION.to_owned(),
    };
    config.options(options);
    Ok(config)
}

/// A pool whose connections may only read: each implements [`Connection`]
/// and not [`ConnectionMut`](crate::ConnectionMut), so a write is a compile
/// error, and each session is opened with `default_transaction_read_only =
/// on`, so the server refuses what the types cannot see — a `SELECT` calling
/// a function that writes — with SQLSTATE `25006`
/// ([`Error::read_only_refusal`]).
///
/// Built by [`connect_read_only`] and its `_with` forms, as lazily as a
/// [`DatabasePool`]: nothing connects until the first [`get`](Self::get).
/// A replica, or a role granted only `SELECT`, fits it; the role is the
/// boundary, since text the server is handed through
/// [`unchecked`](ReadOnlyConnection::unchecked) can `SET` the session back.
// [spec:pgorm:req:read-only.pool]
#[derive(Debug, Clone)]
pub struct ReadOnlyPool(DatabasePool);

// [spec:pgorm:req:read-only.pool]
impl ReadOnlyPool {
    /// Take a connection from the pool, waiting for one to become available.
    pub async fn get(&self) -> Result<ReadOnlyConnection, Error> {
        self.0.get().await.map(ReadOnlyConnection)
    }

    /// The tag this pool was built with, or its generated `default-N` tag.
    pub fn tag(&self) -> Arc<String> {
        self.0.tag()
    }

    /// A snapshot of the pool's size and how many connections are in use.
    pub fn status(&self) -> Status {
        self.0.status()
    }

    /// Close the pool, as [`DatabasePool::close`] does.
    pub fn close(&self) {
        self.0.close();
    }

    /// Whether this pool has been explicitly closed.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    /// Drop every connection's cached prepared statements, as
    /// [`DatabasePool::clear_statement_caches`] does.
    pub fn clear_statement_caches(&self) {
        self.0.clear_statement_caches();
    }
}

/// A [`ReadOnlyPool`] over `config`, with the defaults [`connect`] uses.
///
/// Fallible where [`connect`] is not, because `config` can name a target
/// that cannot be read-only: a native `:memory:` database, which would be
/// empty and stay so.
///
/// [`connect`]: crate::connect
///
/// # Errors
///
/// Returns [`Error::Postgres`] for a native `:memory:` target, before
/// anything is opened.
// [spec:pgorm:req:read-only.pool]
pub fn connect_read_only(config: Config) -> Result<ReadOnlyPool, Error> {
    connect_read_only_with(config, NoTls, simple_manager_config(None), identity)
}

/// [`connect_read_only`] with the pool's sizing and timeouts shaped by
/// `build`, as [`connect_with_builder`](crate::connect_with_builder) shapes
/// a [`DatabasePool`].
///
/// # Errors
///
/// As [`connect_read_only_with`].
// [spec:pgorm:req:read-only.pool]
pub fn connect_read_only_with_builder(
    config: Config,
    build: impl FnOnce(PoolBuilder) -> PoolBuilder,
) -> Result<ReadOnlyPool, Error> {
    connect_read_only_with(config, NoTls, simple_manager_config(None), build)
}

/// A [`ReadOnlyPool`] over every knob [`connect_with`] takes: TLS, the
/// manager's configuration and the pool's shape.
///
/// # Errors
///
/// What [`connect_with`] returns, and [`Error::Postgres`] for a native
/// `:memory:` target, before anything is opened.
// [spec:pgorm:req:read-only.pool]
pub fn connect_read_only_with<T>(
    config: Config,
    tls: T,
    manager: ManagerConfig,
    build: impl FnOnce(PoolBuilder) -> PoolBuilder,
) -> Result<ReadOnlyPool, Error>
where
    T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
    T::Stream: Sync + Send,
    T::TlsConnect: Sync + Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    connect_with(read_only_config(config)?, tls, manager, build).map(ReadOnlyPool)
}

/// A connection checked out of a [`ReadOnlyPool`], returned to the pool on
/// drop. It implements [`Connection`] alone.
// [spec:pgorm:req:read-only.pool]
#[derive(Debug)]
pub struct ReadOnlyConnection(pub(crate) DatabaseConnection);

forward_connection!([] ReadOnlyConnection, |this| this.0);

impl ReadOnlyConnection {
    /// This connection, as one that runs SQL text of any kind: text known
    /// only at run time, and `sql!` statements that may write. The server
    /// is what refuses a write here (SQLSTATE `25006`), since the session
    /// is read-only; the returned [`Unchecked`] is still no
    /// [`ConnectionMut`](crate::ConnectionMut), so pgorm's own writes stay
    /// compile errors on it.
    // [spec:pgorm:req:read-only.unchecked]
    pub fn unchecked(&self) -> Unchecked<'_, Self> {
        Unchecked(self)
    }

    /// Run `f` inside `START TRANSACTION [ISOLATION LEVEL <isolation>] READ
    /// ONLY`, committing on `Ok` and rolling back on `Err`, as
    /// [`transaction`](crate::Transact::transaction) does.
    // [spec:pgorm:req:read-only.transactions]
    pub async fn read_only_transaction<T, E, F>(
        &mut self,
        isolation: Option<IsolationLevel>,
        f: F,
    ) -> Result<T, TransactionError<E>>
    where
        F: AsyncFnOnce(&mut ReadOnlyTransaction<'_>) -> Result<T, E>,
    {
        read_only(self, ReadOnlyMode::Isolation(isolation), f).await
    }

    /// Run `f` inside `START TRANSACTION ISOLATION LEVEL SERIALIZABLE, READ
    /// ONLY, DEFERRABLE`: opening it may wait until a snapshot is safe, after
    /// which no serialization failure can end it.
    // [spec:pgorm:req:read-only.transactions]
    pub async fn snapshot<T, E, F>(&mut self, f: F) -> Result<T, TransactionError<E>>
    where
        F: AsyncFnOnce(&mut ReadOnlyTransaction<'_>) -> Result<T, E>,
    {
        read_only(self, ReadOnlyMode::Snapshot, f).await
    }
}

impl OpenReadOnly for ReadOnlyConnection {
    async fn open_read_only(
        &mut self,
        mode: ReadOnlyMode,
    ) -> Result<ReadOnlyTransaction<'_>, Error> {
        self.0.open_read_only(mode).await
    }
}

/// `read_only_transaction` and `snapshot`'s body, on every connection one
/// opens on.
// [spec:pgorm:req:read-only.transactions]
pub(crate) async fn read_only<C, T, E, F>(
    parent: &mut C,
    mode: ReadOnlyMode,
    f: F,
) -> Result<T, TransactionError<E>>
where
    C: OpenReadOnly,
    F: AsyncFnOnce(&mut ReadOnlyTransaction<'_>) -> Result<T, E>,
{
    let txn = parent
        .open_read_only(mode)
        .await
        .map_err(TransactionError::Connection)?;
    drive(txn, f).await
}

/// The handle a read-only transaction's closure is given: a transaction
/// opened `READ ONLY` on a connection that may write, or any transaction on
/// a [`ReadOnlyConnection`]. It implements [`Connection`] alone, and its own
/// transactions are savepoints of the same kind.
// [spec:pgorm:req:read-only.transactions]
#[derive(Debug)]
pub struct ReadOnlyTransaction<'a>(pub(crate) DatabaseTransaction<'a>);

forward_connection!(['a] ReadOnlyTransaction<'a>, |this| this.0);

impl ReadOnlyTransaction<'_> {
    /// This transaction, as one that runs SQL text of any kind, the server
    /// refusing a write, as [`ReadOnlyConnection::unchecked`] does.
    // [spec:pgorm:req:read-only.unchecked]
    pub fn unchecked(&self) -> Unchecked<'_, Self> {
        Unchecked(self)
    }
}

/// A connection that may write, borrowed as one that may only read, from
/// [`ConnectionMut::read_only`](crate::ConnectionMut::read_only). It runs
/// every read, and no write and no text not known to read, by its type
/// alone: the session is the connection's own, and nothing asks the server
/// to refuse a write. Nothing turns it back into the connection it borrows.
// [spec:pgorm:req:read-only.downgrade]
pub struct ReadOnly<'a, C>(&'a C);

impl<'a, C> ReadOnly<'a, C> {
    pub(crate) fn new(connection: &'a C) -> Self {
        Self(connection)
    }
}

impl<C> Clone for ReadOnly<'_, C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C> Copy for ReadOnly<'_, C> {}

impl<C> fmt::Debug for ReadOnly<'_, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ReadOnly").finish_non_exhaustive()
    }
}

forward_connection!(['a, C: Connection] ReadOnly<'a, C>, |this| this.0);

/// A read-only connection or transaction that runs SQL text of any kind,
/// from [`ReadOnlyConnection::unchecked`] and
/// [`ReadOnlyTransaction::unchecked`]: the session or the transaction may
/// only read, so the server refuses a write with SQLSTATE `25006`.
///
/// It is no [`ConnectionMut`](crate::ConnectionMut), so pgorm's own writes
/// are compile errors on it as on the connection it borrows.
// [spec:pgorm:req:read-only.unchecked]
pub struct Unchecked<'a, C>(pub(crate) &'a C);

impl<C> Clone for Unchecked<'_, C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C> Copy for Unchecked<'_, C> {}

impl<C> fmt::Debug for Unchecked<'_, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Unchecked").finish_non_exhaustive()
    }
}

forward_connection!(['a, C: Connection] Unchecked<'a, C>, |this| this.0);

impl<C: Connection> Text for Unchecked<'_, C> {}

// [spec:pgorm:req:read-only.unchecked]
impl<C: Connection> RunsText for Unchecked<'_, C> {}

/// A pool of connections, as [`InstrumentedPool`](crate::metric::InstrumentedPool)
/// lends them: a [`DatabasePool`] lends connections that may write, a
/// [`ReadOnlyPool`] ones that may only read. The trait is sealed.
// [spec:pgorm:req:read-only.metrics]
pub trait ConnectionPool: pool::Sealed {
    /// The connection the pool lends.
    type Connection: Connection;

    /// Take a connection from the pool, waiting for one to become available.
    fn get(&self) -> impl Future<Output = Result<Self::Connection, Error>> + Send;

    /// The tag this pool was built with, or its generated `default-N` tag.
    fn tag(&self) -> Arc<String>;

    /// A snapshot of the pool's size and how many connections are in use.
    fn status(&self) -> Status;

    /// Drop every connection's cached prepared statements.
    fn clear_statement_caches(&self);
}

mod pool {
    pub trait Sealed {}
}

impl pool::Sealed for DatabasePool {}

impl ConnectionPool for DatabasePool {
    type Connection = DatabaseConnection;

    fn get(&self) -> impl Future<Output = Result<DatabaseConnection, Error>> + Send {
        DatabasePool::get(self)
    }

    fn tag(&self) -> Arc<String> {
        DatabasePool::tag(self)
    }

    fn status(&self) -> Status {
        DatabasePool::status(self)
    }

    fn clear_statement_caches(&self) {
        DatabasePool::clear_statement_caches(self);
    }
}

impl pool::Sealed for ReadOnlyPool {}

impl ConnectionPool for ReadOnlyPool {
    type Connection = ReadOnlyConnection;

    fn get(&self) -> impl Future<Output = Result<ReadOnlyConnection, Error>> + Send {
        ReadOnlyPool::get(self)
    }

    fn tag(&self) -> Arc<String> {
        ReadOnlyPool::tag(self)
    }

    fn status(&self) -> Status {
        ReadOnlyPool::status(self)
    }

    fn clear_statement_caches(&self) {
        ReadOnlyPool::clear_statement_caches(self);
    }
}
