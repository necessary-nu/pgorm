//! Native lifecycle tests use only isolated memory or temporary files.
#![cfg(feature = "turso")]

use pgorm_driver::{Client, Error, NoTls, TursoDatabase};
use std::{sync::Arc, time::Duration};
use tokio::task::JoinHandle;

const LONG_QUERY: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 100000000) SELECT sum(x) FROM n";

async fn connect(database: &TursoDatabase) -> (Arc<Client>, JoinHandle<Result<(), Error>>) {
    let (client, connection) = database.connect().await.unwrap();
    (Arc::new(client), tokio::spawn(connection))
}

async fn finish(client: Arc<Client>, task: JoinHandle<Result<(), Error>>) {
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

// [spec:pgorm:sem:turso.lifecycle/test] one handle shares memory, separate opens isolate it
#[tokio::test]
async fn memory_ownership_and_distinct_connections() {
    let database = TursoDatabase::open(":memory:").await.unwrap();
    let (first, first_task) = connect(&database).await;
    let (second, second_task) = connect(&database.clone()).await;
    first
        .batch_execute("CREATE TABLE shared (id integer PRIMARY KEY)")
        .await
        .unwrap();
    second
        .batch_execute("INSERT INTO shared VALUES (1)")
        .await
        .unwrap();
    let error = first
        .batch_execute("INSERT INTO shared VALUES (1)")
        .await
        .unwrap_err();
    assert_eq!(error.code().unwrap().code(), "23505");
    first.check_connection().await.unwrap();
    second.check_connection().await.unwrap();

    let independent = TursoDatabase::open(":memory:").await.unwrap();
    let (other, other_task) = connect(&independent).await;
    other
        .batch_execute("CREATE TABLE shared (id integer PRIMARY KEY)")
        .await
        .unwrap();
    other
        .batch_execute("INSERT INTO shared VALUES (1)")
        .await
        .unwrap();
    finish(other, other_task).await;
    finish(first, first_task).await;
    finish(second, second_task).await;
}

// [spec:pgorm:sem:turso.lifecycle/test] committed file data survives complete handle teardown
#[tokio::test]
async fn file_persistence_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("database.db");
    {
        let database = TursoDatabase::open(&path).await.unwrap();
        let (client, task) = connect(&database).await;
        client
            .batch_execute(
                "CREATE TABLE persisted (id integer PRIMARY KEY); INSERT INTO persisted VALUES (7)",
            )
            .await
            .unwrap();
        finish(client, task).await;
    }
    let database = TursoDatabase::open(&path).await.unwrap();
    let (client, task) = connect(&database).await;
    let error = client
        .batch_execute("INSERT INTO persisted VALUES (7)")
        .await
        .unwrap_err();
    assert_eq!(error.code().unwrap().code(), "23505");
    client.check_connection().await.unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.lifecycle/test] failed open releases ownership and does not return a client
#[tokio::test]
async fn failed_open_then_successful_open() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing/database.db");
    assert!(TursoDatabase::open(&path).await.is_err());
    std::fs::create_dir(path.parent().unwrap()).unwrap();
    let database = TursoDatabase::open(&path).await.unwrap();
    let (client, task) = connect(&database).await;
    client.check_connection().await.unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.lifecycle/test] dropping even an unpolled lifetime closes the client
#[tokio::test]
async fn dropped_connection_future_closes_client() {
    let database = TursoDatabase::open(":memory:").await.unwrap();
    let (client, connection) = database.connect().await.unwrap();
    drop(connection);
    assert!(client.is_closed());
    assert!(client.check_connection().await.unwrap_err().is_closed());
    assert!(
        client
            .batch_execute("CREATE TABLE must_not_run(id integer)")
            .await
            .unwrap_err()
            .is_closed()
    );
}

// [spec:pgorm:sem:turso.lifecycle/test] cancelling an operation finishes cleanup before reuse
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_request_cancels_cpu_work_before_reuse() {
    let database = TursoDatabase::open(":memory:").await.unwrap();
    let (client, task) = connect(&database).await;
    let operation_client = Arc::clone(&client);
    let operation = tokio::spawn(async move { operation_client.batch_execute(LONG_QUERY).await });
    tokio::time::sleep(Duration::from_millis(25)).await;
    operation.abort();
    assert!(operation.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(5), client.check_connection())
        .await
        .unwrap()
        .unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.lifecycle/test] public cancellation reaches native CPU execution
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_and_subsequent_connection_health() {
    let database = TursoDatabase::open(":memory:").await.unwrap();
    let (client, task) = connect(&database).await;
    let operation_client = Arc::clone(&client);
    let operation = tokio::spawn(async move { operation_client.batch_execute(LONG_QUERY).await });
    tokio::time::sleep(Duration::from_millis(25)).await;
    client.cancel_token().cancel_query(NoTls).await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(5), operation)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code().unwrap().code(), "57014");
    client.check_connection().await.unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.lifecycle/test] aborted execution retains the session's transaction state
