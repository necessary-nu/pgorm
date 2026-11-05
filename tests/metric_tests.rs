#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, setup::*};

use async_trait::async_trait;
use pgorm::{
    ConnectionTrait, Error, SqlText, TransactionTrait,
    metric::{
        InstrumentedConnection, InstrumentedPool, InstrumentedTransaction, LoggingMetrics,
        MetricsCollector, NoOpMetrics, QueryContext,
    },
};
use pretty_assertions::assert_eq;
use std::{
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt},
    registry::LookupSpan,
    util::SubscriberInitExt,
};

#[derive(Clone, Debug, Default)]
struct RecordingMetrics {
    events: Arc<Mutex<Vec<String>>>,
    queries: Arc<Mutex<Vec<RecordedQuery>>>,
}

/// What a query hook's [`QueryContext`] said, copied out of the borrowed view.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedQuery {
    operation: String,
    sql: Option<String>,
    fingerprint: Option<String>,
}

impl RecordedQuery {
    fn of(query: QueryContext<'_>) -> Self {
        Self {
            operation: query.operation().to_owned(),
            sql: query.sql().map(str::to_owned),
            fingerprint: query.fingerprint().map(|print| print.to_string()),
        }
    }
}

impl RecordingMetrics {
    fn push(&self, event: String) {
        self.events.lock().unwrap().push(event);
    }

    fn record(&self, query: QueryContext<'_>) {
        self.queries.lock().unwrap().push(RecordedQuery::of(query));
    }

    fn queries(&self) -> Vec<RecordedQuery> {
        self.queries.lock().unwrap().clone()
    }

    fn fingerprint_of(&self, sql: &str) -> Option<String> {
        self.queries()
            .into_iter()
            .find(|query| query.sql.as_deref() == Some(sql))
            .and_then(|query| query.fingerprint)
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    fn count(&self, prefix: &str) -> usize {
        self.events()
            .iter()
            .filter(|event| event.starts_with(prefix))
            .count()
    }
}

#[async_trait]
impl MetricsCollector for RecordingMetrics {
    async fn record_query_success(
        &self,
        query: QueryContext<'_>,
        _duration: Duration,
        rows: Option<u64>,
    ) {
        self.push(format!("query_success:{}:{rows:?}", query.operation()));
        self.record(query);
    }

    async fn record_query_error(
        &self,
        query: QueryContext<'_>,
        _duration: Duration,
        _error: &Error,
    ) {
        self.push(format!("query_error:{}", query.operation()));
        self.record(query);
    }

    async fn record_connection_acquired(&self, _duration: Duration) {
        self.push("connection_acquired".to_owned());
    }

    async fn record_connection_error(&self, _duration: Duration, _error: &Error) {
        self.push("connection_error".to_owned());
    }

    async fn record_transaction_begin(&self, _duration: Duration) {
        self.push("transaction_begin".to_owned());
    }

    async fn record_transaction_commit(&self, _duration: Duration) {
        self.push("transaction_commit".to_owned());
    }

    async fn record_transaction_rollback(&self, _duration: Duration) {
        self.push("transaction_rollback".to_owned());
    }
}

// [spec:pgorm:sem:metric.layer.tx+5/test]    a closure that fails reports its begin and its rollback, with the statements inside instrumented
#[pgorm_macros::test]
pub async fn instrumented_begin_and_rollback() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_rollback_metrictx").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let mut conn = pool.get().await?;
    conn.execute("CREATE TABLE widget (id int primary key)", &[])
        .await?;

