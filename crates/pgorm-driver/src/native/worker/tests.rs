use super::*;
use std::path::Path;

const LONG: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT sum(x) FROM n";

pub(super) async fn actor() -> (Arc<Client>, JoinHandle<Result<(), Error>>) {
    let database = super::super::Database::open(Path::new(":memory:"))
        .await
        .unwrap();
    let (client, connection) = database.connect().await.unwrap();
    (client, tokio::spawn(connection))
}

pub(super) fn gate() -> (StepGate, oneshot::Receiver<()>, std::sync::mpsc::Sender<()>) {
    let (entered, ready) = oneshot::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    (
        StepGate {
            entered,
            resume: paused,
        },
        ready,
        resume,
    )
}

pub(super) async fn finish(client: Arc<Client>, task: JoinHandle<Result<(), Error>>) {
    drop(client);
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

// [spec:pgorm:sem:turso.lifecycle/test] cancellation between check and first VM step is latched
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_before_first_native_step_is_not_lost() {
    let (client, task) = actor().await;
    let (gate, entered, resume) = gate();
    *client.control.before_step.lock().unwrap() = Some(gate);
    let op_client = Arc::clone(&client);
    let operation = tokio::spawn(async move { op_client.batch_execute(LONG).await });
    entered.await.unwrap();
    // Connection::interrupt ignores this idle native root; only the
    // progress-handler latch can cancel the impending CPU-heavy step.
    client.cancel();
    resume.send(()).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(3), operation)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code().unwrap().code(), "57014");
    client.check_connection().await.unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.lifecycle/test] interrupted RETURNING must terminate before reset can commit
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_returning_does_not_commit_during_reset() {
    let (client, task) = actor().await;
    client
        .batch_execute("CREATE TABLE writes(id integer PRIMARY KEY)")
        .await
        .unwrap();
    let (gate, entered, resume) = gate();
    *client.control.after_row.lock().unwrap() = Some(gate);
    let op_client = Arc::clone(&client);
    let operation = tokio::spawn(async move {
        op_client
            .batch_execute("INSERT INTO writes VALUES(1) RETURNING id")
            .await
    });
    entered.await.unwrap();
    client.cancel();
    resume.send(()).unwrap();
    assert_eq!(
        operation.await.unwrap().unwrap_err().code().unwrap().code(),
        "57014"
    );
    // A commit hidden in reset would cause a duplicate-key violation.
    client
        .batch_execute("INSERT INTO writes VALUES(1)")
        .await
        .unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.lifecycle/test] cleanup failure preserves primary cause and forbids reuse
#[tokio::test]
async fn cleanup_failure_quarantines_and_keeps_primary_error() {
    let (client, task) = actor().await;
    client
        .batch_execute("CREATE TABLE writes(id integer PRIMARY KEY); INSERT INTO writes VALUES(1)")
        .await
        .unwrap();
    client
        .control
        .fail_next_cleanup
        .store(true, Ordering::Release);
    let error = client
        .batch_execute("INSERT INTO writes VALUES(1)")
        .await
        .unwrap_err();
    assert_eq!(error.code().unwrap().code(), "23505");
    assert!(client.is_closed());
    assert!(client.check_connection().await.unwrap_err().is_closed());
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.lifecycle/test] unexpected worker termination never becomes successful completion
#[tokio::test]
async fn worker_panic_fails_active_request() {
    let (client, task) = actor().await;
    client
        .control
        .panic_next_step
        .store(true, Ordering::Release);
    assert!(client.check_connection().await.unwrap_err().is_closed());
    assert!(client.is_closed());
    drop(client);
    assert!(task.await.unwrap().is_err());
}

