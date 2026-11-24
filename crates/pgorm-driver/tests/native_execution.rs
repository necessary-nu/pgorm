//! Public execution, prepared-handle and bounded-stream contracts.
#![cfg(feature = "turso")]

use bytes::BytesMut;
use futures_util::TryStreamExt;
use pgorm_driver::{
    Client, Error, NoTls, TursoDatabase,
    types::{IsNull, ToSql, Type},
};
use std::{
    error::Error as StdError,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::oneshot, task::JoinHandle, time::timeout};

type Task = JoinHandle<Result<(), Error>>;
const LIMIT: Duration = Duration::from_secs(5);
const MANY: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT x::bigint FROM n";
const SLOW: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT count(*) FROM n";

async fn connect(database: &TursoDatabase) -> (Arc<Client>, Task) {
    let (client, connection) = database.connect().await.unwrap();
    (Arc::new(client), tokio::spawn(connection))
}

async fn session() -> (Arc<Client>, Task) {
    connect(&TursoDatabase::open(":memory:").await.unwrap()).await
}

async fn finish(client: Arc<Client>, task: Task) {
    drop(client);
    timeout(LIMIT, task).await.unwrap().unwrap().unwrap();
}

fn refusal<T>(result: Result<T, Error>) -> Error {
    let error = result.err().expect("operation must be refused");
    assert!(
        error.code().is_none(),
        "driver refusal must not invent SQLSTATE"
    );
    error
}

#[derive(Debug)]
struct Counted(Arc<AtomicUsize>);

impl ToSql for Counted {
    fn to_sql(
        &self,
        ty: &Type,
        output: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn StdError + Send + Sync>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        7_i32.to_sql(ty, output)
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::INT4
    }

    pgorm_driver::types::to_sql_checked!();
}

// [spec:pgorm:req:turso.parameters/test]
// [spec:pgorm:sem:turso.execution/test]
#[tokio::test]
async fn explicit_types_precede_codec_calls() {
    let (client, task) = session().await;
    client
        .batch_execute("CREATE TABLE typed(id integer)")
        .await
        .unwrap();
    let sql = "INSERT INTO typed VALUES($1) RETURNING id";
    let calls = Arc::new(AtomicUsize::new(0));
    let value = Counted(Arc::clone(&calls));
    let error = refusal(client.prepare(sql).await);
    assert!(error.to_string().contains("prepare_typed"), "{error}");
    refusal(client.query(sql, &[&value]).await);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let count = client
        .query_one("SELECT count(*) FROM typed", &[])
        .await
        .unwrap();
    assert_eq!(count.get::<_, i64>(0), 0);
    let statement = client.prepare_typed(sql, &[Type::INT4]).await.unwrap();
    assert_eq!(statement.params(), &[Type::INT4]);
    for expected in 1..=3 {
        let row = client.query_one(&statement, &[&value]).await.unwrap();
        assert_eq!(row.get::<_, i32>(0), 7);
        assert_eq!(calls.load(Ordering::SeqCst), expected);
    }
    finish(client, task).await;
}

