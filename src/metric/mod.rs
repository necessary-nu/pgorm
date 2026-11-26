mod fingerprint;
mod pool;

pub use fingerprint::{QueryContext, QueryFingerprint};
pub use pool::InstrumentedPool;

use crate::{
    Connection, ConnectionMut, DatabaseConnection, DatabaseTransaction, Error, IsolationLevel,
    ReadOnlyConnection, ReadOnlyTransaction, RetryableError, RunsOn, SqlText, Transact,
    TransactionError, TransactionMode, Unchecked,
    database::{
        Kinded, OpenWith, drive,
        sealed::{Close, Handle, Open, OpenReadOnly, ReadOnlyMode, Token},
        with_mode, with_retry,
    },
};
use async_trait::async_trait;
use fingerprint::FingerprintSuffix;
use pgorm_driver::{
    Row, RowStream,
    types::{BorrowToSql, ToSql},
};
use std::{
    marker::PhantomData,
    time::{Duration, Instant},
};

/// The body every instrumented `Connection` method shares: build the
/// context, time the awaited delegation, report the outcome under the hook it
/// selects, and hand back the wrapped value's own result untouched.
///
/// What actually differs between the fourteen methods is three things — the
/// operation name, where the SQL text comes from, and how a row count is read
/// out of the `Ok` payload — so those are the parameters and the rest is
/// written once. `$rows` is evaluated against the payload by reference.
// [spec:pgorm:req:metric.layer.delegate+7]    the timing and reporting body
macro_rules! report {
    ($self:ident, $operation:literal, $sql:expr, $call:expr, $ok:pat => $rows:expr) => {{
        let query = QueryContext::new($operation, $sql);
        let start = Instant::now();
        let result = $call;
        let elapsed = start.elapsed();

        match &result {
            Ok($ok) => {
                $self
                    .metrics
                    .record_query_success(query, elapsed, $rows)
                    .await;
            }
            Err(error) => {
                $self
                    .metrics
                    .record_query_error(query, elapsed, error)
                    .await;
            }
        }

        result
    }};
}

/// Trait for collecting database metrics
// [spec:pgorm:def:metric.layer.collector+3]
#[async_trait]
pub trait MetricsCollector: Clone + Send + Sync + 'static {
    /// Record a successful database operation
    async fn record_query_success(
        &self,
        query: QueryContext<'_>,
        duration: Duration,
        rows: Option<u64>,
    );

    /// Record a failed database operation
    async fn record_query_error(&self, query: QueryContext<'_>, duration: Duration, error: &Error);

    /// Record connection acquisition
    async fn record_connection_acquired(&self, duration: Duration);

    /// Record connection acquisition failure
    async fn record_connection_error(&self, duration: Duration, error: &Error);

    /// Record transaction begin
    async fn record_transaction_begin(&self, duration: Duration);

    /// Record that a transaction committed.
    ///
    /// Success only: a `COMMIT` that returned an error did not commit, and is
    /// reported through [`record_transaction_rollback`] and
    /// [`record_query_error`] instead.
    ///
    /// [`record_transaction_rollback`]: MetricsCollector::record_transaction_rollback
    /// [`record_query_error`]: MetricsCollector::record_query_error
    async fn record_transaction_commit(&self, duration: Duration);

    /// Record that a transaction ended without committing.
    ///
    /// Not only a closure that returned `Err`. PostgreSQL aborts a transaction
    /// whose `COMMIT` fails, so a failed commit ends the transaction exactly as
    /// a rollback does and is reported here too — an implementor counting this
    /// hook is counting transactions that did not take effect, which is the
    /// number worth counting, not the number of closures that failed. A failed
    /// commit additionally reports the error itself through
    /// [`record_query_error`] under the operation `"commit"`, as a failed
    /// rollback does under `"rollback"`, so the cause is never only implied by
    /// a counter.
    ///
    /// A transaction whose closure's future is dropped before it finishes
    /// records nothing at all, here or anywhere: its handle rolls back as it
    /// drops, every hook is `async`, and `Drop::drop` is not.
    ///
    /// [`record_query_error`]: MetricsCollector::record_query_error
    // [spec:pgorm:sem:metric.layer.tx+7]    the semantic an implementor reads at the trait
    async fn record_transaction_rollback(&self, duration: Duration);
}

