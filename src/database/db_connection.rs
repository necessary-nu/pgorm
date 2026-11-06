use std::{collections::BTreeMap, sync::Arc};

use super::{
    copy::refuse_copy_statement,
    drive, rolled_back,
    sealed::{Close, Handle, TOKEN},
};
use crate::{ConnectionTrait, RetryableError, SqlText, TransactionError, error::*};
use deadpool::Status;
use pgorm_pool::{Object, Pool, Transaction};
use tokio_postgres::{
    IsolationLevel, RowStream,
    error::SqlState,
    types::{BorrowToSql, ToSql},
};

/// Run a statement against the connection's statement cache, re-preparing it
/// and running it again once if PostgreSQL rejects the cached plan.
///
/// `$owner` is the pooled client holding the cache. Every statement carries its
/// SQL text (`conn.sql-text`), so every one is resolved through the cache and
/// the same text is parsed once per connection rather than once per call; there
/// is no second path.
///
/// `$call` is expanded twice, once per attempt, so it must be replayable: only
/// the methods whose parameters are a reusable `&[&dyn ToSql]` slice can use
/// this macro, and only outside a transaction — see [`cached_evict`] for why an
/// open transaction takes the other one.
// [spec:pgorm:req:conn.pool.statement-cache.invalidate+4]    evict, re-prepare, retry once
macro_rules! cached_retry {
    ($owner:expr, $statement:expr, |$prepared:ident| $call:expr) => {{
        let owner = $owner;
        let sql = $statement.sql_text();
        refuse_copy_statement(sql)?;
        let cached = owner.prepare_cached(sql).await?;
        let outcome = {
            let $prepared = &cached;
            $call
        };

        match outcome {
            Err(error) if is_stale_cached_plan(&error) => {
                drop(owner.statement_cache.remove(sql, &[]));
                let reprepared = owner.prepare_cached(sql).await?;
                let $prepared = &reprepared;
                Ok($call?)
            }
            outcome => Ok(outcome?),
        }
    }};
}

/// [`cached_retry`] without the second attempt: a rejected plan is evicted, so
/// the next call re-prepares, and the error is returned exactly as it came.
///
/// Two callers need this, for unrelated reasons.
///
/// `execute_raw` and `query_raw` take an `IntoIterator` consumed by the first
/// attempt, which is not `Clone` and cannot be held across a retry without a
/// `Send` bound `ConnectionTrait` does not carry, so the recovery is one call
/// later rather than absent.
///
/// Every method on a [`DatabaseTransaction`] needs it because the rejected plan
/// has already aborted the transaction. The re-prepare would fail with `25P02`,
/// *current transaction is aborted*, and returning that would replace the
/// caller's actionable error with a consequence of pgorm's own retry. Recovery
/// inside a transaction belongs to whoever owns the transaction's boundary: the
/// eviction guarantees their next attempt prepares afresh.
// [spec:pgorm:req:conn.pool.statement-cache.invalidate+4]    evict without a retry
macro_rules! cached_evict {
    ($owner:expr, $statement:expr, |$prepared:ident| $call:expr) => {{
        let owner = $owner;
        let sql = $statement.sql_text();
        refuse_copy_statement(sql)?;
        let cached = owner.prepare_cached(sql).await?;
        let outcome = {
            let $prepared = &cached;
            $call
        };

        if let Err(error) = &outcome
            && is_stale_cached_plan(error)
        {
            drop(owner.statement_cache.remove(sql, &[]));
            tracing::debug!(sql, "cached plan rejected; evicted without a retry");
        }

        Ok(outcome?)
    }};
}

/// Whether PostgreSQL refused to run a plan because the statement it was built
/// for no longer produces the result it was described as producing.
///
/// The server reports it as SQLSTATE `0A000`, which is also its generic
/// *feature not supported*. Nothing distinguishes the two but the message text,
/// which is localized, so a statement rejected on its own merits is treated the
/// same way: outside a transaction it is retried once and fails identically, at
/// the cost of one round trip on a call that was already failing, and inside
/// one it costs an eviction that makes the next call parse the text again.
// [spec:pgorm:req:conn.pool.statement-cache.invalidate+4]    the SQLSTATE that means "re-prepare"
fn is_stale_cached_plan(error: &tokio_postgres::Error) -> bool {
    error.code() == Some(&SqlState::FEATURE_NOT_SUPPORTED)
}

