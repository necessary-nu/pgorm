#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, setup::*};

use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use futures::StreamExt;
use pgorm::{
    ConnectionTrait, DatabaseConnection, Error, ListenError, Listener, NoTls, Notification,
    Notifications, TransactionTrait, connect_listener,
    metric::{InstrumentedPool, MetricsCollector, QueryContext},
    notify,
};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

/// How long a notification that must arrive is waited for. Nothing waits for
/// one that must not: a later notification fences it, since a session
/// receives notifications in commit order.
const ARRIVAL: Duration = Duration::from_secs(10);

/// The next item the stream yields, as `(channel, payload)`.
async fn next(notifications: &mut Notifications) -> Result<(String, String), Error> {
    let notification = next_notification(notifications).await?;
    Ok((
        notification.channel().to_owned(),
        notification.payload().to_owned(),
    ))
}

async fn next_notification(notifications: &mut Notifications) -> Result<Notification, Error> {
    tokio::time::timeout(ARRIVAL, notifications.next())
        .await
        .expect("a notification arrives in time")
        .expect("the stream has not ended")
}

fn pair(channel: &str, payload: &str) -> (String, String) {
    (channel.to_owned(), payload.to_owned())
}

fn capacity(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("a test capacity is not zero")
}

fn sqlstate(error: &Error) -> Option<&SqlState> {
    match error {
        Error::Postgres(error) => error.code(),
        _ => None,
    }
}

async fn backend_pid(db: &DatabaseConnection) -> Result<i32, Error> {
    Ok(db.query_one("SELECT pg_backend_pid()", &[]).await?.get(0))
}