/// A no-op metrics collector that does nothing (zero cost)
#[derive(Clone, Debug)]
pub struct NoOpMetrics;

#[async_trait]
impl MetricsCollector for NoOpMetrics {
    async fn record_query_success(
        &self,
        _query: QueryContext<'_>,
        _duration: Duration,
        _rows: Option<u64>,
    ) {
    }
    async fn record_query_error(
        &self,
        _query: QueryContext<'_>,
        _duration: Duration,
        _error: &Error,
    ) {
    }
    async fn record_connection_acquired(&self, _duration: Duration) {}
    async fn record_connection_error(&self, _duration: Duration, _error: &Error) {}
    async fn record_transaction_begin(&self, _duration: Duration) {}
    async fn record_transaction_commit(&self, _duration: Duration) {}
    async fn record_transaction_rollback(&self, _duration: Duration) {}
}

/// A simple logging-based metrics collector
#[derive(Clone, Debug)]
pub struct LoggingMetrics;

// [spec:pgorm:def:metric.layer.collector+3]    LoggingMetrics tracing levels
#[async_trait]
impl MetricsCollector for LoggingMetrics {
    async fn record_query_success(
        &self,
        query: QueryContext<'_>,
        duration: Duration,
        rows: Option<u64>,
    ) {
        let fingerprint = FingerprintSuffix(query.fingerprint());
        if let Some(rows) = rows {
            tracing::debug!(
                "Query {} succeeded in {:?}, {} rows{}",
                query.operation(),
                duration,
                rows,
                fingerprint
            );
        } else {
            tracing::debug!(
                "Query {} succeeded in {:?}{}",
                query.operation(),
                duration,
                fingerprint
            );
        }
    }

    async fn record_query_error(&self, query: QueryContext<'_>, duration: Duration, error: &Error) {
        tracing::warn!(
            "Query {} failed after {:?}: {:?}{}",
            query.operation(),
            duration,
            error,
            FingerprintSuffix(query.fingerprint())
        );
    }

    async fn record_connection_acquired(&self, duration: Duration) {
        tracing::debug!("Connection acquired in {:?}", duration);
    }

    async fn record_connection_error(&self, duration: Duration, error: &Error) {
        tracing::error!("Connection failed after {:?}: {:?}", duration, error);
    }

    async fn record_transaction_begin(&self, duration: Duration) {
        tracing::debug!("Transaction began in {:?}", duration);
    }

    async fn record_transaction_commit(&self, duration: Duration) {
        tracing::debug!("Transaction committed in {:?}", duration);
    }

    async fn record_transaction_rollback(&self, duration: Duration) {
        tracing::warn!("Transaction rolled back in {:?}", duration);
    }
}

/// A connection wrapper that instruments all database operations: a
/// [`DatabaseConnection`] by default, or the [`ReadOnlyConnection`] an
/// [`InstrumentedPool`] over a [`ReadOnlyPool`](crate::ReadOnlyPool) lends,
/// which stays a connection that may only read.
// [spec:pgorm:def:metric.layer+4]    connection wrapper
// [spec:pgorm:req:read-only.metrics]
#[derive(Debug)]
pub struct InstrumentedConnection<M: MetricsCollector, C = DatabaseConnection> {
    connection: C,
    metrics: M,
}

impl<M: MetricsCollector, C> InstrumentedConnection<M, C> {
    /// Create a new instrumented connection
    pub fn new(connection: C, metrics: M) -> Self {
        Self {
            connection,
            metrics,
        }
    }

    /// Get a reference to the underlying connection
    pub fn inner(&self) -> &C {
        &self.connection
    }

    /// The wrapped connection, given back: what runs on it from here is not
    /// reported.
    // [spec:pgorm:def:metric.layer+4]    the connection wrapper given back
    pub fn into_inner(self) -> C {
        self.connection
    }

    /// Get a reference to the metrics collector
    pub fn metrics(&self) -> &M {
        &self.metrics
    }
}