/// Handle a database connection depending on the backend enabled by the feature
/// flags. This creates a database pool.
// [spec:pgorm:req:conn.pool.no-conn-trait]
#[derive(Debug, Clone)]
#[repr(transparent)]
#[doc(alias = "DbConn")]
pub struct DatabasePool(pub(crate) Pool);

/// A set of [`DatabasePool`]s keyed by their tags, as built by
/// [`connect_multi_with_builder`](crate::connect_multi_with_builder).
///
/// Which pool a statement runs on is the caller's choice, made by tag:
/// nothing here balances load across the pools or falls back from one to
/// another.
// [spec:pgorm:sem:conn.pool.multi+3]    tag-keyed accessor surface
#[derive(Debug, Clone)]
pub struct DatabaseMultiPool(pub(crate) BTreeMap<String, DatabasePool>);

impl DatabaseMultiPool {
    /// The pool tagged `tag`, or `None` if no pool carries that tag.
    pub fn get(&self, tag: &str) -> Option<&DatabasePool> {
        self.0.get(tag)
    }

    /// Every pool with its tag, in the tags' order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &DatabasePool)> {
        self.0.iter().map(|(tag, pool)| (tag.as_str(), pool))
    }

    /// The current [`Status`] of every pool, keyed by tag.
    pub fn status(&self) -> BTreeMap<&str, Status> {
        self.iter()
            .map(|(tag, pool)| (tag, pool.status()))
            .collect()
    }

    /// [`DatabasePool::close`] every pool.
    pub fn close(&self) {
        self.0.values().for_each(DatabasePool::close);
    }
}

// [spec:pgorm:sem:conn.pool.get+1]
impl DatabasePool {
    /// Take a connection from the pool, waiting for one to become available.
    pub async fn get(&self) -> Result<DatabaseConnection, Error> {
        let conn = Pool::get(&self.0).await?;
        Ok(DatabaseConnection(conn))
    }

    /// The tag this pool was built with, or its generated `default-N` tag.
    pub fn tag(&self) -> Arc<String> {
        self.0.manager().tag()
    }

    /// A snapshot of the pool's size and how many connections are in use.
    pub fn status(&self) -> Status {
        self.0.status()
    }
}

/// A connection checked out of a [`DatabasePool`], returned to the pool on drop.
#[derive(Debug)]
pub struct DatabaseConnection(pub(crate) Object);

/// How [`DatabaseConnection::transaction_with`] opens its transaction: the
/// isolation level and access mode it selects, as the combinations PostgreSQL
/// acts on.
///
/// `DEFERRABLE` is reachable only through [`TransactionMode::DeferrableSnapshot`],
/// which carries the whole `SERIALIZABLE READ ONLY DEFERRABLE` combination,
/// because that is the only one in which the server honours it.
// [spec:pgorm:req:conn.tx+3]    transaction configuration
#[derive(Debug, Clone, Copy, Default)]
#[doc(alias = "AccessMode")]
pub enum TransactionMode {
    /// Inherit the session defaults: `START TRANSACTION` with no clause, the
    /// same transaction [`TransactionTrait::transaction`] opens.
    ///
    /// [`TransactionTrait::transaction`]: crate::TransactionTrait::transaction
    #[default]
    Default,
    /// `READ WRITE`, emitted explicitly so it overrides a session running
    /// under `SET default_transaction_read_only = on`.
    ReadWrite {
        /// `ISOLATION LEVEL <level>`; `None` inherits the session default.
        isolation: Option<IsolationLevel>,
    },
    /// `READ ONLY`: the server rejects any write with SQLSTATE `25006`.
    ReadOnly {
        /// `ISOLATION LEVEL <level>`; `None` inherits the session default.
        isolation: Option<IsolationLevel>,
    },
    /// `ISOLATION LEVEL SERIALIZABLE, READ ONLY, DEFERRABLE`: opening it may
    /// block until a snapshot is safe, after which the transaction cannot fail
    /// with a serialization error.
    DeferrableSnapshot,
}