// [spec:pgorm:sem:turso.lifecycle/test] a stale request drop cannot cancel another operation
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_cancellation_does_not_affect_current_operation() {
    let (client, task) = actor().await;
    let (gate, entered, resume) = gate();
    *client.control.before_step.lock().unwrap() = Some(gate);
    let op_client = Arc::clone(&client);
    let operation = tokio::spawn(async move { op_client.check_connection().await });
    entered.await.unwrap();
    let stale = Operation::detached(u64::MAX);
    client.control.cancel(Some(&stale));
    resume.send(()).unwrap();
    operation.await.unwrap().unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.lifecycle/test] pending native engine waits observe cancellation
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_native_completion_is_aborted_on_cancel() {
    let database = super::super::Database::open(Path::new(":memory:"))
        .await
        .unwrap();
    let native = PgConnection::connect(&database.inner).unwrap();
    let statement = native.prepare("SELECT 1").unwrap();
    let (client, lifetime) = database.connect().await.unwrap();
    let task = tokio::spawn(lifetime);
    let operation = Arc::new(Operation::detached(u64::MAX));
    let completion = turso_core::Completion::new_wait();
    let pending = completion.clone();
    let operation_wait = Arc::clone(&operation);
    let control_wait = Arc::clone(&client.control);
    let waiting = tokio::task::spawn_blocking(move || {
        wait_completion(
            &statement,
            IOCompletions(pending),
            &operation_wait,
            &control_wait,
        )
    });
    client.control.cancel(Some(&operation));
    let error = tokio::time::timeout(Duration::from_secs(3), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code().unwrap().code(), "57014");
    assert!(completion.finished());
    native.close().unwrap();
    finish(client, task).await;
}

const MANY_ROWS: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT x::bigint FROM n";

// [spec:pgorm:req:turso.rows/test] bounded production and row ownership work through the common stream
// [spec:pgorm:sem:turso.streaming/test]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepared_stream_backpressure_and_drop() {
    let (actor, task) = actor().await;
    let client = crate::Client::from_turso(Arc::clone(&actor));
    let rows = client
        .query_raw(MANY_ROWS, std::iter::empty::<&i32>())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while actor.control.produced_rows.load(Ordering::Acquire) < (rows::BATCH_ROWS * 2) as u64 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        actor.control.produced_rows.load(Ordering::Acquire),
        (rows::BATCH_ROWS * 2) as u64
    );
    drop(rows);
    tokio::time::timeout(Duration::from_secs(3), client.check_connection())
        .await
        .unwrap()
        .unwrap();
    drop(client);
    finish(actor, task).await;
}

// [spec:pgorm:req:turso.compatibility/test]
// [spec:pgorm:sem:turso.streaming/test] byte batches also bound wide and oversized rows
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wide_rows_obey_byte_backpressure() {
    use futures_util::TryStreamExt;
    for (width, expected) in [
        (40 * 1024, 4_u64),
        (65535, 4),
        (65536, 2),
        (65537, 2),
        (128 * 1024, 2),
    ] {
        let (actor, task) = actor().await;
        let client = crate::Client::from_turso(Arc::clone(&actor));
        let payload = vec![0xab_u8; width];
        let statement = client.prepare_typed(
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT $1::bytea FROM n",
            &[crate::types::Type::BYTEA],
        ).await.unwrap();
        let mut rows = Box::pin(client.query_raw(&statement, [&payload]).await.unwrap());
        tokio::time::timeout(Duration::from_secs(3), async {
            while actor.control.produced_rows.load(Ordering::Acquire) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            actor.control.produced_rows.load(Ordering::Acquire),
            expected
        );
        let retained = rows.try_next().await.unwrap().unwrap();
        assert_eq!(retained.raw_size_bytes(), width);
        drop(rows);
        tokio::time::timeout(Duration::from_secs(3), client.check_connection())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained.get::<_, &[u8]>(0), payload);
        drop(client);
        finish(actor, task).await;
    }
}