impl<M: MetricsCollector> InstrumentedConnection<M> {
    /// [`DatabaseConnection::transaction_with`], reported as
    /// [`transaction`](Transact::transaction) reports: the closure is
    /// given an [`InstrumentedTransaction`], and the configured `BEGIN` is
    /// timed and reported as any opening is.
    // [spec:pgorm:sem:metric.layer.tx+7]    a configured transaction on an instrumented connection
    pub async fn transaction_with<T, E, F>(
        &mut self,
        mode: TransactionMode,
        f: F,
    ) -> Result<T, TransactionError<E>>
    where
        F: AsyncFnOnce(&mut InstrumentedTransaction<'_, M>) -> Result<T, E>,
    {
        with_mode(self, mode, f).await
    }

    /// [`DatabaseConnection::transaction_with_retry`], each attempt reported
    /// as one transaction: its opening, its statements, and its commit or
    /// rollback.
    // [spec:pgorm:sem:metric.layer.tx+7]    every retried attempt reported as a transaction
    pub async fn transaction_with_retry<T, E, F>(
        &mut self,
        mode: TransactionMode,
        max_retries: u32,
        f: F,
    ) -> Result<T, TransactionError<E>>
    where
        F: AsyncFnMut(&mut InstrumentedTransaction<'_, M>) -> Result<T, E>,
        E: RetryableError,
    {
        with_retry(self, mode, max_retries, f).await
    }
}

impl<M: MetricsCollector, C: OpenReadOnly> InstrumentedConnection<M, C> {
    /// [`DatabaseConnection::read_only_transaction`], reported as any
    /// transaction is: the closure is given an [`InstrumentedTransaction`]
    /// over a [`ReadOnlyTransaction`], which may only read.
    // [spec:pgorm:req:read-only.metrics]
    pub async fn read_only_transaction<T, E, F>(
        &mut self,
        isolation: Option<IsolationLevel>,
        f: F,
    ) -> Result<T, TransactionError<E>>
    where
        F: AsyncFnOnce(
            &mut InstrumentedTransaction<'_, M, ReadOnlyTransaction<'_>>,
        ) -> Result<T, E>,
    {
        let txn = self
            .open_read_only(ReadOnlyMode::Isolation(isolation))
            .await
            .map_err(TransactionError::Connection)?;
        drive(txn, f).await
    }

    /// [`DatabaseConnection::snapshot`], reported as any transaction is.
    // [spec:pgorm:req:read-only.metrics]
    pub async fn snapshot<T, E, F>(&mut self, f: F) -> Result<T, TransactionError<E>>
    where
        F: AsyncFnOnce(
            &mut InstrumentedTransaction<'_, M, ReadOnlyTransaction<'_>>,
        ) -> Result<T, E>,
    {
        let txn = self
            .open_read_only(ReadOnlyMode::Snapshot)
            .await
            .map_err(TransactionError::Connection)?;
        drive(txn, f).await
    }

    /// A read-only transaction's opening, timed and reported.
    async fn open_read_only(
        &mut self,
        mode: ReadOnlyMode,
    ) -> Result<InstrumentedTransaction<'_, M, ReadOnlyTransaction<'_>>, Error> {
        begun(self.metrics.clone(), self.connection.open_read_only(mode)).await
    }
}

impl<M: MetricsCollector> InstrumentedConnection<M, ReadOnlyConnection> {
    /// [`ReadOnlyConnection::unchecked`], reported: SQL text of any kind,
    /// which the read-only session's server refuses to let write.
    // [spec:pgorm:req:read-only.metrics]
    pub fn unchecked(&self) -> Unchecked<'_, Self> {
        Unchecked(self)
    }
}

/// The `Connection` impl of an instrumented wrapper: each statement method
/// reported through `report!`, the statement handed to the wrapped
/// connection as a read once the wrapper's own bound has admitted it.
macro_rules! instrumented {
    ([$($generics:tt)*] $ty:ty, $field:ident) => {
        #[async_trait]
        impl<$($generics)*> Connection for $ty {
            async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
            where
                T: ?Sized + SqlText + Sync,
                T::Kind: RunsOn<Self>,
            {
                report!(self, "execute", Some(statement.sql_text()),
                    self.$field.execute(&Kinded::read(statement), params).await, rows => Some(*rows))
            }

            async fn execute_raw<T, P, I>(&self, statement: &T, params: I) -> Result<u64, Error>
            where
                T: ?Sized + SqlText + Sync,
                T::Kind: RunsOn<Self>,
                P: BorrowToSql,
                I: IntoIterator<Item = P> + Send,
                I::IntoIter: ExactSizeIterator,
            {
                report!(self, "execute_raw", Some(statement.sql_text()),
                    self.$field.execute_raw(&Kinded::read(statement), params).await, rows => Some(*rows))
            }

            async fn query_one<T>(
                &self,
                statement: &T,
                params: &[&(dyn ToSql + Sync)],
            ) -> Result<Row, Error>
            where
                T: ?Sized + SqlText + Sync,
                T::Kind: RunsOn<Self>,
            {
                report!(self, "query_one", Some(statement.sql_text()),
                    self.$field.query_one(&Kinded::read(statement), params).await, _ => Some(1))
            }

            async fn query_opt<T>(
                &self,
                statement: &T,
                params: &[&(dyn ToSql + Sync)],
            ) -> Result<Option<Row>, Error>
            where
                T: ?Sized + SqlText + Sync,
                T::Kind: RunsOn<Self>,
            {
                report!(self, "query_opt", Some(statement.sql_text()),
                    self.$field.query_opt(&Kinded::read(statement), params).await, row => Some(if row.is_some() { 1 } else { 0 }))
            }

            async fn query_all<T>(
                &self,
                statement: &T,
                params: &[&(dyn ToSql + Sync)],
            ) -> Result<Vec<Row>, Error>
            where
                T: ?Sized + SqlText + Sync,
                T::Kind: RunsOn<Self>,
            {
                report!(self, "query_all", Some(statement.sql_text()),
                    self.$field.query_all(&Kinded::read(statement), params).await, rows => Some(rows.len() as u64))
            }

            // [spec:pgorm:sem:exec.stream.decode+2]    row count unknown at stream creation
            async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
            where
                T: ?Sized + SqlText + Sync,
                T::Kind: RunsOn<Self>,
                P: BorrowToSql,
                I: IntoIterator<Item = P> + Send,
                I::IntoIter: ExactSizeIterator,
            {
                report!(self, "query_raw", Some(statement.sql_text()),
                    self.$field.query_raw(&Kinded::read(statement), params).await, _ => None)
            }
        }
    };
}

// [spec:pgorm:req:metric.layer.delegate+7]
instrumented!([M: MetricsCollector, C: Connection] InstrumentedConnection<M, C>, connection);

// [spec:pgorm:req:metric.layer.delegate+7]
#[async_trait]
impl<M: MetricsCollector, C: ConnectionMut> ConnectionMut for InstrumentedConnection<M, C> {
    async fn run_batch(&self, sql: &str) -> Result<(), Error> {
        report!(self, "batch_execute", Some(sql),
            self.connection.batch_execute(sql).await, () => None)
    }
}

/// The handle a transaction closure on an [`InstrumentedConnection`] is given:
/// a [`DatabaseTransaction`] by default, or a [`ReadOnlyTransaction`], whose
/// statements, savepoints and ending are reported to the collector.
// [spec:pgorm:sem:metric.layer.tx+7]    an instrumented handle for every instrumented transaction
#[derive(Debug)]
pub struct InstrumentedTransaction<'a, M: MetricsCollector, T = DatabaseTransaction<'a>> {
    transaction: T,
    metrics: M,
    borrow: PhantomData<&'a ()>,
}

impl<M: MetricsCollector, T> InstrumentedTransaction<'_, M, T> {
    /// Get a reference to the underlying transaction
    pub fn inner(&self) -> &T {
        &self.transaction
    }