// [spec:pgorm:req:turso.parameters/test]
// [spec:pgorm:sem:turso.execution/test]
#[tokio::test]
async fn typed_and_owned_raw_query_routes() {
    let (client, task) = session().await;
    let sql = "SELECT $1::integer AS value";
    let params: &[(&(dyn ToSql + Sync), Type)] = &[(&8_i32, Type::INT4)];
    assert_eq!(
        client.query_typed(sql, params).await.unwrap()[0].get::<_, i32>(0),
        8
    );
    assert_eq!(
        client
            .query_typed_one(sql, params)
            .await
            .unwrap()
            .get::<_, i32>(0),
        8
    );
    assert_eq!(
        client
            .query_typed_opt(sql, params)
            .await
            .unwrap()
            .unwrap()
            .get::<_, i32>(0),
        8
    );
    assert!(
        client
            .query_typed_opt("SELECT $1::integer WHERE false", params)
            .await
            .unwrap()
            .is_none()
    );
    let mut stream = Box::pin(
        client
            .query_typed_raw("SELECT $1::text", [("owned".to_owned(), Type::TEXT)])
            .await
            .unwrap(),
    );
    assert_eq!(
        stream.try_next().await.unwrap().unwrap().get::<_, &str>(0),
        "owned"
    );
    assert!(stream.try_next().await.unwrap().is_none());
    assert_eq!(stream.rows_affected(), Some(1));
    let statement = client.prepare_typed(sql, &[Type::INT4]).await.unwrap();
    let mut stream = Box::pin(client.query_raw(&statement, [9_i32]).await.unwrap());
    assert_eq!(
        stream.try_next().await.unwrap().unwrap().get::<_, i32>(0),
        9
    );
    assert!(stream.try_next().await.unwrap().is_none());
    assert_eq!(client.execute_raw(&statement, [10_i32]).await.unwrap(), 1);
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.execution/test]
// [spec:pgorm:sem:turso.streaming/test]
#[tokio::test]
async fn cardinality_errors_release_the_stream() {
    let (client, task) = session().await;
    assert!(
        client
            .query_opt("SELECT 1 WHERE false", &[])
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        client
            .query_opt("SELECT 1", &[])
            .await
            .unwrap()
            .unwrap()
            .get::<_, i32>(0),
        1
    );
    refusal(client.query_one("SELECT 1 WHERE false", &[]).await);
    refusal(client.query_one(MANY, &[]).await);
    refusal(client.query_opt(MANY, &[]).await);
    refusal(client.query_typed_one("SELECT 1 WHERE false", &[]).await);
    refusal(client.query_typed_opt(MANY, &[]).await);
    timeout(LIMIT, client.check_connection())
        .await
        .unwrap()
        .unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.execution/test]
#[tokio::test]
async fn command_counts_match_completed_execution() {
    let (client, task) = session().await;
    assert_eq!(
        client
            .execute("CREATE TABLE counts(id integer)", &[])
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        client
            .execute("INSERT INTO counts VALUES(1),(2),(3)", &[])
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        client.execute("SELECT id FROM counts", &[]).await.unwrap(),
        3
    );
    assert_eq!(
        client
            .execute("SELECT id FROM counts WHERE false", &[])
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        client
            .execute_typed(
                "UPDATE counts SET id = id + $1 WHERE id <= 2 RETURNING id",
                &[(&10_i32, Type::INT4)]
            )
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        client
            .execute("DELETE FROM counts WHERE id = 3 RETURNING id", &[])
            .await
            .unwrap(),
        1
    );
    let mut rows = Box::pin(
        client
            .query_raw(
                "INSERT INTO counts VALUES(20),(21) RETURNING id",
                std::iter::empty::<i32>(),
            )
            .await
            .unwrap(),
    );
    assert_eq!(rows.rows_affected(), None);
    assert_eq!(rows.try_next().await.unwrap().unwrap().get::<_, i32>(0), 20);
    assert_eq!(rows.rows_affected(), None);
    assert_eq!(rows.try_next().await.unwrap().unwrap().get::<_, i32>(0), 21);
    assert!(rows.try_next().await.unwrap().is_none());
    assert_eq!(rows.rows_affected(), Some(2));
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.execution/test]
#[tokio::test]
async fn scripts_stop_after_partial_effects() {
    let (client, task) = session().await;
    client
        .batch_execute("CREATE TABLE scripts(id integer PRIMARY KEY, note text)")
        .await
        .unwrap();
    let multiple = "INSERT INTO scripts VALUES(1, 'not executed'); SELECT 1";
    refusal(client.prepare(multiple).await);
    refusal(client.query(multiple, &[]).await);
    refusal(client.execute(multiple, &[]).await);
    refusal(client.query_typed(multiple, &[]).await);
    refusal(client.execute_typed(multiple, &[]).await);
    let error = client.batch_execute("INSERT INTO scripts VALUES(1, 'one;two'); /* ; */ SELECT 1; INSERT INTO scripts VALUES(1, 'duplicate'); INSERT INTO scripts VALUES(2, 'too late')").await.unwrap_err();
    assert_eq!(error.code().unwrap().code(), "23505");
    let rows = client
        .query("SELECT id, note FROM scripts", &[])
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, i32>(0), 1);
    assert_eq!(rows[0].get::<_, &str>(1), "one;two");
    client.batch_execute("").await.unwrap();
    // The pinned frontend deliberately reports a nonblank parser-empty script
    // as native syntax failure; the driver does not invent a comment parser.
    let error = client
        .batch_execute("-- only a comment\n; /* and another ; */")
        .await
        .unwrap_err();
    assert_eq!(error.code().unwrap().code(), "42601");
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.execution/test]
#[tokio::test]
async fn cardinality_failure_does_not_imply_rollback() {
    let (client, task) = session().await;
    client
        .batch_execute("CREATE TABLE effects(id integer)")
        .await
        .unwrap();
    refusal(
        client
            .query_one("INSERT INTO effects VALUES(1),(2) RETURNING id", &[])
            .await,
    );
    let count = client
        .query_one("SELECT count(*) FROM effects", &[])
        .await
        .unwrap();
    assert_eq!(count.get::<_, i64>(0), 2);
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.execution/test]
#[tokio::test]
async fn foreign_and_closed_handles_skip_codecs() {
    let database = TursoDatabase::open(":memory:").await.unwrap();
    let (creator, creator_task) = connect(&database).await;
    let (other, other_task) = connect(&database).await;
    let statement = creator
        .prepare_typed("SELECT $1::integer", &[Type::INT4])
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let value = Counted(Arc::clone(&calls));
    refusal(other.query(&statement, &[&value]).await);
    refusal(other.execute(&statement, &[&value]).await);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    finish(creator, creator_task).await;
    assert!(
        other
            .query(&statement, &[&value])
            .await
            .unwrap_err()
            .is_closed()
    );
    assert!(
        other
            .execute(&statement, &[&value])
            .await
            .unwrap_err()
            .is_closed()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        other
            .query_one("SELECT 2", &[])
            .await
            .unwrap()
            .get::<_, i32>(0),
        2
    );
    finish(other, other_task).await;
}