// [spec:pgorm:req:turso.rows/test] a failed producer cannot turn a partial stream into EOF
// [spec:pgorm:sem:turso.streaming/test]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepared_stream_reports_worker_loss() {
    use futures_util::TryStreamExt;
    let (actor, task) = actor().await;
    let client = crate::Client::from_turso(Arc::clone(&actor));
    let (gate, entered, resume) = gate();
    *actor.control.before_step.lock().unwrap() = Some(gate);
    actor.control.panic_next_step.store(true, Ordering::Release);
    let mut rows = Box::pin(
        client
            .query_raw("SELECT 1", std::iter::empty::<&i32>())
            .await
            .unwrap(),
    );
    entered.await.unwrap();
    resume.send(()).unwrap();
    assert!(rows.try_next().await.unwrap_err().is_closed());
    assert!(client.is_closed());
    drop(rows);
    drop(client);
    drop(actor);
    assert!(task.await.unwrap().is_err());
}

// [spec:pgorm:sem:turso.lifecycle/test] a retained stream cannot keep the last client alive
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn last_client_drop_stops_unpolled_stream() {
    let (actor, task) = actor().await;
    let client = crate::Client::from_turso(actor);
    let rows = client
        .query_raw(MANY_ROWS, std::iter::empty::<&i32>())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    drop(client);
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(rows);
}

async fn stale_schema_refusal(other_connection: bool) {
    use crate::types::Type;

    let database = super::super::Database::open(Path::new(":memory:"))
        .await
        .unwrap();
    let (actor, lifetime) = database.connect().await.unwrap();
    let task = tokio::spawn(lifetime);
    let client = crate::Client::from_turso(Arc::clone(&actor));
    client
        .batch_execute("CREATE TABLE changed(id integer PRIMARY KEY)")
        .await
        .unwrap();
    let read = client.prepare("SELECT id FROM changed").await.unwrap();
    assert!(!read.is_invalidated());
    let write = client
        .prepare_typed("INSERT INTO changed VALUES($1) RETURNING id", &[Type::INT4])
        .await
        .unwrap();
    let execute = client
        .prepare_typed("INSERT INTO changed VALUES($1) RETURNING id", &[Type::INT4])
        .await
        .unwrap();
    let ddl = "DROP TABLE changed; CREATE TABLE changed(id bigint PRIMARY KEY)";
    if other_connection {
        let (other, lifetime) = database.connect().await.unwrap();
        let other_task = tokio::spawn(lifetime);
        other.batch_execute(ddl).await.unwrap();
        finish(other, other_task).await;
    } else {
        client.batch_execute(ddl).await.unwrap();
    }
    let before_write = actor.control.produced_rows.load(Ordering::Acquire);
    // Try writes before any SELECT can make the engine adopt/reprepare its
    // schema. Neither execute nor query may reach a native RETURNING row.
    let error = client.execute(&execute, &[&2_i32]).await.unwrap_err();
    assert!(error.is_statement_invalidated(), "{error}");
    assert!(execute.is_invalidated());
    let error = client.query(&write, &[&1_i32]).await.unwrap_err();
    assert!(error.is_statement_invalidated(), "{error}");
    assert!(write.is_invalidated());
    assert_eq!(
        actor.control.produced_rows.load(Ordering::Acquire),
        before_write
    );
    let error = client.query(&read, &[]).await.unwrap_err();
    assert!(error.is_statement_invalidated(), "{error}");
    assert!(read.is_invalidated());
    assert!(error.code().is_none());
    let row = client
        .query_one("SELECT count(*) FROM changed", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 0, "preflight must not execute writes");
    let produced = actor.control.produced_rows.load(Ordering::Acquire);
    let fresh = client.prepare("SELECT id FROM changed").await.unwrap();
    assert_eq!(*fresh.columns()[0].type_(), Type::INT8);
    assert!(client.query(&fresh, &[]).await.unwrap().is_empty());
    assert!(client.query(&fresh, &[]).await.unwrap().is_empty());
    assert_eq!(
        actor.control.produced_rows.load(Ordering::Acquire),
        produced,
        "matching schema and cached execution must not run a synchronization read"
    );
    drop(client);
    finish(actor, task).await;
}

// [spec:pgorm:req:turso.rows/test] stale metadata is refused before a known-invalid write starts
#[tokio::test]
async fn same_connection_invalidates_metadata() {
    stale_schema_refusal(false).await;
}