    /// Get a reference to the metrics collector
    pub fn metrics(&self) -> &M {
        &self.metrics
    }
}

impl<'a, M: MetricsCollector> InstrumentedTransaction<'a, M, ReadOnlyTransaction<'a>> {
    /// [`ReadOnlyTransaction::unchecked`], reported.
    // [spec:pgorm:req:read-only.metrics]
    pub fn unchecked(&self) -> Unchecked<'_, Self> {
        Unchecked(self)
    }
}

// [spec:pgorm:req:metric.layer.delegate+7]    statements inside a transaction
instrumented!(['a, M: MetricsCollector, Tx: Connection] InstrumentedTransaction<'a, M, Tx>, transaction);

// [spec:pgorm:req:metric.layer.delegate+7]
#[async_trait]
impl<M: MetricsCollector, T: ConnectionMut> ConnectionMut for InstrumentedTransaction<'_, M, T> {
    async fn run_batch(&self, sql: &str) -> Result<(), Error> {
        report!(self, "batch_execute", Some(sql),
            self.transaction.batch_execute(sql).await, () => None)
    }
}

// Declared below the reporting macros: a `macro_rules!` macro is visible
// only to code after its definition, a child module's included.
mod copy;

// [spec:pgorm:sem:metric.layer.tx+7]    a transaction opened on an instrumented connection
impl<M: MetricsCollector, C: Open + Send> Transact for InstrumentedConnection<M, C> {}