    let outcome = conn
        .transaction(async |txn| {
            txn.execute("INSERT INTO widget (id) VALUES (1)", &[])
                .await?;
            assert_eq!(txn.query_all("SELECT id FROM widget", &[]).await?.len(), 1);
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);

    assert_eq!(conn.query_all("SELECT id FROM widget", &[]).await?.len(), 0);

    assert_eq!(metrics.count("connection_acquired"), 1);
    assert_eq!(metrics.count("transaction_begin"), 1);
    assert_eq!(metrics.count("transaction_rollback"), 1);
    assert_eq!(metrics.count("transaction_commit"), 0);
    assert_eq!(metrics.count("query_error"), 0);
    assert!(
        metrics
            .events()
            .contains(&"query_success:execute:Some(1)".to_owned()),
        "statements inside the wrapped transaction stay instrumented: {:?}",
        metrics.events()
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:metric.layer.tx+5/test]    a closure that succeeds reports its begin and its commit
#[pgorm_macros::test]
pub async fn instrumented_begin_and_commit() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_commit_metrictx").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let mut conn = pool.get().await?;
    conn.execute("CREATE TABLE widget (id int primary key)", &[])
        .await?;

    conn.transaction(async |txn| txn.execute("INSERT INTO widget (id) VALUES (1)", &[]).await)
        .await
        .expect("the instrumented transaction commits");

    assert_eq!(conn.query_all("SELECT id FROM widget", &[]).await?.len(), 1);

    assert_eq!(metrics.count("transaction_begin"), 1);
    assert_eq!(metrics.count("transaction_commit"), 1);
    // A commit that worked is not an ending-without-committing, and not an
    // error: the two hooks a failed commit reports are both silent here.
    assert_eq!(metrics.count("transaction_rollback"), 0);
    assert_eq!(metrics.count("query_error"), 0);

    drop(conn);
    ctx.delete().await;

    Ok(())
}

/// A `COMMIT` the server refuses ends the transaction without committing, so
/// it reports the rollback hook — and reports the error itself too, under the
/// operation `"commit"`, rather than leaving a counter to imply it.
// [spec:pgorm:sem:metric.layer.tx+5/test]    a failed commit reaches an error hook
// [spec:pgorm:def:metric.layer.collector+3/test]    what the rollback hook counts
#[pgorm_macros::test]
pub async fn a_failed_commit_reports_error_and_rollback() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_commit_fails_metrictx").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let mut conn = pool.get().await?;
    // Deferring the constraint moves its check to `COMMIT`, so what fails is
    // the commit rather than a statement inside the transaction.
    conn.execute(
        "CREATE TABLE widget (id int, CONSTRAINT widget_id_unique UNIQUE (id) \
         DEFERRABLE INITIALLY DEFERRED)",
        &[],
    )
    .await?;

    commit_refused(
        conn.transaction(async |txn| {
            txn.execute("INSERT INTO widget (id) VALUES (1)", &[])
                .await?;
            txn.execute("INSERT INTO widget (id) VALUES (1)", &[]).await
        })
        .await,
    );

    // Nothing took effect, which is what the rollback hook is reporting.
    assert_eq!(conn.query_all("SELECT id FROM widget", &[]).await?.len(), 0);

    assert_eq!(metrics.count("transaction_commit"), 0);
    assert_eq!(metrics.count("transaction_rollback"), 1);
    assert_eq!(metrics.count("query_error:commit"), 1);

    // Both hooks, error first, as a failed `rollback` reports them.
    let events = metrics.events();
    let error_at = events
        .iter()
        .position(|event| event == "query_error:commit");
    let rollback_at = events
        .iter()
        .position(|event| event == "transaction_rollback");
    assert!(
        error_at < rollback_at && error_at.is_some(),
        "expected query_error:commit before transaction_rollback: {events:?}"
    );

