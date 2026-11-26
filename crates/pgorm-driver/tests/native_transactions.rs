//! Native transaction outcomes, modes and ordered rollback-on-drop.
#![cfg(feature = "turso")]

use pgorm_driver::{
    Client, Error, IsolationLevel, TursoCommitError, TursoDatabase, error::SqlState, types::Type,
};
use std::time::Duration;
use tokio::{task::JoinHandle, time::timeout};

type Task = JoinHandle<Result<(), Error>>;
const LIMIT: Duration = Duration::from_secs(5);

async fn connect(database: &TursoDatabase) -> (Client, Task) {
    let (client, connection) = database.connect().await.unwrap();
    (client, tokio::spawn(connection))
}

async fn session() -> (Client, Task) {
    connect(&TursoDatabase::open(":memory:").await.unwrap()).await
}

async fn finish(client: Client, task: Task) {
    drop(client);
    timeout(LIMIT, task).await.unwrap().unwrap().unwrap();
}

async fn ids(client: &Client) -> Vec<i32> {
    client
        .query("SELECT id FROM tx_values ORDER BY id", &[])
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

// [spec:pgorm:sem:turso.transactions/test]
#[tokio::test]
async fn commit_and_rollback_report_completed_outcomes() {
    let (mut client, task) = session().await;
    client
        .batch_execute("CREATE TABLE tx_values(id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    let tx = client.transaction().await.unwrap();
    assert_eq!(
        tx.execute("INSERT INTO tx_values VALUES(1)", &[])
            .await
            .unwrap(),
        1
    );
    tx.commit().await.unwrap();
    assert_eq!(ids(&client).await, [1]);
    let tx = client.transaction().await.unwrap();
    tx.execute("INSERT INTO tx_values VALUES(2)", &[])
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(ids(&client).await, [1]);
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.transactions/test]
#[tokio::test]
async fn aborted_commits_preserve_the_native_refusal() {
    let (mut client, task) = session().await;
    client
        .batch_execute("CREATE TABLE tx_values(id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    for checked in [false, true] {
        let tx = client.transaction().await.unwrap();
        tx.execute("INSERT INTO tx_values VALUES(1)", &[])
            .await
            .unwrap();
        let duplicate = tx
            .execute("INSERT INTO tx_values VALUES(1)", &[])
            .await
            .unwrap_err();
        assert_eq!(duplicate.code(), Some(&SqlState::UNIQUE_VIOLATION));
        let refusal = if checked {
            match tx.commit_checked_turso().await {
                Err(TursoCommitError::Aborted {
                    refusal,
                    rollback: None,
                }) => refusal,
                other => panic!("expected a clean aborted outcome: {other:?}"),
            }
        } else {
            tx.commit().await.unwrap_err()
        };
        assert_eq!(refusal.code(), Some(&SqlState::IN_FAILED_SQL_TRANSACTION));
        assert!(ids(&client).await.is_empty());
    }
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.transactions/test]
#[tokio::test]
async fn nested_savepoints_commit_and_rollback_independently() {
    let (mut client, task) = session().await;
    client
        .batch_execute("CREATE TABLE tx_values(id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    let mut outer = client.transaction().await.unwrap();
    outer
        .execute("INSERT INTO tx_values VALUES(1)", &[])
        .await
        .unwrap();
    let inner = outer.transaction().await.unwrap();
    inner
        .execute("INSERT INTO tx_values VALUES(2)", &[])
        .await
        .unwrap();
    inner.commit().await.unwrap();
    let mut inner = outer.transaction().await.unwrap();
    inner
        .execute("INSERT INTO tx_values VALUES(3)", &[])
        .await
        .unwrap();
    let nested = inner.transaction().await.unwrap();
    nested
        .execute("INSERT INTO tx_values VALUES(4)", &[])
        .await
        .unwrap();
    nested.commit().await.unwrap();
    inner.rollback().await.unwrap();
    outer.commit().await.unwrap();
    assert_eq!(ids(&client).await, [1, 2]);
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.transactions/test]
#[tokio::test]
async fn savepoint_rollback_invalidates_before_cached_handle_reuse() {
    let (mut client, task) = session().await;
    client
        .batch_execute("CREATE TABLE tx_values(id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    let insert_sql = "INSERT INTO tx_values VALUES($1)";
    let insert = client
        .prepare_typed(insert_sql, &[Type::INT4])
        .await
        .unwrap();
    let select = client
        .prepare("SELECT id FROM tx_values ORDER BY id")
        .await
        .unwrap();
    let mut outer = client.transaction().await.unwrap();
    outer.execute(&insert, &[&1i32]).await.unwrap();
    let inner = outer.transaction().await.unwrap();
    inner.execute(&insert, &[&2i32]).await.unwrap();
    assert_eq!(inner.query(&select, &[]).await.unwrap().len(), 2);
    assert!(!insert.is_invalidated());
    assert!(!select.is_invalidated());
    inner.rollback().await.unwrap();
    // The pinned engine changes its prepare context on ROLLBACK TO even when
    // no DDL ran. The pool must see this before looking up a cached handle.
    assert!(insert.is_invalidated());
    assert!(select.is_invalidated());
    let refused = outer.execute(&insert, &[&3i32]).await.unwrap_err();
    assert!(refused.is_statement_invalidated());
    let fresh = outer
        .prepare_typed(insert_sql, &[Type::INT4])
        .await
        .unwrap();
    let inner = outer.transaction().await.unwrap();
    inner.execute(&fresh, &[&3i32]).await.unwrap();
    inner.commit().await.unwrap();
    assert!(
        !fresh.is_invalidated(),
        "RELEASE alone keeps the prepared context"
    );
    outer.commit().await.unwrap();
    assert!(
        !fresh.is_invalidated(),
        "COMMIT without context changes keeps the handle"
    );
    client.execute(&fresh, &[&4i32]).await.unwrap();
    assert_eq!(ids(&client).await, [1, 3, 4]);
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.cleanup/test]
#[tokio::test]
async fn dropped_savepoint_rolls_back_before_outer_work() {
    let (mut client, task) = session().await;
    client
        .batch_execute("CREATE TABLE tx_values(id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    let mut outer = client.transaction().await.unwrap();
    outer
        .execute("INSERT INTO tx_values VALUES(1)", &[])
        .await
        .unwrap();
    let inner = outer.transaction().await.unwrap();
    inner
        .execute("INSERT INTO tx_values VALUES(2)", &[])
        .await
        .unwrap();
    drop(inner);
    timeout(LIMIT, outer.execute("INSERT INTO tx_values VALUES(3)", &[]))
        .await
        .unwrap()
        .unwrap();
    outer.commit().await.unwrap();
    assert_eq!(ids(&client).await, [1, 3]);
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.cleanup/test]
#[tokio::test]
async fn dropped_transaction_rolls_back_before_next_transaction() {
    let (mut client, task) = session().await;
    client
        .batch_execute("CREATE TABLE tx_values(id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute("INSERT INTO tx_values VALUES(1)", &[])
        .await
        .unwrap();
    drop(tx);
    let next = timeout(LIMIT, client.transaction()).await.unwrap().unwrap();
    next.execute("INSERT INTO tx_values VALUES(2)", &[])
        .await
        .unwrap();
    next.commit().await.unwrap();
    assert_eq!(ids(&client).await, [2]);
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.transactions/test]
#[tokio::test]
async fn repeatable_read_keeps_its_snapshot_until_commit() {
    let database = TursoDatabase::open(":memory:").await.unwrap();
    let (mut first, first_task) = connect(&database).await;
    first
        .batch_execute(
            "CREATE TABLE tx_values(id INTEGER PRIMARY KEY); INSERT INTO tx_values VALUES(1)",
        )
        .await
        .unwrap();
    let (second, second_task) = connect(&database).await;
    let tx = first
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .start()
        .await
        .unwrap();
    assert_eq!(
        tx.query_one("SELECT count(*) FROM tx_values", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    second
        .execute("INSERT INTO tx_values VALUES(2)", &[])
        .await
        .unwrap();
    assert_eq!(
        tx.query_one("SELECT count(*) FROM tx_values", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    tx.commit().await.unwrap();
    assert_eq!(ids(&first).await, [1, 2]);
    finish(first, first_task).await;
    finish(second, second_task).await;
}

// [spec:pgorm:sem:turso.transactions/test]
#[tokio::test]
async fn unsupported_builder_modes_are_refused_before_begin() {
    let (mut client, task) = session().await;
    for mode in 0..7 {
        {
            let builder = client.build_transaction();
            let result = match mode {
                0 => {
                    builder
                        .isolation_level(IsolationLevel::ReadUncommitted)
                        .start()
                        .await
                }
                1 => {
                    builder
                        .isolation_level(IsolationLevel::ReadCommitted)
                        .start()
                        .await
                }
                2 => {
                    builder
                        .isolation_level(IsolationLevel::Serializable)
                        .start()
                        .await
                }
                3 => builder.read_only(true).start().await,
                4 => builder.read_only(false).start().await,
                5 => builder.deferrable(true).start().await,
                6 => builder.deferrable(false).start().await,
                _ => unreachable!(),
            };
            let error = result.err().expect("unsupported request must be refused");
            assert!(
                error.code().is_none(),
                "driver refusal must not invent SQLSTATE"
            );
        }
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            tx.query_one("SELECT 1", &[])
                .await
                .unwrap()
                .get::<_, i32>(0),
            1
        );
        tx.rollback().await.unwrap();
    }
    finish(client, task).await;
}