// [spec:pgorm:req:turso.rows/test] preflight refreshes schema published by another native connection
#[tokio::test]
async fn other_connection_invalidates_metadata() {
    stale_schema_refusal(true).await;
}

// [spec:pgorm:req:turso.rows/test] a reprepare racing preflight cannot expose a stale row or empty result
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn racing_reprepare_cannot_expose_old_metadata() {
    for include_row in [false, true] {
        let database = super::super::Database::open(Path::new(":memory:"))
            .await
            .unwrap();
        let (actor, lifetime) = database.connect().await.unwrap();
        let task = tokio::spawn(lifetime);
        let client = crate::Client::from_turso(Arc::clone(&actor));
        client
            .batch_execute("CREATE TABLE changed(id integer)")
            .await
            .unwrap();
        let statement = client.prepare("SELECT id FROM changed").await.unwrap();
        let (gate, entered, resume) = gate();
        *actor.control.before_step.lock().unwrap() = Some(gate);
        let mut rows = Box::pin(
            client
                .query_raw(&statement, std::iter::empty::<&i32>())
                .await
                .unwrap(),
        );
        entered.await.unwrap();
        let (other, lifetime) = database.connect().await.unwrap();
        let other_task = tokio::spawn(lifetime);
        other
            .batch_execute("DROP TABLE changed; CREATE TABLE changed(id bigint)")
            .await
            .unwrap();
        if include_row {
            // This value fits either width: value conversion alone could
            // otherwise silently return INT4 metadata for a BIGINT column.
            other
                .batch_execute("INSERT INTO changed VALUES(9)")
                .await
                .unwrap();
        }
        resume.send(()).unwrap();
        use futures_util::TryStreamExt;
        let error = rows.try_next().await.unwrap_err();
        assert!(error.is_statement_invalidated(), "{error}");
        assert!(statement.is_invalidated());
        assert!(rows.try_next().await.unwrap().is_none());
        client.check_connection().await.unwrap();
        // Reset cannot make the now-reprepared native handle match the
        // original public metadata; a fresh public prepare is required.
        assert!(
            client
                .query(&statement, &[])
                .await
                .unwrap_err()
                .is_statement_invalidated()
        );
        drop(rows);
        finish(other, other_task).await;
        drop(client);
        finish(actor, task).await;
    }
}

// [spec:pgorm:sem:turso.execution/test] preserve a native error while invalidating metadata replaced in that step
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_reprepared_write_invalidates_handle() {
    use futures_util::TryStreamExt;

    let database = super::super::Database::open(Path::new(":memory:"))
        .await
        .unwrap();
    let (actor, lifetime) = database.connect().await.unwrap();
    let task = tokio::spawn(lifetime);
    let client = crate::Client::from_turso(Arc::clone(&actor));
    client
        .batch_execute("CREATE TABLE changed(id integer PRIMARY KEY)")
        .await
        .unwrap();
    let statement = client
        .prepare("INSERT INTO changed VALUES(1) RETURNING id")
        .await
        .unwrap();
    let (gate, entered, resume) = gate();
    *actor.control.before_step.lock().unwrap() = Some(gate);
    let mut rows = Box::pin(
        client
            .query_raw(&statement, std::iter::empty::<&i32>())
            .await
            .unwrap(),
    );
    entered.await.unwrap();
    let (other, lifetime) = database.connect().await.unwrap();
    let other_task = tokio::spawn(lifetime);
    other
        .batch_execute("DROP TABLE changed; CREATE TABLE changed(id bigint PRIMARY KEY); INSERT INTO changed VALUES(1)")
        .await
        .unwrap();
    resume.send(()).unwrap();
    let error = rows.try_next().await.unwrap_err();
    assert_eq!(error.code().unwrap().code(), "23505");
    assert!(statement.is_invalidated());
    client.check_connection().await.unwrap();
    drop(rows);
    finish(other, other_task).await;
    drop(client);
    finish(actor, task).await;
}