    // The context names the round trip and carries no statement text, because
    // none was sent — so it has no fingerprint either.
    let reported = metrics
        .queries()
        .into_iter()
        .find(|query| query.operation == "commit")
        .expect("the commit failure must reach a query hook");
    assert_eq!(reported.sql, None);
    assert_eq!(reported.fingerprint, None);

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:metric.layer.delegate+5/test]    query_opt's Some/None row counts
#[pgorm_macros::test]
pub async fn query_opt_reports_one_row_or_zero() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_query_opt_metricopt").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let mut conn = pool.get().await?;
    conn.batch_execute("CREATE TABLE widget (id int primary key)")
        .await?;
    conn.execute("INSERT INTO widget (id) VALUES (1)", &[])
        .await?;

    let hit = conn.query_opt("SELECT id FROM widget WHERE id = 1", &[]);
    assert!(hit.await?.is_some());
    let miss = conn.query_opt("SELECT id FROM widget WHERE id = 2", &[]);
    assert!(miss.await?.is_none());

    conn.transaction(async |txn| {
        assert!(
            txn.query_opt("SELECT id FROM widget WHERE id = 1", &[])
                .await?
                .is_some()
        );
        assert!(
            txn.query_opt("SELECT id FROM widget WHERE id = 2", &[])
                .await?
                .is_none()
        );
        Ok::<_, Error>(())
    })
    .await
    .expect("the instrumented transaction commits");

    assert_eq!(
        metrics.count("query_success:query_opt:Some(1)"),
        2,
        "a row found is one row through either wrapper: {:?}",
        metrics.events()
    );
    assert_eq!(
        metrics.count("query_success:query_opt:Some(0)"),
        2,
        "no row found is zero rows, not an absent count: {:?}",
        metrics.events()
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

mod widget {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "widget")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

async fn widget_count<C: ConnectionTrait>(db: &C) -> Result<i64, Error> {
    Ok(db
        .query_one("SELECT count(*) FROM widget", &[])
        .await?
        .get(0))
}

// [spec:pgorm:req:metric.layer.delegate+5/test]    an instrumented closure's handle is passed as `&txn`
// [spec:pgorm:def:conn.pool.conn-trait+9/test]    every closure's `&mut` handle is a connection
#[pgorm_macros::test]
pub async fn instrumented_handle_is_a_connection() -> Result<(), Error> {
    use pgorm::EntityTrait;

    let ctx = TestContext::new("metric_layer_handle_metrichandle").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let mut conn = pool.get().await?;
    conn.batch_execute(
        "CREATE TABLE widget (id int primary key); INSERT INTO widget VALUES (1), (2)",
    )
    .await?;

    let (found, counted) = conn
        .transaction(async |txn| {
            let found = widget::Entity::find().all(&txn).await?;
            let counted = txn
                .transaction(async |savepoint| widget_count(&savepoint).await)
                .await
                .expect("the savepoint commits");
            Ok::<_, Error>((found.len(), counted))
        })
        .await
        .expect("the instrumented transaction commits");
    assert_eq!((found, counted), (2, 2));

    assert_eq!(
        metrics.count("query_success:query_all:Some(2)"),
        1,
        "the ORM read through `&txn` is the instrumented handle's: {:?}",
        metrics.events()
    );
    assert_eq!(
        metrics.count("query_success:query_one:Some(1)"),
        1,
        "the helper given `&savepoint` ran on the instrumented savepoint: {:?}",
        metrics.events()
    );
    assert_eq!(metrics.count("transaction_begin"), 2);
    assert_eq!(metrics.count("transaction_commit"), 2);

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:metric.layer.delegate+5/test]    a cache hit is still a reported call
// [spec:pgorm:def:conn.pool.conn-trait+9/test]    the wrapper inherits the routing it delegates to
#[pgorm_macros::test]
pub async fn cached_statements_still_report_each_call() -> Result<(), Error> {
    const SQL: &str = "SELECT id FROM widget WHERE id = 1";

    let ctx = TestContext::new("metric_layer_cached_metriccache").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let conn = pool.get().await?;
    conn.batch_execute("CREATE TABLE widget (id int primary key)")
        .await?;
    conn.execute("INSERT INTO widget (id) VALUES (1)", &[])
        .await?;

    for _ in 0..3 {
        assert_eq!(conn.query_all(SQL, &[]).await?.len(), 1);
    }

    assert_eq!(
        metrics.count("query_success:query_all:Some(1)"),
        3,
        "every call reports, whether it parsed a statement or reused one: {:?}",
        metrics.events()
    );
    assert!(
        metrics
            .queries()
            .iter()
            .filter(|query| query.operation == "query_all")
            .all(|query| query.sql.as_deref() == Some(SQL)),
        "the context still carries the caller's own text"
    );

    let prepared: i64 = conn
        .query_one(
            "SELECT count(*) FROM pg_prepared_statements WHERE statement = $1",
            &[&SQL],
        )
        .await?
        .get(0);
    assert_eq!(prepared, 1, "three reported calls, one parse");

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:metric.layer.delegate+5/test]    batch_execute reports a rowless success
#[pgorm_macros::test]
pub async fn instrumented_batch_execute_reports_no_rows() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_batch_execute_txbatch").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let mut conn = pool.get().await?;
    conn.batch_execute(
        "CREATE TABLE widget (id int primary key);
         INSERT INTO widget (id) VALUES (1);",
    )
    .await?;

    conn.transaction(async |txn| {
        txn.batch_execute("INSERT INTO widget (id) VALUES (2); INSERT INTO widget (id) VALUES (3);")
            .await
    })
    .await
    .expect("the instrumented transaction commits");

    assert_eq!(conn.query_all("SELECT id FROM widget", &[]).await?.len(), 3);

    assert_eq!(
        metrics.count("query_success:batch_execute:None"),
        2,
        "batch_execute reports no row count through either wrapper: {:?}",
        metrics.events()
    );
    assert_eq!(metrics.count("query_error"), 0);

    let error = conn
        .batch_execute("CREATE TABLE widget (id int primary key);")
        .await
        .expect_err("the table already exists");
    assert!(matches!(error, Error::Postgres(_)));
    assert_eq!(metrics.count("query_error:batch_execute"), 1);

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:metric.layer.tx+5/test]    a dropped closure future records no ending
#[pgorm_macros::test]
pub async fn dropped_instrumented_transaction_records_nothing() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_drop_metrictx").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let mut conn = pool.get().await?;
    conn.execute("CREATE TABLE widget (id int primary key)", &[])
        .await?;

    let (inserted, ready) = tokio::sync::oneshot::channel();
    let transaction = conn.transaction(async |txn| {
        txn.execute("INSERT INTO widget (id) VALUES (1)", &[])
            .await?;
        inserted.send(()).expect("the test is waiting");
        std::future::pending::<Result<(), Error>>().await
    });
    tokio::select! {
        outcome = transaction => panic!("the transaction cannot finish: {outcome:?}"),
        ready = ready => ready.expect("the closure signals its insert"),
    }

    assert_eq!(metrics.count("transaction_begin"), 1);
    assert_eq!(metrics.count("transaction_rollback"), 0);
    assert_eq!(metrics.count("transaction_commit"), 0);
    assert_eq!(conn.query_all("SELECT id FROM widget", &[]).await?.len(), 0);

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:metric.layer.tx+5/test]    a savepoint inside an instrumented transaction reports its own begin and ending
#[pgorm_macros::test]
pub async fn instrumented_savepoints_report_their_ends() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_savepoint_metrictx").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let mut conn = pool.get().await?;
    conn.execute("CREATE TABLE widget (id int primary key)", &[])
        .await?;

    conn.transaction(async |txn| {
        txn.transaction(async |savepoint| {
            savepoint
                .execute("INSERT INTO widget (id) VALUES (1)", &[])
                .await
        })
        .await
        .expect("the savepoint is released");
        let outcome = txn
            .transaction(async |savepoint| {
                savepoint
                    .execute("INSERT INTO widget (id) VALUES (2)", &[])
                    .await?;
                roll_back::<()>()
            })
            .await;
        assert_rolled_back(outcome);
        Ok::<_, Error>(())
    })
    .await
    .expect("the instrumented transaction commits");

    assert_eq!(conn.query_all("SELECT id FROM widget", &[]).await?.len(), 1);
    assert_eq!(metrics.count("transaction_begin"), 3);
    assert_eq!(metrics.count("transaction_commit"), 2);
    assert_eq!(metrics.count("transaction_rollback"), 1);
    assert_eq!(
        metrics.count("query_success:execute:Some(1)"),
        2,
        "statements inside a savepoint stay instrumented: {:?}",
        metrics.events()
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:metric.layer.tx+5/test]    an instrumented transaction runs in a spawned task
#[pgorm_macros::test]
pub async fn instrumented_transaction_runs_in_spawned_task() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_spawned_metrictx").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    tokio::spawn(async move {
        let mut conn = pool.get().await?;
        conn.execute("CREATE TABLE widget (id int primary key)", &[])
            .await?;
        conn.transaction(async |txn| {
            txn.transaction(async |savepoint| {
                savepoint
                    .execute("INSERT INTO widget (id) VALUES (1)", &[])
                    .await
            })
            .await
            .map_err(|e| Error::Custom(e.to_string()))
        })
        .await
        .map_err(|e| Error::Custom(e.to_string()))
    })
    .await
    .expect("the spawned task completes")?;

    assert_eq!(metrics.count("transaction_begin"), 2);
    assert_eq!(metrics.count("transaction_commit"), 2);

    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:metric.layer.tx+5/test]    a configured transaction on an instrumented connection is reported, its mode applied, in a spawned task
