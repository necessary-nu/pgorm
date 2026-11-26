use super::{
    tests::{actor, finish, gate},
    *,
};
use futures_util::TryStreamExt;

// [spec:pgorm:sem:turso.cleanup/test] cancellation of an unacknowledged boundary disposes the connection
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abandoned_transaction_boundaries_close() {
    for sql in ["BEGIN", "SAVEPOINT nested", "COMMIT", "ROLLBACK"] {
        let (client, task) = actor().await;
        if sql != "BEGIN" {
            client.batch_execute("BEGIN").await.unwrap();
        }
        let (pause, entered, resume) = gate();
        *client.control.before_step.lock().unwrap() = Some(pause);
        let executing = Arc::clone(&client);
        let request = tokio::spawn(async move {
            match sql {
                "COMMIT" => executing
                    .commit(None)
                    .await
                    .map_err(crate::TursoCommitError::into_error),
                "ROLLBACK" => executing.rollback(None).await,
                _ => executing.batch_execute(sql).await,
            }
        });
        tokio::time::timeout(Duration::from_secs(3), entered)
            .await
            .unwrap()
            .unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        assert!(client.is_closed(), "{sql}");
        resume.send(()).unwrap();
        finish(client, task).await;
    }
}

// [spec:pgorm:sem:turso.cleanup/test] a completed COMMIT remains dirty until its consumer acknowledges it
#[tokio::test]
async fn unacknowledged_commit_keeps_committed_data() {
    let database = super::super::Database::open(std::path::Path::new(":memory:"))
        .await
        .unwrap();
    let (actor, lifetime) = database.connect().await.unwrap();
    let task = tokio::spawn(lifetime);
    let client = crate::Client::from_turso(Arc::clone(&actor));
    client
        .batch_execute(
            "CREATE TABLE writes(id integer PRIMARY KEY); BEGIN; INSERT INTO writes VALUES(1)",
        )
        .await
        .unwrap();
    let rows = client
        .query_raw("COMMIT", std::iter::empty::<&i32>())
        .await
        .unwrap();
    // FIFO health completion proves COMMIT reached Done and finished cleanup,
    // while the stream itself has never been polled to observe its outcome.
    actor.check_connection().await.unwrap();
    assert!(!actor.is_clean());
    let (peer, lifetime) = database.connect().await.unwrap();
    let peer_task = tokio::spawn(lifetime);
    let peer_client = crate::Client::from_turso(Arc::clone(&peer));
    assert_eq!(
        peer_client
            .query_one("SELECT count(*) FROM writes", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    drop(rows);
    assert!(actor.is_closed());
    drop(client);
    finish(actor, task).await;
    assert_eq!(
        peer_client
            .query_one("SELECT count(*) FROM writes", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    drop(peer_client);
    finish(peer, peer_task).await;
}

// [spec:pgorm:sem:turso.cleanup/test] acknowledging a boundary allows reuse and its later drop cannot cancel new work
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acknowledged_commit_allows_reuse() {
    let (actor, task) = actor().await;
    let client = crate::Client::from_turso(Arc::clone(&actor));
    actor.batch_execute("BEGIN").await.unwrap();
    let mut rows = Box::pin(
        client
            .query_raw("COMMIT", std::iter::empty::<&i32>())
            .await
            .unwrap(),
    );
    actor.check_connection().await.unwrap();
    assert!(!actor.is_clean());
    assert!(rows.try_next().await.unwrap().is_none());
    assert!(actor.is_clean());
    let (pause, entered, resume) = gate();
    *actor.control.before_step.lock().unwrap() = Some(pause);
    let checking = Arc::clone(&actor);
    let check = tokio::spawn(async move { checking.check_connection().await });
    tokio::time::timeout(Duration::from_secs(3), entered)
        .await
        .unwrap()
        .unwrap();
    drop(rows);
    assert!(!actor.is_closed());
    resume.send(()).unwrap();
    check.await.unwrap().unwrap();
    assert!(actor.is_clean());
    drop(client);
    finish(actor, task).await;
}

// [spec:pgorm:sem:turso.cleanup/test] drop rollback stays behind active cleanup and before later requests
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drop_rollback_preserves_fifo() {
    let (client, task) = actor().await;
    client
        .batch_execute(
            "CREATE TABLE writes(id integer PRIMARY KEY); BEGIN; INSERT INTO writes VALUES(1)",
        )
        .await
        .unwrap();
    let (pause, entered, resume) = gate();
    *client.control.before_step.lock().unwrap() = Some(pause);
    let executing = Arc::clone(&client);
    let request = tokio::spawn(async move { executing.check_connection().await });
    tokio::time::timeout(Duration::from_secs(3), entered)
        .await
        .unwrap()
        .unwrap();
    client.rollback_on_drop(None);
    assert!(!client.is_clean());
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert!(!client.is_closed());
    resume.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), client.check_connection())
        .await
        .unwrap()
        .unwrap();
    assert!(client.is_clean());
    client
        .batch_execute("INSERT INTO writes VALUES(1)")
        .await
        .unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.cleanup/test] a saturated FIFO cannot lose or prioritize a transaction's rollback
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saturated_drop_rollback_closes() {
    let (client, task) = actor().await;
    client.batch_execute("BEGIN").await.unwrap();
    let (pause, entered, resume) = gate();
    *client.control.before_step.lock().unwrap() = Some(pause);
    let executing = Arc::clone(&client);
    let request = tokio::spawn(async move { executing.check_connection().await });
    tokio::time::timeout(Duration::from_secs(3), entered)
        .await
        .unwrap()
        .unwrap();
    let mut queued = Vec::new();
    for _ in 0..COMMAND_CAPACITY {
        let operation = client.control.new_operation();
        operation.acknowledge();
        let (reply, response) = oneshot::channel();
        assert!(
            client
                .sender
                .try_send(Command::Batch {
                    sql: "SELECT 1".to_owned(),
                    operation,
                    reply
                })
                .is_ok()
        );
        queued.push(response);
    }
    client.rollback_on_drop(None);
    assert!(client.is_closed());
    resume.send(()).unwrap();
    assert!(request.await.unwrap().is_err());
    for response in queued {
        assert!(response.await.unwrap().unwrap_err().is_closed());
    }
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.transactions/test] losing an outer transaction cannot turn failed RELEASE into safe autocommit reuse
#[tokio::test]
async fn lost_savepoint_end_quarantines() {
    let (actor, task) = actor().await;
    let mut client = crate::Client::from_turso(Arc::clone(&actor));
    let mut outer = client.transaction().await.unwrap();
    let inner = outer.transaction().await.unwrap();
    inner.batch_execute("ROLLBACK").await.unwrap();
    let error = inner.commit().await.unwrap_err();
    assert!(error.is_turso_cleanup_failure());
    assert!(actor.is_closed());
    assert!(
        outer
            .batch_execute("SELECT 1")
            .await
            .unwrap_err()
            .is_closed()
    );
    drop(outer);
    drop(client);
    finish(actor, task).await;
}

// [spec:pgorm:sem:turso.transactions/test] a cached VM refused before Init remains recoverable by rollback
#[tokio::test]
async fn cached_refusal_preserves_rollback() {
    let (actor, task) = actor().await;
    let client = crate::Client::from_turso(Arc::clone(&actor));
    client
        .batch_execute("CREATE TABLE writes(id integer PRIMARY KEY); INSERT INTO writes VALUES(1)")
        .await
        .unwrap();
    let statement = client.prepare("SELECT 1").await.unwrap();
    client.batch_execute("BEGIN").await.unwrap();
    assert_eq!(
        client
            .batch_execute("INSERT INTO writes VALUES(1)")
            .await
            .unwrap_err()
            .code()
            .unwrap()
            .code(),
        "23505"
    );
    assert_eq!(
        client
            .query(&statement, &[])
            .await
            .unwrap_err()
            .code()
            .unwrap()
            .code(),
        "25P02"
    );
    assert!(!actor.is_closed());
    actor.rollback(None).await.unwrap();
    assert!(actor.is_clean());
    client.query("SELECT 1", &[]).await.unwrap();
    drop(statement);
    drop(client);
    finish(actor, task).await;
}

// [spec:pgorm:sem:turso.cleanup/test] cleanup-only failure is marked unsafe for retries and recycling
#[tokio::test]
async fn rollback_cleanup_failure_quarantines() {
    let (client, task) = actor().await;
    client.batch_execute("BEGIN").await.unwrap();
    client
        .control
        .fail_next_cleanup
        .store(true, Ordering::Release);
    let error = client.rollback(None).await.unwrap_err();
    assert!(error.is_turso_cleanup_failure());
    assert!(client.is_closed());
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.transactions/test] aborted commit retains the refusal and a failed rollback's cause
#[tokio::test]
async fn aborted_commit_keeps_rollback_failure() {
    let (client, task) = actor().await;
    client
        .batch_execute(
            "CREATE TABLE writes(id integer PRIMARY KEY); INSERT INTO writes VALUES(1); BEGIN",
        )
        .await
        .unwrap();
    client
        .batch_execute("INSERT INTO writes VALUES(1)")
        .await
        .unwrap_err();
    client
        .control
        .fail_next_cleanup
        .store(true, Ordering::Release);
    match client.commit(None).await.unwrap_err() {
        crate::TursoCommitError::Aborted {
            refusal,
            rollback: Some(error),
        } => {
            assert_eq!(refusal.code().unwrap().code(), "25P02");
            assert!(error.is_turso_cleanup_failure());
        }
        error => panic!("unexpected commit error: {error}"),
    }
    assert!(client.is_closed());
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.cleanup/test] health completion publishes reusable state before waking its caller
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completion_publishes_clean_state() {
    let (client, task) = actor().await;
    assert!(client.is_clean());
    for _ in 0..32 {
        client.check_connection().await.unwrap();
        assert!(client.is_clean());
        client.batch_execute("BEGIN").await.unwrap();
        assert!(!client.is_clean());
        client.commit(None).await.unwrap();
        assert!(client.is_clean());
    }
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.cleanup/test] awaited shutdown releases an open transaction without committing its writes
#[tokio::test]
async fn close_rolls_back_pending_writes() {
    let database = super::super::Database::open(std::path::Path::new(":memory:"))
        .await
        .unwrap();
    let (client, lifetime) = database.connect().await.unwrap();
    let task = tokio::spawn(lifetime);
    client
        .batch_execute(
            "CREATE TABLE writes(id integer PRIMARY KEY); BEGIN; INSERT INTO writes VALUES(1)",
        )
        .await
        .unwrap();
    client.shutdown();
    finish(client, task).await;
    let (peer, lifetime) = database.connect().await.unwrap();
    let task = tokio::spawn(lifetime);
    // A leaked write would fail this insert; a leaked lock would block it.
    tokio::time::timeout(
        Duration::from_secs(3),
        peer.batch_execute("INSERT INTO writes VALUES(1)"),
    )
    .await
    .unwrap()
    .unwrap();
    finish(peer, task).await;
}
