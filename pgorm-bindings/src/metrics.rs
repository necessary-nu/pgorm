//! pgorm's metrics layer with a binding's collector: an `InstrumentedPool`
//! whose hooks are the language's own, each told on the language's thread.
//!
//! The timing and the choice of hook are pgorm's: an instrumented pool lends
//! `InstrumentedConnection`s, so every statement, acquisition and transaction
//! is reported by `pgorm::metric`. What a hook is told is decided here, once
//! for both bindings; the binding's [`Reporter`] carries it across to the
//! language and answers once the language's hook has finished, so a statement
//! finishes only once its collector has been told.

use std::{fmt, sync::Arc, time::Duration};

use futures_util::future::BoxFuture;
use pgorm::{
    DatabaseRef, Error,
    metric::{MetricsCollector, QueryContext},
};

use crate::{Dialect, Failure, Redactions, classify};

/// Whichever executor a binding holds: a pool's connection, a transaction,
/// either through pgorm's metrics wrapper, or a pool hook's connection.
pub type Database<'a> = DatabaseRef<'a, Metrics>;

/// One of `MetricsCollector`'s seven hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hook {
    /// A statement ran.
    QuerySuccess,
    /// A statement failed.
    QueryError,
    /// The pool lent a connection.
    ConnectionAcquired,
    /// The pool could not lend one.
    ConnectionError,
    /// A transaction or savepoint opened.
    TransactionBegin,
    /// One committed or was released.
    TransactionCommit,
    /// One rolled back.
    TransactionRollback,
}

impl Hook {
    /// The seven hooks, in `MetricsCollector`'s order.
    pub const ALL: [Self; 7] = [
        Self::QuerySuccess,
        Self::QueryError,
        Self::ConnectionAcquired,
        Self::ConnectionError,
        Self::TransactionBegin,
        Self::TransactionCommit,
        Self::TransactionRollback,
    ];

    /// The hook's method name, as `dialect` spells it.
    // [spec:pgorm:req:bindings.metrics]
    pub fn name(self, dialect: Dialect) -> String {
        dialect.option(match self {
            Self::QuerySuccess => "record_query_success",
            Self::QueryError => "record_query_error",
            Self::ConnectionAcquired => "record_connection_acquired",
            Self::ConnectionError => "record_connection_error",
            Self::TransactionBegin => "record_transaction_begin",
            Self::TransactionCommit => "record_transaction_commit",
            Self::TransactionRollback => "record_transaction_rollback",
        })
    }
}

/// What a hook is told, owned until the language's thread makes it values.
#[derive(Debug)]
pub enum Report {
    /// A statement's hook: its context, how long it ran, and the rows it
    /// reported or why it failed.
    Query {
        /// `QueryContext::operation`.
        operation: String,
        /// `QueryContext::sql`.
        sql: Option<String>,
        /// How long the statement ran.
        duration: Duration,
        /// The rows it affected, where it says, or its failure.
        outcome: Result<Option<u64>, Failure>,
    },
    /// An acquisition's or a transaction's hook: how long it took.
    Duration(Duration),
    /// A failed acquisition: how long it took, and why it failed.
    Failed(Duration, Failure),
}

/// A binding's crossing into its language: whether the collector it was
/// given reports a hook, and the hook told what `report` says.
pub trait Reporter: Send + Sync + 'static {
    /// Whether the collector reports `hook`, or left it as `NoOpMetrics`
    /// wrote it, so that it is never sent.
    fn reports(&self, hook: Hook) -> bool;

    /// Hand `report` to the language's `hook`. The future finishes once the
    /// hook has run, or at once when it could not be handed over; a hook's
    /// own failure is the language's to report, so nothing here fails the
    /// statement.
    fn report(&self, hook: Hook, report: Report) -> BoxFuture<'static, ()>;
}

/// The collector an instrumented binding pool hands pgorm's wrappers: each
/// hook they call is handed to the binding's [`Reporter`], its failures
/// classified and redacted as `bindings.errors` says.
// [spec:pgorm:req:bindings.metrics]
#[derive(Clone)]
pub struct Metrics {
    reporter: Arc<dyn Reporter>,
    secrets: Redactions,
    dialect: Dialect,
}

impl fmt::Debug for Metrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reported: Vec<String> = Hook::ALL
            .into_iter()
            .filter(|hook| self.reporter.reports(*hook))
            .map(|hook| hook.name(self.dialect))
            .collect();
        f.debug_struct("Metrics")
            .field("reported", &reported)
            .finish_non_exhaustive()
    }
}