#[pgorm_macros::test]
pub async fn instrumented_transaction_with_mode_is_reported() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_tx_with_metrictx").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let refused = tokio::spawn(async move {
        let mut conn = pool.get().await?;
        conn.execute("CREATE TABLE widget (id int primary key)", &[])
            .await?;
        let read_only = pgorm::TransactionMode::ReadOnly { isolation: None };
        let refused = conn
            .transaction_with(read_only, async |txn| {
                txn.execute("INSERT INTO widget (id) VALUES (1)", &[]).await
            })
            .await;
        let shown = conn
            .transaction_with(read_only, async |txn| {
                txn.query_one("SHOW transaction_read_only", &[]).await
            })
            .await
            .map_err(|e| Error::Custom(e.to_string()))?;
        assert_eq!(shown.get::<_, String>(0), "on");
        Ok::<_, Error>(refused.is_err())
    })
    .await
    .expect("the spawned task completes")?;

    assert!(refused, "a read-only transaction refuses the write");
    assert_eq!(
        metrics.events(),
        vec![
            "connection_acquired",
            "query_success:execute:Some(0)",
            "transaction_begin",
            "query_error:execute",
            "transaction_rollback",
            "transaction_begin",
            "query_success:query_one:Some(1)",
            "transaction_commit",
        ]
    );

    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:metric.layer.tx+5/test]    every attempt a retried transaction makes is reported as a transaction of its own