/// A parent a configured transaction opens on: a [`DatabaseConnection`], and
/// the metrics layer's [`InstrumentedConnection`](crate::metric::InstrumentedConnection)
/// around one, whose handle reports its statements and its ending. A
/// savepoint takes no mode, so no transaction is one.
///
/// Opening is generic over the borrow alone, as [`Open`](super::sealed::Open)'s
/// is, so the configured forms stay `Send` on a concrete receiver.
pub(crate) trait OpenWith: Handle {
    fn open_with(
        &mut self,
        mode: TransactionMode,
    ) -> impl Future<Output = Result<Self::Transaction<'_>, Error>> + Send;
}

impl OpenWith for DatabaseConnection {
    // [spec:pgorm:req:conn.tx+3]    configured START TRANSACTION on the pooled client
    async fn open_with(&mut self, mode: TransactionMode) -> Result<DatabaseTransaction<'_>, Error> {
        let mut builder = self.0.build_transaction();

        match mode {
            TransactionMode::Default => {}
            TransactionMode::ReadWrite { isolation } => {
                if let Some(level) = isolation {
                    builder = builder.isolation_level(level);
                }
                builder = builder.read_only(false);
            }
            TransactionMode::ReadOnly { isolation } => {
                if let Some(level) = isolation {
                    builder = builder.isolation_level(level);
                }
                builder = builder.read_only(true);
            }
            TransactionMode::DeferrableSnapshot => {
                builder = builder
                    .isolation_level(IsolationLevel::Serializable)
                    .read_only(true)
                    .deferrable(true);
            }
        }

        Ok(DatabaseTransaction(Some(builder.start().await?)))
    }
}

