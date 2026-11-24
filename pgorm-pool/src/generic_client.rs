//! This is a 1:1 copy of the `pgorm_driver::GenericClient`
//! trait as of `tokio-postgres 0.7.7` with two changes:
//! - The `client()` method is not available.
//! - The `prepare_cached()` and `prepare_typed_cached()` are
//!   added.
use pgorm_driver::RowStream;
use pgorm_driver::types::{BorrowToSql, ToSql, Type};
use pgorm_driver::{Error, Row, Statement, ToStatement};

use async_trait::async_trait;

use crate::{Client, ClientWrapper, Transaction};

mod private {
    pub trait Sealed {}
}

/// A trait allowing abstraction over connections and transactions.
///
/// This trait is "sealed", and cannot be implemented outside of this crate.
// [spec:pgorm:def:conn.pool.generic-client+1]
#[async_trait]
pub trait GenericClient: Sync + private::Sealed {
    /// Like `Client::execute`.
    async fn execute<T>(&self, query: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement + Sync + Send;

    /// Like `Client::execute_raw`.
    async fn execute_raw<P, I, T>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Sync + Send,
        I::IntoIter: ExactSizeIterator;

    /// Like `Client::query`.
    async fn query<T>(&self, query: &T, params: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + ToStatement + Sync + Send;

    /// Like `Client::query_one`.
    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + ToStatement + Sync + Send;

    /// Like `Client::query_opt`.
    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + ToStatement + Sync + Send;

    /// Like `Client::query_raw`.
    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Sync + Send,
        I::IntoIter: ExactSizeIterator;

    /// Like `Client::prepare`.
    async fn prepare(&self, query: &str) -> Result<Statement, Error>;

    /// Like `Client::prepare_typed`.
    async fn prepare_typed(
        &self,
        query: &str,
        parameter_types: &[Type],
    ) -> Result<Statement, Error>;

    /// Like [`Client::prepare_cached`].
    async fn prepare_cached(&self, query: &str) -> Result<Statement, Error>;

    /// Like [`Client::prepare_typed_cached`]
    async fn prepare_typed_cached(&self, query: &str, types: &[Type]) -> Result<Statement, Error>;

    /// Like `Client::transaction`.
    async fn transaction(&mut self) -> Result<Transaction<'_>, Error>;

    /// Like `Client::batch_execute`.
    async fn batch_execute(&self, query: &str) -> Result<(), Error>;
}

impl private::Sealed for Client {}

// [spec:pgorm:def:conn.pool.generic-client+1]    Client delegation
#[async_trait]
impl GenericClient for Client {
    async fn execute<T>(&self, query: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
    {
        pgorm_driver::Client::execute(self, query, params).await
    }

    async fn execute_raw<P, I, T>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Sync + Send,
        I::IntoIter: ExactSizeIterator,
    {
        pgorm_driver::Client::execute_raw(self, statement, params).await
    }

    async fn query<T>(&self, query: &T, params: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
    {
        pgorm_driver::Client::query(self, query, params).await
    }

    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
    {
        pgorm_driver::Client::query_one(self, statement, params).await
    }

    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
    {
        pgorm_driver::Client::query_opt(self, statement, params).await
    }

    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Sync + Send,
        I::IntoIter: ExactSizeIterator,
    {
        pgorm_driver::Client::query_raw(self, statement, params).await
    }

    async fn prepare(&self, query: &str) -> Result<Statement, Error> {
        pgorm_driver::Client::prepare(self, query).await
    }

    async fn prepare_typed(
        &self,
        query: &str,
        parameter_types: &[Type],
    ) -> Result<Statement, Error> {
        pgorm_driver::Client::prepare_typed(self, query, parameter_types).await
    }

    async fn prepare_cached(&self, query: &str) -> Result<Statement, Error> {
        ClientWrapper::prepare_cached(self, query).await
    }

    async fn prepare_typed_cached(&self, query: &str, types: &[Type]) -> Result<Statement, Error> {
        ClientWrapper::prepare_typed_cached(self, query, types).await
    }

    async fn transaction(&mut self) -> Result<Transaction<'_>, Error> {
        ClientWrapper::transaction(self).await
    }

    async fn batch_execute(&self, query: &str) -> Result<(), Error> {
        pgorm_driver::Client::batch_execute(self, query).await
    }
}

impl private::Sealed for Transaction<'_> {}

// [spec:pgorm:def:conn.pool.generic-client+1]    Transaction delegation
#[async_trait]
#[allow(clippy::needless_lifetimes)]
impl GenericClient for Transaction<'_> {
    async fn execute<T>(&self, query: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
    {
        pgorm_driver::Transaction::execute(&self.txn, query, params).await
    }

    async fn execute_raw<P, I, T>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Sync + Send,
        I::IntoIter: ExactSizeIterator,
    {
        pgorm_driver::Transaction::execute_raw(&self.txn, statement, params).await
    }

    async fn query<T>(&self, query: &T, params: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
    {
        pgorm_driver::Transaction::query(&self.txn, query, params).await
    }

    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
    {
        pgorm_driver::Transaction::query_one(&self.txn, statement, params).await
    }

    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
    {
        pgorm_driver::Transaction::query_opt(&self.txn, statement, params).await
    }

    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + ToStatement + Sync + Send,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Sync + Send,
        I::IntoIter: ExactSizeIterator,
    {
        pgorm_driver::Transaction::query_raw(&self.txn, statement, params).await
    }

    async fn prepare(&self, query: &str) -> Result<Statement, Error> {
        pgorm_driver::Transaction::prepare(&self.txn, query).await
    }

    async fn prepare_typed(
        &self,
        query: &str,
        parameter_types: &[Type],
    ) -> Result<Statement, Error> {
        pgorm_driver::Transaction::prepare_typed(&self.txn, query, parameter_types).await
    }

    async fn prepare_cached(&self, query: &str) -> Result<Statement, Error> {
        Transaction::prepare_cached(self, query).await
    }

    async fn prepare_typed_cached(&self, query: &str, types: &[Type]) -> Result<Statement, Error> {
        Transaction::prepare_typed_cached(self, query, types).await
    }

    #[allow(clippy::needless_lifetimes)]
    async fn transaction<'a>(&'a mut self) -> Result<Transaction<'a>, Error> {
        Transaction::transaction(self).await
    }

    async fn batch_execute(&self, query: &str) -> Result<(), Error> {
        pgorm_driver::Transaction::batch_execute(&self.txn, query).await
    }
}