// [spec:pgorm:req:conn.listen/test]    a pooled transaction's notification arrives at commit, with its sender's pid
#[pgorm_macros::test]
async fn committed_notification_arrives_with_sender() -> Result<(), Error> {
    let ctx = TestContext::new("committed_notification_arrives").await;
    let (listener, mut notifications) = ctx.db.listener(Listener::DEFAULT_CAPACITY).await?;
    listener.listen("jobs").await?;

    let mut db = ctx.db.get().await?;
    let sender = backend_pid(&db).await?;
    db.transaction(async |txn| notify(&txn, "jobs", "42").await)
        .await
        .expect("the transaction commits");

    let notification = next_notification(&mut notifications).await?;
    assert_eq!(
        (
            notification.channel(),
            notification.payload(),
            notification.process_id()
        ),
        ("jobs", "42", sender)
    );

    drop((listener, notifications, db));
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.listen/test]    a rolled-back transaction's notification is never delivered
#[pgorm_macros::test]
async fn rolled_back_notification_never_arrives() -> Result<(), Error> {
    let ctx = TestContext::new("rolled_back_notification").await;
    let (listener, mut notifications) = ctx.db.listener(Listener::DEFAULT_CAPACITY).await?;
    listener.listen("jobs").await?;

    let mut db = ctx.db.get().await?;
    let outcome = db
        .transaction(async |txn| {
            notify(&txn, "jobs", "discarded").await?;
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);
    notify(&db, "jobs", "fence").await?;

    assert_eq!(next(&mut notifications).await?, pair("jobs", "fence"));

    drop((listener, notifications, db));
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.listen/test]    commit order across transactions, send order within one, duplicates folded
#[pgorm_macros::test]
async fn notifications_arrive_in_commit_order() -> Result<(), Error> {
    let ctx = TestContext::new("notifications_commit_order").await;
    let (listener, mut notifications) = ctx.db.listener(Listener::DEFAULT_CAPACITY).await?;
    listener.listen("a").await?;
    listener.listen("b").await?;

    let mut first = ctx.db.get().await?;
    let mut second = ctx.db.get().await?;
    // The late transaction runs to its commit inside the early one, so it
    // commits first though it began second.
    first
        .transaction(async |early| {
            second
                .transaction(async |late| {
                    for payload in ["1", "2", "1", "3"] {
                        notify(&early, "a", payload).await?;
                    }
                    notify(&early, "b", "1").await?;
                    notify(&late, "a", "first committed").await
                })
                .await
                .expect("the late transaction commits");
            Ok::<_, Error>(())
        })
        .await
        .expect("the early transaction commits");

    let mut received = Vec::new();
    for _ in 0..5 {
        received.push(next(&mut notifications).await?);
    }
    assert_eq!(
        received,
        [
            pair("a", "first committed"),
            pair("a", "1"),
            pair("a", "2"),
            pair("a", "3"),
            pair("b", "1"),
        ]
    );

    drop((listener, notifications, first, second));
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.listen/test]    unlisten and unlisten_all stop delivery; a repeated listen is one subscription
#[pgorm_macros::test]
async fn unlisten_stops_delivery() -> Result<(), Error> {
    let ctx = TestContext::new("unlisten_stops_delivery").await;
    let (listener, mut notifications) = ctx.db.listener(Listener::DEFAULT_CAPACITY).await?;
    listener.listen("jobs").await?;
    listener.listen("jobs").await?;
    listener.listen("fence").await?;
    let db = ctx.db.get().await?;

    notify(&db, "jobs", "heard").await?;
    assert_eq!(next(&mut notifications).await?, pair("jobs", "heard"));

    listener.unlisten("jobs").await?;
    listener.unlisten("never listened").await?;
    notify(&db, "jobs", "unheard").await?;
    notify(&db, "fence", "1").await?;
    assert_eq!(next(&mut notifications).await?, pair("fence", "1"));

    listener.listen("jobs").await?;
    listener.unlisten_all().await?;
    listener.listen("after").await?;
    notify(&db, "jobs", "unheard").await?;
    notify(&db, "fence", "unheard").await?;
    notify(&db, "after", "2").await?;
    assert_eq!(next(&mut notifications).await?, pair("after", "2"));

    drop((listener, notifications, db));
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.listen/test]    a channel name is exact: case, spaces and quotes kept, a case variant another channel
#[pgorm_macros::test]
async fn quoted_channel_names_are_exact() -> Result<(), Error> {
    let ctx = TestContext::new("quoted_channel_names").await;
    let (listener, mut notifications) = ctx.db.listener(Listener::DEFAULT_CAPACITY).await?;
    let odd = r#"Mixed Case "quoted"; DROP"#;
    listener.listen(odd).await?;
    listener.listen("fence").await?;
    let db = ctx.db.get().await?;

    notify(&db, &odd.to_lowercase(), "other channel").await?;
    notify(&db, odd, "exact").await?;
    notify(&db, "fence", "1").await?;
    assert_eq!(next(&mut notifications).await?, pair(odd, "exact"));
    assert_eq!(next(&mut notifications).await?, pair("fence", "1"));

    let listening: Vec<String> = db
        .query_all(
            "SELECT pid::text FROM pg_stat_activity \
             WHERE datname = current_database() AND query = $1",
            &[&format!("LISTEN {}", r#""fence""#)],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(listening.len(), 1, "the LISTEN was sent quoted");

    drop((listener, notifications, db));
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.listen/test]    a name past 63 bytes is refused before it is sent; 63 bytes works end to end; an empty one is the server's error
// [spec:pgorm:def:error.model+12/test]    a listener's refusal converts into Error::Listen
#[pgorm_macros::test]
async fn channel_name_past_63_bytes_is_refused() -> Result<(), Error> {
    let ctx = TestContext::new("channel_name_past_63_bytes").await;
    let (listener, mut notifications) = ctx.db.listener(Listener::DEFAULT_CAPACITY).await?;

    let longest = format!("é{}", "c".repeat(61));
    assert_eq!(longest.len(), 63);
    let too_long = format!("{longest}c");
    for refused in [
        listener.listen(&too_long).await,
        listener.unlisten(&too_long).await,
    ] {
        assert_eq!(
            refused,
            Err(Error::Listen(ListenError::ChannelTooLong {
                channel: too_long.clone()
            }))
        );
    }

    listener.listen(&longest).await?;
    let db = ctx.db.get().await?;
    notify(&db, &longest, "whole").await?;
    assert_eq!(next(&mut notifications).await?, pair(&longest, "whole"));

    let refused = notify(&db, &too_long, "x")
        .await
        .expect_err("pg_notify refuses it");
    assert_eq!(sqlstate(&refused), Some(&SqlState::INVALID_PARAMETER_VALUE));

    let empty = listener.listen("").await.expect_err("LISTEN refuses it");
    assert_eq!(sqlstate(&empty), Some(&SqlState::SYNTAX_ERROR));
    let empty = notify(&db, "", "x")
        .await
        .expect_err("pg_notify refuses it");
    assert_eq!(sqlstate(&empty), Some(&SqlState::INVALID_PARAMETER_VALUE));
    listener.listen(&longest).await?;

    drop((listener, notifications, db));
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.listen/test]    the payload bound is the server's: 7999 bytes arrive, 8000 are refused with 22023
#[pgorm_macros::test]
async fn payload_limit_is_the_servers_error() -> Result<(), Error> {
    let ctx = TestContext::new("payload_limit_servers_error").await;
    let (listener, mut notifications) = ctx.db.listener(Listener::DEFAULT_CAPACITY).await?;
    listener.listen("jobs").await?;
    let mut db = ctx.db.get().await?;

    let largest = "p".repeat(7999);
    notify(&db, "jobs", &largest).await?;
    assert_eq!(next(&mut notifications).await?, pair("jobs", &largest));

    let refused = notify(&db, "jobs", &"p".repeat(8000))
        .await
        .expect_err("the server refuses an 8000-byte payload");
    assert_eq!(sqlstate(&refused), Some(&SqlState::INVALID_PARAMETER_VALUE));

    let outcome = db
        .transaction(async |txn| {
            assert!(notify(&txn, "jobs", &"p".repeat(8000)).await.is_err());
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);
    notify(&db, "jobs", "fence").await?;
    assert_eq!(next(&mut notifications).await?, pair("jobs", "fence"));

    drop((listener, notifications, db));
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.listen/test]    a terminated backend ends the stream with its error, after what had arrived
#[pgorm_macros::test]
async fn terminated_connection_ends_the_stream() -> Result<(), Error> {
    let ctx = TestContext::new("terminated_connection_ends").await;
    let (listener, mut notifications) = ctx.db.listener(Listener::DEFAULT_CAPACITY).await?;
    listener.listen("doomed").await?;
    let db = ctx.db.get().await?;

    let pid: i32 = db
        .query_one(
            "SELECT pid FROM pg_stat_activity \
             WHERE datname = current_database() AND query = 'LISTEN \"doomed\"'",
            &[],
        )
        .await?
        .get(0);
    notify(&db, "doomed", "1").await?;
    assert_eq!(next(&mut notifications).await?, pair("doomed", "1"));
    notify(&db, "doomed", "2").await?;
    notify(&db, "doomed", "3").await?;
    // The listening session sends what it has pending before it answers a
    // request, so once this returns both are waiting in the stream.
    listener.listen("doomed").await?;
    let terminated = db
        .execute(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
             WHERE pid = $1 AND datname = current_database()",
            &[&pid],
        )
        .await?;
    assert_eq!(
        terminated, 1,
        "the listening session is in this test's database"
    );

    let mut received = Vec::new();
    let failure = loop {
        match tokio::time::timeout(ARRIVAL, notifications.next())
            .await
            .expect("the stream ends in time")
        {
            Some(Ok(notification)) => received.push(notification.payload().to_owned()),
            Some(Err(error)) => break error,
            None => panic!("the stream ended without an error"),
        }
    };
    assert_eq!(received, ["2", "3"]);
    assert_eq!(sqlstate(&failure), Some(&SqlState::ADMIN_SHUTDOWN));
    assert!(notifications.next().await.is_none());
    assert!(matches!(
        listener.listen("again").await,
        Err(Error::Postgres(_))
    ));

    drop((listener, notifications, db));
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.listen/test]    past its capacity the listener closes and the stream ends with Overflow
// [spec:pgorm:def:error.model+12/test]    the overflow that ends a listener's stream is Error::Listen
#[pgorm_macros::test]
async fn overflow_closes_the_listener() -> Result<(), Error> {
    let ctx = TestContext::new("overflow_closes_the_listener").await;
    let (listener, mut notifications) = ctx.db.listener(capacity(2)).await?;
    listener.listen("burst").await?;
    let mut db = ctx.db.get().await?;

    db.transaction(async |txn| {
        for payload in ["1", "2", "3", "4"] {
            notify(&txn, "burst", payload).await?;
        }
        Ok::<_, Error>(())
    })
    .await
    .expect("the transaction commits");

    // Nothing reads the stream until the listener has closed, so the overflow
    // is decided by the capacity alone. Each probe is a round trip the
    // listener's session answers after delivering what it has pending.
    tokio::time::timeout(ARRIVAL, async {
        while listener.listen("probe").await.is_ok() {}
    })
    .await
    .expect("the listener closes in time");

    assert_eq!(next(&mut notifications).await?, pair("burst", "1"));
    assert_eq!(next(&mut notifications).await?, pair("burst", "2"));
    let overflow = notifications.next().await;
    assert!(
        matches!(
            overflow,
            Some(Err(Error::Listen(ListenError::Overflow { capacity: 2 })))
        ),
        "{overflow:?}"
    );
    assert!(notifications.next().await.is_none());

    drop((listener, notifications, db));
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.listen/test]    dropping the stream closes the connection; a clone listens while another task reads
#[pgorm_macros::test]
async fn dropped_stream_closes_the_connection() -> Result<(), Error> {
    let ctx = TestContext::new("dropped_stream_closes").await;
    let (listener, mut notifications) = ctx.db.listener(Listener::DEFAULT_CAPACITY).await?;
    let db = ctx.db.get().await?;

    let reader = tokio::spawn(async move {
        let heard = next(&mut notifications).await;
        (heard, notifications)
    });
    let clone = listener.clone();
    clone.listen("late").await?;
    notify(&db, "late", "heard").await?;
    let (heard, notifications) = reader.await.expect("the reader finishes");
    assert_eq!(heard?, pair("late", "heard"));

    drop(notifications);
    let closed = tokio::time::timeout(ARRIVAL, async {
        loop {
            if let Err(error) = listener.listen("late").await {
                break error;
            }
        }
    })
    .await
    .expect("the connection closes in time");
    assert!(matches!(closed, Error::Postgres(_)));

    // The server ends the session once it reads the closed socket.
    tokio::time::timeout(ARRIVAL, async {
        loop {
            let listening: i64 = db
                .query_one(
                    "SELECT count(*) FROM pg_stat_activity \
                     WHERE datname = current_database() AND query LIKE 'LISTEN%'",
                    &[],
                )
                .await?
                .get(0);
            if listening == 0 {
                return Ok::<_, Error>(());
            }
        }
    })
    .await
    .expect("the listening session ends in time")?;

    drop((listener, clone, db));
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.listen/test]    connect_listener opens a listener from a config and a TLS connector
#[pgorm_macros::test]
async fn connect_listener_from_config() -> Result<(), Error> {
    let ctx = TestContext::new("connect_listener_from_config").await;
    let base_url = std::env::var("DATABASE_URL").expect("DATABASE_URL is set");
    let (listener, mut notifications) =
        connect_listener(config(&base_url, ctx.db_name()), NoTls, capacity(8)).await?;
    listener.listen("direct").await?;

    let db = ctx.db.get().await?;
    notify(&db, "direct", "").await?;
    assert_eq!(next(&mut notifications).await?, pair("direct", ""));
    assert_eq!(
        ctx.db.status().size,
        1,
        "the listener took no pooled connection"
    );

    drop((listener, notifications, db));
    ctx.delete().await;
    Ok(())
}

#[derive(Clone, Debug, Default)]
struct Operations(Arc<Mutex<Vec<String>>>);

#[async_trait]
impl MetricsCollector for Operations {
    async fn record_query_success(
        &self,
        query: QueryContext<'_>,
        _duration: Duration,
        rows: Option<u64>,
    ) {
        self.0.lock().unwrap().push(format!(
            "ok:{}:{rows:?}:{}",
            query.operation(),
            query.sql().unwrap_or_default()
        ));
    }

    async fn record_query_error(
        &self,
        query: QueryContext<'_>,
        _duration: Duration,
        _error: &Error,
    ) {
        self.0
            .lock()
            .unwrap()
            .push(format!("error:{}", query.operation()));
    }

    async fn record_connection_acquired(&self, _duration: Duration) {}
    async fn record_connection_error(&self, _duration: Duration, _error: &Error) {}
    async fn record_transaction_begin(&self, _duration: Duration) {}
    async fn record_transaction_commit(&self, _duration: Duration) {}
    async fn record_transaction_rollback(&self, _duration: Duration) {}
}

// [spec:pgorm:req:conn.listen/test]    notify through an instrumented connection is reported as its execute
#[pgorm_macros::test]
async fn instrumented_notify_is_an_execute() -> Result<(), Error> {
    let ctx = TestContext::new("instrumented_notify_execute").await;
    let metrics = Operations::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), metrics.clone());
    let (listener, mut notifications) = pool.inner().listener(capacity(4)).await?;
    listener.listen("metered").await?;

    let mut db = pool.get().await?;
    notify(&db, "metered", "1").await?;
    db.transaction(async |txn| notify(&*txn, "metered", "2").await)
        .await
        .expect("the instrumented transaction commits");
    assert_eq!(next(&mut notifications).await?, pair("metered", "1"));
    assert_eq!(next(&mut notifications).await?, pair("metered", "2"));
    assert_eq!(
        metrics.0.lock().unwrap().clone(),
        [
            "ok:execute:Some(1):SELECT pg_notify($1, $2)",
            "ok:execute:Some(1):SELECT pg_notify($1, $2)",
        ]
    );

    drop((listener, notifications, db));
    ctx.delete().await;
    Ok(())
}