/// `transaction_with`'s body, for every parent [`OpenWith`] covers.
// [spec:pgorm:sem:conn.tx.closure+6]    configured BEGIN
pub(crate) async fn with_mode<C, T, E, F>(
    parent: &mut C,
    mode: TransactionMode,
    f: F,
) -> Result<T, TransactionError<E>>
where
    C: OpenWith,
    F: AsyncFnOnce(&mut C::Transaction<'_>) -> Result<T, E>,
{
    let txn = parent
        .open_with(mode)
        .await
        .map_err(TransactionError::Connection)?;

    drive(txn, f).await
}

/// `transaction_with_retry`'s body, for every parent [`OpenWith`] covers.
// [spec:pgorm:sem:conn.tx.retry+3]
pub(crate) async fn with_retry<C, T, E, F>(
    parent: &mut C,
    mode: TransactionMode,
    max_retries: u32,
    mut f: F,
) -> Result<T, TransactionError<E>>
where
    C: OpenWith,
    F: AsyncFnMut(&mut C::Transaction<'_>) -> Result<T, E>,
    E: RetryableError,
{
    let mut retries = 0;

    loop {
        let mut txn = parent
            .open_with(mode)
            .await
            .map_err(TransactionError::Connection)?;

        let outcome = match f(&mut txn).await {
            Ok(value) => match txn.commit(TOKEN).await {
                Ok(()) => return Ok(value),
                Err(error) => Retryable::Connection(error),
            },
            Err(cause) => match rolled_back(txn, cause).await {
                TransactionError::Transaction(error) => Retryable::Transaction(error),
                unfinished => return Err(unfinished),
            },
        };

        let (retryable, raised_by) = match &outcome {
            Retryable::Connection(error) => (error.is_retryable(), "commit"),
            Retryable::Transaction(error) => (error.is_retryable(), "closure"),
        };

        if !retryable || retries == max_retries {
            return Err(match outcome {
                Retryable::Connection(error) => TransactionError::Connection(error),
                Retryable::Transaction(error) => TransactionError::Transaction(error),
            });
        }

        retries += 1;
        tracing::debug!(retries, max_retries, raised_by, "retrying transaction");
    }
}

impl DatabaseConnection {
    /// [`TransactionTrait::transaction`] over a transaction configured by
    /// `mode`.
    ///
    /// This is an inherent method rather than a [`TransactionTrait`] one
    /// because a nested transaction is a savepoint, and `SAVEPOINT` takes
    /// neither an isolation level nor an access mode.
    ///
    /// [`TransactionTrait`]: crate::TransactionTrait
    /// [`TransactionTrait::transaction`]: crate::TransactionTrait::transaction
    // [spec:pgorm:sem:conn.tx.closure+6]    configured BEGIN
    #[doc(alias("transaction_with_config", "begin_with_config"))]
    pub async fn transaction_with<T, E, F>(
        &mut self,
        mode: TransactionMode,
        f: F,
    ) -> Result<T, TransactionError<E>>
    where
        F: AsyncFnOnce(&mut DatabaseTransaction<'_>) -> Result<T, E>,
    {
        with_mode(self, mode, f).await
    }

    /// Run `f` inside a transaction configured by `mode`, retrying the whole
    /// begin/run/commit cycle up to `max_retries` extra times while the failure
    /// is retryable.
    ///
    /// Intended for `IsolationLevel::Serializable`, where the server may reject
    /// a transaction that would break serial order and expects the client to
    /// replay it. `f` is [`AsyncFnMut`] because it is called once per attempt,
    /// and must therefore be replayable: every effect it has outside the
    /// transaction happens again on each attempt.
    // [spec:pgorm:sem:conn.tx.retry+3]
    pub async fn transaction_with_retry<T, E, F>(
        &mut self,
        mode: TransactionMode,
        max_retries: u32,
        f: F,
    ) -> Result<T, TransactionError<E>>
    where
        F: AsyncFnMut(&mut DatabaseTransaction<'_>) -> Result<T, E>,
        E: RetryableError,
    {
        with_retry(self, mode, max_retries, f).await
    }
}

enum Retryable<E> {
    Connection(Error),
    Transaction(E),
}

/// The handle a transaction closure is given, borrowing the
/// [`DatabaseConnection`] or parent transaction it was opened on.
///
/// What the closure returns ends it. A closure whose future is dropped before
/// it finishes leaves this handle to roll back as it drops, which queues the
/// `ROLLBACK` without awaiting the server's answer.
// [spec:pgorm:sem:conn.tx.guard+4]
#[derive(Debug)]
pub struct DatabaseTransaction<'a>(pub(crate) Option<Transaction<'a>>);

impl<'a> DatabaseTransaction<'a> {
    /// The open transaction. The `Option` is emptied only by [`take`], which
    /// the consuming commit and rollback alone call, so it is always `Some`
    /// here.
    ///
    /// [`take`]: DatabaseTransaction::take
    pub(super) fn tx(&self) -> &Transaction<'a> {
        self.0.as_ref().expect("transaction already consumed")
    }

    /// As [`DatabaseTransaction::tx`], for the mutable borrow a nested
    /// savepoint needs.
    pub(super) fn tx_mut(&mut self) -> &mut Transaction<'a> {
        self.0.as_mut().expect("transaction already consumed")
    }

    /// The transaction, for a commit or rollback that consumes the handle
    /// and so leaves its `Drop` nothing to roll back.
    pub(super) fn take(&mut self) -> Transaction<'a> {
        self.0.take().expect("transaction already consumed")
    }
}

// [spec:pgorm:sem:conn.tx.guard+4]    rollback-on-drop warning
impl Drop for DatabaseTransaction<'_> {
    fn drop(&mut self) {
        if self.0.is_some() {
            tracing::warn!("Transaction dropped without committing!");
        }
    }
}