impl Metrics {
    /// A collector reporting through `reporter`, its failures redacted of
    /// `secrets` and worded for `dialect`.
    pub fn new(reporter: impl Reporter, secrets: Redactions, dialect: Dialect) -> Self {
        Self {
            reporter: Arc::new(reporter),
            secrets,
            dialect,
        }
    }

    async fn report(&self, hook: Hook, report: impl FnOnce() -> Report) {
        if self.reporter.reports(hook) {
            self.reporter.report(hook, report()).await;
        }
    }

    fn query(
        query: QueryContext<'_>,
        duration: Duration,
        outcome: Result<Option<u64>, Failure>,
    ) -> Report {
        Report::Query {
            operation: query.operation().to_owned(),
            sql: query.sql().map(str::to_owned),
            duration,
            outcome,
        }
    }

    fn failure(&self, error: &Error) -> Failure {
        classify(error, &self.secrets, self.dialect)
    }
}

#[pgorm::entity::prelude::async_trait::async_trait]
impl MetricsCollector for Metrics {
    async fn record_query_success(
        &self,
        query: QueryContext<'_>,
        duration: Duration,
        rows: Option<u64>,
    ) {
        self.report(Hook::QuerySuccess, || {
            Self::query(query, duration, Ok(rows))
        })
        .await;
    }

    async fn record_query_error(&self, query: QueryContext<'_>, duration: Duration, error: &Error) {
        self.report(Hook::QueryError, || {
            Self::query(query, duration, Err(self.failure(error)))
        })
        .await;
    }

    async fn record_connection_acquired(&self, duration: Duration) {
        self.report(Hook::ConnectionAcquired, || Report::Duration(duration))
            .await;
    }

    async fn record_connection_error(&self, duration: Duration, error: &Error) {
        self.report(Hook::ConnectionError, || {
            Report::Failed(duration, self.failure(error))
        })
        .await;
    }

    async fn record_transaction_begin(&self, duration: Duration) {
        self.report(Hook::TransactionBegin, || Report::Duration(duration))
            .await;
    }

    async fn record_transaction_commit(&self, duration: Duration) {
        self.report(Hook::TransactionCommit, || Report::Duration(duration))
            .await;
    }

    async fn record_transaction_rollback(&self, duration: Duration) {
        self.report(Hook::TransactionRollback, || Report::Duration(duration))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Told(Mutex<Vec<(Hook, String)>>);

    struct Recorder(Arc<Told>);

    impl Reporter for Recorder {
        fn reports(&self, hook: Hook) -> bool {
            hook != Hook::TransactionBegin
        }

        fn report(&self, hook: Hook, report: Report) -> BoxFuture<'static, ()> {
            let told = match report {
                Report::Query {
                    operation, outcome, ..
                } => format!("{operation} {outcome:?}"),
                Report::Duration(_) => "timed".to_owned(),
                Report::Failed(_, failure) => failure.class().name().to_owned(),
            };
            if let Ok(mut log) = self.0.0.lock() {
                log.push((hook, told));
            }
            Box::pin(async {})
        }
    }

    // [spec:pgorm:req:bindings.metrics/test]    each hook is named by the binding's rule, a hook
    // the collector left out is never sent, and a failure reaches it classified
    #[tokio::test]
    async fn hooks_are_told_what_happened_in_each_spelling() {
        assert_eq!(
            Hook::QuerySuccess.name(Dialect::Python),
            "record_query_success"
        );
        assert_eq!(
            Hook::TransactionRollback.name(Dialect::JavaScript),
            "recordTransactionRollback"
        );
        let told = Arc::new(Told::default());
        let metrics = Metrics::new(
            Recorder(told.clone()),
            Redactions::default(),
            Dialect::Python,
        );
        let context = QueryContext::new("select", Some("SELECT 1"));
        metrics
            .record_query_success(context, Duration::ZERO, Some(1))
            .await;
        metrics
            .record_query_error(context, Duration::ZERO, &Error::RecordNotFound)
            .await;
        metrics.record_transaction_begin(Duration::ZERO).await;
        metrics
            .record_connection_error(Duration::ZERO, &Error::Custom("refused".into()))
            .await;
        let log = told.0.lock().map(|log| log.clone()).unwrap_or_default();
        assert_eq!(
            log,
            vec![
                (Hook::QuerySuccess, "select Ok(Some(1))".to_owned()),
                (
                    Hook::QueryError,
                    "select Err(Decode(\"No records were returned for the given query\"))"
                        .to_owned()
                ),
                (Hook::ConnectionError, "ConstructionError".to_owned()),
            ]
        );
        assert!(!format!("{metrics:?}").contains("record_transaction_begin"));
    }
}