// [spec:pgorm:sem:turso.execution/test]
#[tokio::test]
async fn known_invalidation_skips_later_codecs() {
    let (client, task) = session().await;
    client
        .batch_execute("CREATE TABLE invalidator(id integer)")
        .await
        .unwrap();
    let statement = client
        .prepare_typed("SELECT id FROM invalidator WHERE id = $1", &[Type::INT4])
        .await
        .unwrap();
    let alias = statement.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let value = Counted(Arc::clone(&calls));
    client
        .batch_execute("DROP TABLE invalidator; CREATE TABLE invalidator(id bigint)")
        .await
        .unwrap();
    assert!(
        client
            .query(&statement, &[&value])
            .await
            .unwrap_err()
            .is_statement_invalidated()
    );
    assert!(statement.is_invalidated());
    assert!(alias.is_invalidated());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        client
            .execute(&alias, &[&value])
            .await
            .unwrap_err()
            .is_statement_invalidated()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.streaming/test]
#[tokio::test]
async fn conversion_error_follows_emitted_rows() {
    let (client, task) = session().await;
    let sql = "WITH RECURSIVE n(x) AS (SELECT 2147483616 UNION ALL SELECT x+1 FROM n WHERE x<2147483650) SELECT x::integer FROM n";
    let statement = client.prepare(sql).await.unwrap();
    let mut rows = Box::pin(
        client
            .query_raw(&statement, std::iter::empty::<i32>())
            .await
            .unwrap(),
    );
    let mut count = 0;
    let error = loop {
        match rows.try_next().await {
            Ok(Some(row)) => {
                assert_eq!(row.get::<_, i32>(0), 2147483616 + count);
                count += 1;
            }
            Err(error) => break error,
            Ok(None) => panic!("partial result became successful EOF"),
        }
    };
    assert_eq!(count, 32);
    assert!(
        error.code().is_none(),
        "this is a driver width refusal: {error}"
    );
    assert_eq!(rows.rows_affected(), None);
    assert!(rows.try_next().await.unwrap().is_none());
    // execute drains and counts engine rows without encoding discarded cells.
    assert_eq!(client.execute(&statement, &[]).await.unwrap(), 35);
    client.check_connection().await.unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.streaming/test]