#[tokio::test]
async fn execution_error_requires_explicit_transaction_rollback() {
    let database = TursoDatabase::open(":memory:").await.unwrap();
    let (client, task) = connect(&database).await;
    client
        .batch_execute("CREATE TABLE tx (id integer PRIMARY KEY); INSERT INTO tx VALUES (1); BEGIN")
        .await
        .unwrap();
    assert_eq!(
        client
            .batch_execute("INSERT INTO tx VALUES (1)")
            .await
            .unwrap_err()
            .code()
            .unwrap()
            .code(),
        "23505"
    );
    assert_eq!(
        client
            .check_connection()
            .await
            .unwrap_err()
            .code()
            .unwrap()
            .code(),
        "25P02"
    );
    client.batch_execute("ROLLBACK").await.unwrap();
    client.check_connection().await.unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.lifecycle/test] runtime teardown itself drops the lifetime guard
#[test]
fn runtime_shutdown_cancels_worker_without_manual_abort() {
    let (done, completed) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = runtime.block_on(async {
            let database = TursoDatabase::open(":memory:").await.unwrap();
            let (client, task) = connect(&database).await;
            // Retaining a JoinHandle does not prevent runtime teardown from
            // dropping the connection future; no abort is called here.
            let operation_client = Arc::clone(&client);
            tokio::spawn(async move { operation_client.batch_execute(LONG_QUERY).await });
            tokio::time::sleep(Duration::from_millis(25)).await;
            drop(task);
            client
        });
        drop(runtime);
        assert!(client.is_closed());
        done.send(()).unwrap();
    });
    completed
        .recv_timeout(Duration::from_secs(5))
        .expect("native worker prevented Tokio runtime shutdown");
}

// [spec:pgorm:sem:turso.lifecycle/test] runtime-owned supervisor stops even a retained unpolled future
#[test]
fn runtime_shutdown_with_retained_unpolled_connection() {
    let (done, completed) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (client, connection) = runtime.block_on(async {
            let database = TursoDatabase::open(":memory:").await.unwrap();
            database.connect().await.unwrap()
        });
        // Neither value has been dropped, and the connection future has never
        // been polled or spawned. The runtime-owned supervisor must stop it.
        drop(runtime);
        assert!(client.is_closed());
        drop(connection);
        drop(client);
        done.send(()).unwrap();
    });
    completed
        .recv_timeout(Duration::from_secs(5))
        .expect("retained native Connection prevented runtime shutdown");
}

// [spec:pgorm:req:read-only.turso/test]    a file opens read-only and refuses writes with 25006; the registry's other access is refused both ways; `:memory:` is refused
#[tokio::test]
async fn read_only_opens_refuse_writes_and_mixed_access() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("database.db");
    {
        let database = TursoDatabase::open(&path).await.unwrap();
        let (client, task) = connect(&database).await;
        client
            .batch_execute(
                "CREATE TABLE kept (id integer PRIMARY KEY); INSERT INTO kept VALUES (1)",
            )
            .await
            .unwrap();
        let shared = TursoDatabase::open_read_only(&path).await.unwrap_err();
        assert_eq!(
            shared.to_string(),
            "native Turso does not support opening a file read-only while this process has it open read-write"
        );
        finish(client, task).await;
    }

    let database = TursoDatabase::open_read_only(&path).await.unwrap();
    let (client, task) = connect(&database).await;
    assert_eq!(
        client
            .query("SELECT id FROM kept", &[])
            .await
            .unwrap()
            .len(),
        1
    );
    let write = client
        .batch_execute("INSERT INTO kept VALUES (2)")
        .await
        .unwrap_err();
    assert_eq!(write.code().unwrap().code(), "25006");
    let writable = TursoDatabase::open(&path).await.unwrap_err();
    assert_eq!(
        writable.to_string(),
        "native Turso does not support opening a file read-write while this process has it open read-only"
    );
    finish(client, task).await;

    let memory = TursoDatabase::open_read_only(":memory:").await.unwrap_err();
    assert_eq!(
        memory.to_string(),
        "native Turso does not support a read-only `:memory:` database"
    );
}