#[pgorm_macros::test]
pub async fn instrumented_transaction_with_retry_reports_each_attempt() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_tx_retry_metrictx").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());
    let mut conn = pool.get().await?;

    let serializable = pgorm::TransactionMode::ReadWrite {
        isolation: Some(pgorm::IsolationLevel::Serializable),
    };
    let mut attempts = 0;
    let answer = conn
        .transaction_with_retry(serializable, 3, async |txn| {
            attempts += 1;
            if attempts < 3 {
                // SQLSTATE 40001, which `is_retryable` takes for a
                // serialization failure worth replaying.
                txn.execute(
                    "DO $$ BEGIN RAISE EXCEPTION 'conflict' USING ERRCODE = '40001'; END $$",
                    &[],
                )
                .await?;
            }
            let row = txn.query_one("SELECT 42", &[]).await?;
            Ok::<_, Error>(row.get::<_, i32>(0))
        })
        .await
        .map_err(|e| Error::Custom(e.to_string()))?;

    assert_eq!(answer, 42);
    assert_eq!(attempts, 3);
    assert_eq!(metrics.count("transaction_begin"), 3);
    assert_eq!(metrics.count("transaction_rollback"), 2);
    assert_eq!(metrics.count("transaction_commit"), 1);
    assert_eq!(metrics.count("query_error:execute"), 2);

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:metric.layer+3/test]    into_inner gives the connection back, and what runs on it is unreported
#[pgorm_macros::test]
pub async fn into_inner_gives_back_an_unreported_connection() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_into_inner_metrictx").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let conn = pool.get().await?.into_inner();
    conn.query_one("SELECT 1", &[]).await?;

    assert_eq!(metrics.events(), vec!["connection_acquired"]);

    drop(conn);
    ctx.delete().await;

    Ok(())
}

/// Collects `(level, message)` for every `tracing` event emitted while installed.
#[derive(Clone, Default)]
struct CapturedEvents(Arc<Mutex<Vec<(Level, String)>>>);

impl CapturedEvents {
    fn taken(&self) -> Vec<(Level, String)> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S> Layer<S> for CapturedEvents
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap()
            .push((*event.metadata().level(), visitor.0));
    }
}

/// Drives all seven hooks of a collector, in declaration order.
async fn drive_all_hooks<M: MetricsCollector>(metrics: &M) {
    let elapsed = Duration::from_millis(1);
    let error = Error::Custom("boom".to_owned());
    let query = QueryContext::new("execute", Some("SELECT 1"));

    metrics.record_query_success(query, elapsed, Some(3)).await;
    metrics.record_query_error(query, elapsed, &error).await;
    metrics.record_connection_acquired(elapsed).await;
    metrics.record_connection_error(elapsed, &error).await;
    metrics.record_transaction_begin(elapsed).await;
    metrics.record_transaction_commit(elapsed).await;
    metrics.record_transaction_rollback(elapsed).await;
}

// [spec:pgorm:def:metric.layer.collector+3/test]    seven hook points, all required of an implementor
#[pgorm_macros::test]
pub async fn collector_defines_seven_async_hooks() {
    let metrics = RecordingMetrics::default();
    drive_all_hooks(&metrics).await;

    assert_eq!(
        metrics.events(),
        vec![
            "query_success:execute:Some(3)".to_owned(),
            "query_error:execute".to_owned(),
            "connection_acquired".to_owned(),
            "connection_error".to_owned(),
            "transaction_begin".to_owned(),
            "transaction_commit".to_owned(),
            "transaction_rollback".to_owned(),
        ],
        "every hook dispatches to the implementor"
    );
}

// [spec:pgorm:def:metric.layer.collector+3/test]    NoOpMetrics observes nothing at all
#[pgorm_macros::test]
pub async fn noop_metrics_hooks_do_nothing() {
    let captured = CapturedEvents::default();
    let guard = tracing_subscriber::registry()
        .with(captured.clone())
        .set_default();

    drive_all_hooks(&NoOpMetrics).await;
    let events = captured.taken();

    drop(guard);
    assert_eq!(
        events,
        Vec::new(),
        "the no-op collector's hooks have empty bodies"
    );
}

