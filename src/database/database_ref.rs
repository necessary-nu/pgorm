use std::fmt;

use crate::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, Error, SqlText,
    metric::{InstrumentedConnection, InstrumentedTransaction, MetricsCollector, NoOpMetrics},
};
use tokio_postgres::{
    Row, RowStream,
    types::{BorrowToSql, ToSql},
};

/// A borrowed executor of any kind: a connection, or a transaction or
/// savepoint on one, plain or wrapped by the metrics layer's collector `M`.
///
/// Code that chooses at run time where a statement runs — a language binding
/// holding whichever handle its caller passed — reaches pgorm's generic
/// terminals through this one [`ConnectionTrait`] type, each method the
/// borrowed executor's own.
///
/// ```
/// # async fn count(db: &pgorm::DatabaseConnection) -> Result<(), pgorm::Error> {
/// use pgorm::{ConnectionTrait, DatabaseRef};
///
/// let either: DatabaseRef<'_> = DatabaseRef::from(db);
/// either.execute("SELECT 1", &[]).await?;
/// # Ok(())
/// # }
/// ```
// [spec:pgorm:def:conn.database-ref]
pub enum DatabaseRef<'a, M: MetricsCollector = NoOpMetrics> {
    /// A connection, statements running on it outside any transaction.
    Connection(&'a DatabaseConnection),
    /// A transaction or savepoint, statements running inside it.
    Transaction(&'a DatabaseTransaction<'a>),
    /// A connection the metrics layer records through `M`.
    InstrumentedConnection(&'a InstrumentedConnection<M>),
    /// A transaction or savepoint the metrics layer records through `M`.
    InstrumentedTransaction(&'a InstrumentedTransaction<'a, M>),
}

impl<M: MetricsCollector> Clone for DatabaseRef<'_, M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M: MetricsCollector> Copy for DatabaseRef<'_, M> {}

impl<M: MetricsCollector> fmt::Debug for DatabaseRef<'_, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Connection(_) => "DatabaseRef::Connection",
            Self::Transaction(_) => "DatabaseRef::Transaction",
            Self::InstrumentedConnection(_) => "DatabaseRef::InstrumentedConnection",
            Self::InstrumentedTransaction(_) => "DatabaseRef::InstrumentedTransaction",
        })
    }
}

impl<'a, M: MetricsCollector> From<&'a DatabaseConnection> for DatabaseRef<'a, M> {
    fn from(connection: &'a DatabaseConnection) -> Self {
        Self::Connection(connection)
    }
}

impl<'a, M: MetricsCollector> From<&'a DatabaseTransaction<'a>> for DatabaseRef<'a, M> {
    fn from(transaction: &'a DatabaseTransaction<'a>) -> Self {
        Self::Transaction(transaction)
    }
}

impl<'a, M: MetricsCollector> From<&'a InstrumentedConnection<M>> for DatabaseRef<'a, M> {
    fn from(connection: &'a InstrumentedConnection<M>) -> Self {
        Self::InstrumentedConnection(connection)
    }
}

impl<'a, M: MetricsCollector> From<&'a InstrumentedTransaction<'a, M>> for DatabaseRef<'a, M> {
    fn from(transaction: &'a InstrumentedTransaction<'a, M>) -> Self {
        Self::InstrumentedTransaction(transaction)
    }
}

macro_rules! delegate {
    ($db:expr, $method:ident $(, $arg:expr)*) => {
        match $db {
            DatabaseRef::Connection(connection) => connection.$method($($arg),*).await,
            DatabaseRef::Transaction(transaction) => transaction.$method($($arg),*).await,
            DatabaseRef::InstrumentedConnection(connection) => connection.$method($($arg),*).await,
            DatabaseRef::InstrumentedTransaction(transaction) => transaction.$method($($arg),*).await,
        }
    };
}

// [spec:pgorm:def:conn.database-ref]
#[async_trait::async_trait]
impl<M: MetricsCollector> ConnectionTrait for DatabaseRef<'_, M> {
    async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        delegate!(self, execute, statement, params)
    }

    async fn execute_raw<T, P, I>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        delegate!(self, execute_raw, statement, params)
    }

    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        delegate!(self, query_one, statement, params)
    }

    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        delegate!(self, query_opt, statement, params)
    }

    async fn query_all<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        delegate!(self, query_all, statement, params)
    }

    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        delegate!(self, query_raw, statement, params)
    }

    async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        delegate!(self, batch_execute, sql)
    }
}