#[async_trait::async_trait]
impl ConnectionTrait for &DatabaseConnection {
    // #[instrument(level = "trace")]
    async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_retry!(&self.0, statement, |prepared| self
            .0
            .execute(prepared, params)
            .await)
    }

    // #[instrument(level = "trace")]
    async fn execute_raw<T, P, I>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        cached_evict!(&self.0, statement, |prepared| self
            .0
            .execute_raw(prepared, params)
            .await)
    }

    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<tokio_postgres::Row, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_retry!(&self.0, statement, |prepared| self
            .0
            .query_one(prepared, params)
            .await)
    }

    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<tokio_postgres::Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_retry!(&self.0, statement, |prepared| self
            .0
            .query_opt(prepared, params)
            .await)
    }

    async fn query_all<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<tokio_postgres::Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_retry!(&self.0, statement, |prepared| self
            .0
            .query(prepared, params)
            .await)
    }

    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        cached_evict!(&self.0, statement, |prepared| self
            .0
            .query_raw(prepared, params)
            .await)
    }

    async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        refuse_copy_statement(sql)?;
        Ok(pgorm_pool::GenericClient::batch_execute(&self.0, sql).await?)
    }
}

// [spec:pgorm:def:conn.pool.conn-trait+9]    cache-routing impls
#[async_trait::async_trait]
impl ConnectionTrait for DatabaseConnection {
    // #[instrument(level = "trace")]
    async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_retry!(&self.0, statement, |prepared| self
            .0
            .execute(prepared, params)
            .await)
    }

    // #[instrument(level = "trace")]
    async fn execute_raw<T, P, I>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        cached_evict!(&self.0, statement, |prepared| self
            .0
            .execute_raw(prepared, params)
            .await)
    }

    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<tokio_postgres::Row, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_retry!(&self.0, statement, |prepared| self
            .0
            .query_one(prepared, params)
            .await)
    }

    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<tokio_postgres::Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_retry!(&self.0, statement, |prepared| self
            .0
            .query_opt(prepared, params)
            .await)
    }

    async fn query_all<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<tokio_postgres::Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_retry!(&self.0, statement, |prepared| self
            .0
            .query(prepared, params)
            .await)
    }

    // [spec:pgorm:def:exec.stream+2]    pooled-client row stream
    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        cached_evict!(&self.0, statement, |prepared| self
            .0
            .query_raw(prepared, params)
            .await)
    }

    async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        refuse_copy_statement(sql)?;
        Ok(pgorm_pool::GenericClient::batch_execute(&self.0, sql).await?)
    }
}

#[async_trait::async_trait]
impl ConnectionTrait for DatabaseTransaction<'_> {
    // #[instrument(level = "trace")]
    async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_evict!(self.tx(), statement, |prepared| self
            .tx()
            .execute(prepared, params)
            .await)
    }

    // #[instrument(level = "trace")]
    async fn execute_raw<T, P, I>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        cached_evict!(self.tx(), statement, |prepared| self
            .tx()
            .execute_raw(prepared, params)
            .await)
    }

    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<tokio_postgres::Row, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_evict!(self.tx(), statement, |prepared| self
            .tx()
            .query_one(prepared, params)
            .await)
    }

    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<tokio_postgres::Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_evict!(self.tx(), statement, |prepared| self
            .tx()
            .query_opt(prepared, params)
            .await)
    }

    async fn query_all<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<tokio_postgres::Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        cached_evict!(self.tx(), statement, |prepared| self
            .tx()
            .query(prepared, params)
            .await)
    }

    // [spec:pgorm:def:exec.stream+2]    in-transaction row stream
    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        cached_evict!(self.tx(), statement, |prepared| self
            .tx()
            .query_raw(prepared, params)
            .await)
    }

    async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        refuse_copy_statement(sql)?;
        Ok(pgorm_pool::GenericClient::batch_execute(self.tx(), sql).await?)
    }
}

#[cfg(test)]
mod tests {
    use crate::DatabasePool;

    #[test]
    fn assert_database_connection_traits() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<DatabasePool>();
    }
}