// [spec:pgorm:def:metric.layer.collector+3/test]    LoggingMetrics' tracing levels per hook
#[pgorm_macros::test]
pub async fn logging_metrics_emits_expected_levels() {
    let captured = CapturedEvents::default();
    let guard = tracing_subscriber::registry()
        .with(captured.clone())
        .set_default();

    drive_all_hooks(&LoggingMetrics).await;
    // A success with no row count takes the other branch of the same hook.
    LoggingMetrics
        .record_query_success(
            QueryContext::new("query_raw", None),
            Duration::from_millis(1),
            None,
        )
        .await;
    let events = captured.taken();

    drop(guard);

    let levels: Vec<Level> = events.iter().map(|(level, _)| *level).collect();
    assert_eq!(
        levels,
        vec![
            Level::DEBUG, // query success
            Level::WARN,  // query error
            Level::DEBUG, // connection acquired
            Level::ERROR, // connection error
            Level::DEBUG, // transaction begin
            Level::DEBUG, // transaction commit
            Level::WARN,  // transaction rollback
            Level::DEBUG, // query success, no row count
        ],
        "{events:?}"
    );
    assert!(events[0].1.contains("3 rows"), "{:?}", events[0]);
    assert!(events[1].1.contains("execute"), "{:?}", events[1]);
    assert!(!events[7].1.contains("rows"), "{:?}", events[7]);
}

// [spec:pgorm:def:metric.layer+3/test]    three wrappers, their inner()/metrics() accessors, and the pool's
// tag()/status()/clear_statement_caches()
// [spec:pgorm:req:conn.pool.statement-cache.clear+1/test]    the instrumented pool clears the pool it wraps
#[pgorm_macros::test]
pub async fn wrappers_expose_inner_and_metrics() -> Result<(), Error> {
    fn assert_collector_bounds<M: MetricsCollector>() {
        fn assert_bounds<T: Clone + Send + Sync + 'static>() {}
        assert_bounds::<M>();
    }
    assert_collector_bounds::<RecordingMetrics>();
    assert_collector_bounds::<NoOpMetrics>();
    assert_collector_bounds::<LoggingMetrics>();

    let ctx = TestContext::new("metric_layer_accessors_metricacc").await;
    let metrics = RecordingMetrics::default();
    let pool: InstrumentedPool<RecordingMetrics> =
        InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    // The pool wrapper forwards tag() and status(), and hands back both the wrapped
    // pool and the collector.
    assert_eq!(pool.tag(), ctx.db.tag());
    assert_eq!(pool.status().max_size, pool.inner().status().max_size);
    assert_eq!(pool.inner().status().size, ctx.db.status().size);
    metrics.push("probe".to_owned());
    assert_eq!(pool.metrics().events(), metrics.events());

    let mut conn: InstrumentedConnection<RecordingMetrics> = pool.get().await?;
    assert_eq!(metrics.count("connection_acquired"), 1);
    assert_eq!(conn.metrics().events(), metrics.events());

    // inner() reaches the wrapped connection, which carries no metrics hooks.
    let before = metrics.events().len();
    conn.inner()
        .execute("CREATE TABLE widget (id int primary key)", &[])
        .await?;
    assert_eq!(
        metrics.events().len(),
        before,
        "the unwrapped connection records nothing: {:?}",
        metrics.events()
    );
    conn.execute("INSERT INTO widget (id) VALUES (1)", &[])
        .await?;
    assert_eq!(metrics.count("query_success:execute"), 1);

    conn.transaction(
        async |txn: &mut InstrumentedTransaction<'_, RecordingMetrics>| {
            assert_eq!(txn.metrics().events(), metrics.events());
            let before = metrics.events().len();
            txn.inner()
                .execute("INSERT INTO widget (id) VALUES (2)", &[])
                .await?;
            assert_eq!(
                metrics.events().len(),
                before,
                "the unwrapped transaction records nothing: {:?}",
                metrics.events()
            );
            Ok::<_, Error>(())
        },
    )
    .await
    .expect("the instrumented transaction commits");

    assert_eq!(conn.query_all("SELECT id FROM widget", &[]).await?.len(), 2);
    let prepared =
        "SELECT count(*) FROM pg_prepared_statements WHERE statement = 'SELECT id FROM widget'";
    assert_eq!(conn.query_one(prepared, &[]).await?.get::<_, i64>(0), 1);

    drop(conn);
    pool.clear_statement_caches();
    let conn = pool.get().await?;
    assert_eq!(conn.query_one(prepared, &[]).await?.get::<_, i64>(0), 0);

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:conn.sql-text+2/test]    what each statement form answers
#[pgorm_macros::test]
pub async fn sql_text_answers_with_the_statement_text() {
    let owned = "SELECT id FROM widget WHERE id = $1".to_owned();

    assert_eq!("SELECT 1".sql_text(), "SELECT 1");
    assert_eq!(owned.sql_text(), owned.as_str());

    // The bound `ConnectionTrait` puts on its statement. A prepared
    // `Statement` falls outside it, which the `compile_fail` doctest on
    // `ConnectionTrait` holds.
    fn assert_statement_bound<T: ?Sized + SqlText + Sync>() {}
    assert_statement_bound::<str>();
    assert_statement_bound::<String>();
}