impl<M: MetricsCollector, C: Handle> Handle for InstrumentedConnection<M, C> {
    type Transaction<'t> = InstrumentedTransaction<'t, M, C::Transaction<'t>>;
}

impl<M: MetricsCollector, C: Open + Send> Open for InstrumentedConnection<M, C> {
    async fn open(&mut self, token: Token) -> Result<Self::Transaction<'_>, Error> {
        begun(self.metrics.clone(), self.connection.open(token)).await
    }
}

impl<M: MetricsCollector> OpenWith for InstrumentedConnection<M> {
    async fn open_with(
        &mut self,
        mode: TransactionMode,
    ) -> Result<InstrumentedTransaction<'_, M>, Error> {
        begun(self.metrics.clone(), self.connection.open_with(mode)).await
    }
}

// [spec:pgorm:sem:metric.layer.tx+7]    a savepoint inside an instrumented transaction
impl<M: MetricsCollector, T: Open + Send> Transact for InstrumentedTransaction<'_, M, T> {}

impl<M: MetricsCollector, T: Handle> Handle for InstrumentedTransaction<'_, M, T> {
    type Transaction<'t> = InstrumentedTransaction<'t, M, T::Transaction<'t>>;
}

impl<M: MetricsCollector, T: Open + Send> Open for InstrumentedTransaction<'_, M, T> {
    async fn open(&mut self, token: Token) -> Result<Self::Transaction<'_>, Error> {
        begun(self.metrics.clone(), self.transaction.open(token)).await
    }
}

/// Time `opening` and report it: `record_transaction_begin` once it is open,
/// `record_query_error` under `"begin"` if the server refused it.
// [spec:pgorm:sem:metric.layer.tx+7]    a timed BEGIN or SAVEPOINT
async fn begun<'t, M: MetricsCollector, T>(
    metrics: M,
    opening: impl Future<Output = Result<T, Error>>,
) -> Result<InstrumentedTransaction<'t, M, T>, Error> {
    let start = Instant::now();
    let opened = opening.await;
    let elapsed = start.elapsed();

    match opened {
        Ok(transaction) => {
            metrics.record_transaction_begin(elapsed).await;
            Ok(InstrumentedTransaction {
                transaction,
                metrics,
                borrow: PhantomData,
            })
        }
        Err(error) => {
            metrics
                .record_query_error(QueryContext::new("begin", None), elapsed, &error)
                .await;
            Err(error)
        }
    }
}

// [spec:pgorm:sem:metric.layer.tx+7]    the ending a closure's outcome selects, reported
impl<M: MetricsCollector, T: Close + Send> Close for InstrumentedTransaction<'_, M, T> {
    /// A successful commit reports `record_transaction_commit`. A failed one
    /// reports the error through `record_query_error` under `"commit"` and then
    /// `record_transaction_rollback`, the pair a failed rollback reports,
    /// because PostgreSQL has aborted the transaction either way.
    async fn commit(self, token: Token) -> Result<(), Error> {
        let start = Instant::now();
        let committed = self.transaction.commit(token).await;
        let elapsed = start.elapsed();

        match &committed {
            Ok(()) => self.metrics.record_transaction_commit(elapsed).await,
            Err(error) => {
                self.metrics
                    .record_query_error(QueryContext::new("commit", None), elapsed, error)
                    .await;
                self.metrics.record_transaction_rollback(elapsed).await;
            }
        }

        committed
    }

    /// Reports `record_transaction_rollback` on either outcome — the
    /// transaction is discarded whether or not the `ROLLBACK` round trip
    /// succeeded — and a failed round trip additionally through
    /// `record_query_error` under `"rollback"`.
    async fn rollback(self, token: Token) -> Result<(), Error> {
        let start = Instant::now();
        let rolled_back = self.transaction.rollback(token).await;
        let elapsed = start.elapsed();

        if let Err(error) = &rolled_back {
            self.metrics
                .record_query_error(QueryContext::new("rollback", None), elapsed, error)
                .await;
        }
        self.metrics.record_transaction_rollback(elapsed).await;

        rolled_back
    }
}
