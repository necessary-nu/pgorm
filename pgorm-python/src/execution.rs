//! Borrowed execution context: all calls preserve the selected native executor.

mod target;
pub(crate) use target::{Executor, Target};

use crate::metrics::Metrics;
use pgorm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, Error, HookConnection, SqlText,
    metric::{InstrumentedConnection, InstrumentedTransaction},
};
use tokio_postgres::{
    Row, RowStream,
    types::{BorrowToSql, ToSql},
};

/// What an operation runs on: a connection or a transaction, plain or, from
/// an instrumented pool, pgorm's metrics wrapper around one.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Database<'a> {
    Connection(&'a DatabaseConnection),
    Transaction(&'a DatabaseTransaction<'a>),
    /// The connection a pool hook is about, while the hook runs.
    Hook(&'a HookConnection<'a>),
    MeteredConnection(&'a InstrumentedConnection<Metrics>),
    MeteredTransaction(&'a InstrumentedTransaction<'a, Metrics>),
}

macro_rules! native {
    ($db:expr, $method:ident $(, $arg:expr)*) => {
        match $db {
            Database::Connection(connection) => connection.$method($($arg),*).await,
            Database::Transaction(transaction) => transaction.$method($($arg),*).await,
            Database::Hook(connection) => connection.$method($($arg),*).await,
            Database::MeteredConnection(connection) => connection.$method($($arg),*).await,
            Database::MeteredTransaction(transaction) => transaction.$method($($arg),*).await,
        }
    };
}

// [spec:pgorm:req:bindings.transactions]
#[pgorm::entity::prelude::async_trait::async_trait]
impl ConnectionTrait for Database<'_> {
    async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        native!(self, execute, statement, params)
    }

    async fn execute_raw<T, P, I>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        native!(self, execute_raw, statement, params)
    }

    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        native!(self, query_one, statement, params)
    }

    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        native!(self, query_opt, statement, params)
    }

    async fn query_all<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        native!(self, query_all, statement, params)
    }

    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        native!(self, query_raw, statement, params)
    }

    async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        native!(self, batch_execute, sql)
    }
}