// [spec:pgorm:req:metric.fingerprint+3/test]    the hooks see the statement they report on
#[pgorm_macros::test]
pub async fn query_context_carries_statement_text() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_context_metricctx").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let conn = pool.get().await?;
    conn.batch_execute("CREATE TABLE widget (id int primary key)")
        .await?;
    conn.execute("INSERT INTO widget (id) VALUES ($1)", &[&1i32])
        .await?;
    let failed = conn
        .query_one("SELECT id FROM widget WHERE id = $1", &[&404i32])
        .await
        .expect_err("no row matches");
    assert!(matches!(failed, Error::Postgres(_)));

    let recorded: Vec<(String, Option<String>)> = metrics
        .queries()
        .into_iter()
        .map(|query| (query.operation, query.sql))
        .collect();
    assert_eq!(
        recorded,
        vec![
            (
                "batch_execute".to_owned(),
                Some("CREATE TABLE widget (id int primary key)".to_owned())
            ),
            (
                "execute".to_owned(),
                Some("INSERT INTO widget (id) VALUES ($1)".to_owned())
            ),
            (
                "query_one".to_owned(),
                Some("SELECT id FROM widget WHERE id = $1".to_owned())
            ),
        ],
        "success and failure hooks alike name the statement"
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:metric.fingerprint+3/test]    libpg_query's canonical hex rendering
#[cfg(feature = "metrics-fingerprint")]
#[pgorm_macros::test]
pub async fn fingerprint_renders_libpg_query_hex() {
    let query = QueryContext::new(
        "query_all",
        Some("SELECT * FROM contacts WHERE name='Paul'"),
    );
    // Named through its public path: both types live in `pgorm::metric`,
    // whatever module inside the crate defines them.
    let fingerprint: pgorm::metric::QueryFingerprint =
        query.fingerprint().expect("the statement parses");

    assert_eq!(
        fingerprint.to_string(),
        "0e2581a461ece536",
        "the 16-character zero-padded hex libpg_query itself reports"
    );
    assert_eq!(
        format!("{:016x}", fingerprint.value()),
        fingerprint.to_string(),
        "value() and Display are two views of one number"
    );
    assert_eq!(
        query.fingerprint(),
        Some(fingerprint),
        "a second look at the same statement answers from the memo"
    );
}

// [spec:pgorm:req:metric.fingerprint+3/test]    constants are normalized away
#[cfg(feature = "metrics-fingerprint")]
#[pgorm_macros::test]
pub async fn fingerprint_ignores_literal_values() {
    let one = QueryContext::new("query_all", Some("SELECT id FROM widget WHERE id = 1"));
    let other = QueryContext::new("query_all", Some("SELECT id FROM widget WHERE id = 4242"));
    let spaced = QueryContext::new("execute", Some("SELECT id  FROM widget\n WHERE id = 7"));

    assert!(one.fingerprint().is_some(), "the statement parses");
    assert_eq!(
        one.fingerprint(),
        other.fingerprint(),
        "only the literal differs, so the shape is the same query"
    );
    assert_eq!(
        one.fingerprint(),
        spaced.fingerprint(),
        "a parse-tree hash is blind to whitespace and to the reporting operation"
    );
}