#[tokio::test]
async fn empty_and_dropped_streams_allow_reuse() {
    let (client, task) = session().await;
    let mut empty = Box::pin(
        client
            .query_raw("SELECT 1 WHERE false", std::iter::empty::<i32>())
            .await
            .unwrap(),
    );
    assert_eq!(empty.rows_affected(), None);
    assert!(empty.try_next().await.unwrap().is_none());
    assert_eq!(empty.rows_affected(), Some(0));
    let mut rows = Box::pin(
        client
            .query_raw(MANY, std::iter::empty::<i32>())
            .await
            .unwrap(),
    );
    let retained = rows.try_next().await.unwrap().unwrap();
    assert_eq!(retained.get::<_, i64>(0), 1);
    drop(rows);
    let next = timeout(LIMIT, client.query_one("SELECT 42", &[]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.get::<_, i32>(0), 42);
    assert_eq!(retained.get::<_, i64>(0), 1);
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.streaming/test]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborted_query_future_cleans_up_before_reuse() {
    let (client, task) = session().await;
    let (ready, accepted) = oneshot::channel();
    let pending_client = Arc::clone(&client);
    let pending = tokio::spawn(async move {
        let rows = pending_client
            .query_raw(SLOW, std::iter::empty::<i32>())
            .await
            .unwrap();
        ready.send(()).unwrap();
        rows.try_collect::<Vec<_>>().await
    });
    accepted.await.unwrap();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    timeout(LIMIT, client.check_connection())
        .await
        .unwrap()
        .unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.streaming/test]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_requests_resume_after_stream_drop() {
    let (client, task) = session().await;
    client
        .batch_execute("CREATE TABLE queued(id integer)")
        .await
        .unwrap();
    let rows = client
        .query_raw(MANY, std::iter::empty::<i32>())
        .await
        .unwrap();
    let mut queued = Vec::new();
    for _ in 0..64 {
        let client = Arc::clone(&client);
        queued.push(tokio::spawn(async move {
            client.execute("INSERT INTO queued VALUES(1)", &[]).await
        }));
    }
    tokio::task::yield_now().await;
    drop(rows);
    timeout(LIMIT, async {
        for request in queued {
            assert_eq!(request.await.unwrap().unwrap(), 1);
        }
    })
    .await
    .unwrap();
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM queued", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        64
    );
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.streaming/test]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_while_backpressured_is_terminal() {
    let (client, task) = session().await;
    let mut rows = Box::pin(
        client
            .query_raw(MANY, std::iter::empty::<i32>())
            .await
            .unwrap(),
    );
    assert!(rows.try_next().await.unwrap().is_some());
    client.cancel_token().cancel_query(NoTls).await.unwrap();
    let error = timeout(LIMIT, async {
        loop {
            match rows.try_next().await {
                Ok(Some(_)) => {}
                Err(error) => break error,
                Ok(None) => panic!("cancelled producer became successful EOF"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(error.code().unwrap().code(), "57014");
    assert_eq!(rows.rows_affected(), None);
    timeout(LIMIT, client.check_connection())
        .await
        .unwrap()
        .unwrap();
    finish(client, task).await;
}

// [spec:pgorm:sem:turso.streaming/test]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_close_terminates_waiting_work() {
    let (client, task) = session().await;
    let mut rows = Box::pin(
        client
            .query_raw(MANY, std::iter::empty::<i32>())
            .await
            .unwrap(),
    );
    assert!(rows.try_next().await.unwrap().is_some());
    let waiting_client = Arc::clone(&client);
    let waiting = tokio::spawn(async move { waiting_client.check_connection().await });
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        timeout(LIMIT, waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .is_closed()
    );
    let error = timeout(LIMIT, async {
        loop {
            match rows.try_next().await {
                Ok(Some(_)) => {}
                Err(error) => break error,
                Ok(None) => panic!("closed producer became successful EOF"),
            }
        }
    })
    .await
    .unwrap();
    assert!(error.is_closed() || error.code().is_some_and(|code| code.code() == "57014"));
    assert!(client.check_connection().await.unwrap_err().is_closed());
    assert_eq!(rows.rows_affected(), None);
}