// [spec:pgorm:req:metric.fingerprint+3/test]    different shapes stay apart
#[cfg(feature = "metrics-fingerprint")]
#[pgorm_macros::test]
pub async fn distinct_shapes_get_distinct_fingerprints() {
    let select = QueryContext::new("query_all", Some("SELECT id FROM widget WHERE id = 1"));
    let column = QueryContext::new("query_all", Some("SELECT name FROM widget WHERE id = 1"));
    let table = QueryContext::new("query_all", Some("SELECT id FROM gadget WHERE id = 1"));
    let insert = QueryContext::new("execute", Some("INSERT INTO widget (id) VALUES (1)"));

    let prints = [select, column, table, insert].map(|query| {
        query
            .fingerprint()
            .expect("every one of these statements parses")
    });

    let mut distinct = prints.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        prints.len(),
        "a different column, table, or verb is a different query: {prints:?}"
    );
}

// [spec:pgorm:req:metric.fingerprint+3/test]    a fingerprint groups statements
// by PostgreSQL 18's query-ID rules: a schema qualifier is not part of the
// identity, and an alias stands for the relation it names
#[cfg(feature = "metrics-fingerprint")]
#[pgorm_macros::test]
pub async fn fingerprint_follows_postgres_18_query_ids() {
    let print = |sql| {
        QueryContext::new("query_all", Some(sql))
            .fingerprint()
            .expect("the statement parses")
    };

    // The guide's Metrics chapter shows the first of these: statements with no
    // alias, no schema and no RETURNING, fingerprinted as libpg_query 17 did.
    assert_eq!(
        print("SELECT id FROM widget WHERE id = 1").to_string(),
        "394a2f90c244bffe"
    );
    assert_eq!(
        print("SELECT name FROM widget WHERE id = 1").to_string(),
        "f7678147685fe197"
    );
    assert_eq!(
        print("SELECT id FROM widget WHERE id = 1"),
        print("SELECT id FROM public.widget WHERE id = 1"),
        "a schema-qualified relation is the same query as an unqualified one"
    );
    assert_eq!(
        print("SELECT id FROM widget w WHERE id = 1"),
        print("SELECT id FROM gadget w WHERE id = 1"),
        "an aliased relation is identified by its alias"
    );
    assert_ne!(
        print("SELECT id FROM widget w WHERE id = 1"),
        print("SELECT id FROM widget g WHERE id = 1"),
        "so the same table under two aliases is two queries"
    );
}

// [spec:pgorm:req:metric.fingerprint+3/test]    unidentifiable is not an error
#[cfg(feature = "metrics-fingerprint")]
#[pgorm_macros::test]
pub async fn unparseable_sql_has_no_fingerprint() {
    let broken = QueryContext::new("batch_execute", Some("SELECT FROM WHERE (("));
    let empty = QueryContext::new("execute", Some(""));
    let prepared = QueryContext::new("execute", None);

    assert_eq!(
        broken.fingerprint(),
        None,
        "text the grammar rejects has no identity, and asking is not an error"
    );
    assert_eq!(
        broken.sql(),
        Some("SELECT FROM WHERE (("),
        "the text is still reported; only its identity is missing"
    );
    assert_eq!(
        prepared.fingerprint(),
        None,
        "nor does a statement whose text the driver no longer holds"
    );
    assert!(
        empty.fingerprint().is_some(),
        "an empty statement is an empty parse, not a rejected one"
    );
}

// [spec:pgorm:req:metric.fingerprint+3/test]    fingerprints reach the collector through the wrappers
#[cfg(feature = "metrics-fingerprint")]
#[pgorm_macros::test]
pub async fn instrumented_wrapper_reports_fingerprints() -> Result<(), Error> {
    let ctx = TestContext::new("metric_layer_fingerprint_metricfp").await;
    let metrics = RecordingMetrics::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());

    let conn = pool.get().await?;
    conn.batch_execute("CREATE TABLE widget (id int primary key)")
        .await?;
    conn.execute("INSERT INTO widget (id) VALUES (1)", &[])
        .await?;
    conn.execute("INSERT INTO widget (id) VALUES (2)", &[])
        .await?;
    conn.query_all("SELECT id FROM widget", &[]).await?;

    let first = metrics.fingerprint_of("INSERT INTO widget (id) VALUES (1)");
    let second = metrics.fingerprint_of("INSERT INTO widget (id) VALUES (2)");
    let select = metrics.fingerprint_of("SELECT id FROM widget");

    assert!(first.is_some(), "{:?}", metrics.queries());
    assert_eq!(
        first, second,
        "two inserts differing only in their value are one query shape"
    );
    assert_ne!(first, select, "the select is not that shape");

    drop(conn);
    ctx.delete().await;

    Ok(())
}
