//! Real ORM transactions over a test-only native connection factory.

use crate::{
    Connection, ConnectionMut, DatabaseConnection, DatabasePool, DecodeRaw, Error, IsolationLevel,
    Transact, TransactionError, TransactionMode,
};
use futures::{TryStreamExt, future::BoxFuture};
use pgorm_driver::{Client, Config, TursoDatabase, error::SqlState};
use pgorm_pool::{Connect, Manager, ManagerConfig, Pool, RecyclingMethod, Unpooled};
use pgorm_query::Values;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::oneshot, task::JoinHandle, time::timeout};

const LIMIT: Duration = Duration::from_secs(5);
const MANY: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT x::bigint FROM n";

struct NativeConnector {
    database: TursoDatabase,
    connections: Arc<AtomicUsize>,
}

impl Connect for NativeConnector {
    fn connect<'a>(
        &'a self,
        _: &'a Config,
    ) -> BoxFuture<'a, Result<(Client, JoinHandle<()>), pgorm_driver::Error>> {
        Box::pin(async {
            let (client, connection) = self.database.connect().await?;
            self.connections.fetch_add(1, Ordering::SeqCst);
            let task = tokio::spawn(async move {
                // Disposal may deliberately close a quarantined worker.
                let _ = connection.await;
            });
            Ok((client, task))
        })
    }

    fn connect_raw<'a>(
        &'a self,
        _: &'a Config,
    ) -> BoxFuture<'a, Result<Unpooled, pgorm_driver::Error>> {
        Box::pin(async { unreachable!("this fixture never enables a notification listener") })
    }
}

async fn pool(recycling_method: RecyclingMethod) -> (DatabasePool, Arc<AtomicUsize>) {
    let connections = Arc::new(AtomicUsize::new(0));
    let manager = Manager::from_connect(
        Config::new(),
        NativeConnector {
            database: TursoDatabase::open(":memory:")
                .await
                .expect("open transaction fixture"),
            connections: Arc::clone(&connections),
        },
        ManagerConfig {
            recycling_method,
            ..ManagerConfig::default()
        },
    );
    let pool = DatabasePool(
        Pool::builder(manager)
            .max_size(1)
            .build()
            .expect("build single-connection fixture pool"),
    );
    acquire(&pool)
        .await
        .batch_execute("CREATE TABLE tx_values(id INTEGER PRIMARY KEY)")
        .await
        .expect("create transaction fixture table");
    (pool, connections)
}

async fn acquire(pool: &DatabasePool) -> DatabaseConnection {
    timeout(LIMIT, pool.get())
        .await
        .expect("pool acquisition timed out")
        .expect("acquire native fixture connection")
}

async fn insert(db: &impl ConnectionMut, id: i32) -> Result<(), Error> {
    let inserted = (
        "INSERT INTO tx_values VALUES ($1) RETURNING id",
        Values(vec![id.into()]),
    )
        .into_tuple::<i32>()
        .one(db)
        .await?;
    assert_eq!(inserted, id);
    Ok(())
}

async fn ids(db: &impl ConnectionMut) -> Result<Vec<i32>, Error> {
    Ok(db
        .query_all("SELECT id FROM tx_values ORDER BY id", &[])
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect())
}

fn assert_aborted(outcome: Result<(), TransactionError<Error>>) {
    match outcome {
        Err(TransactionError::Connection(Error::TransactionAborted(refusal))) => {
            assert_eq!(refusal.code(), Some(&SqlState::IN_FAILED_SQL_TRANSACTION));
            assert!(std::error::Error::source(&refusal).is_some());
        }
        other => panic!("expected a caught native abort, got {other:?}"),
    }
}

// [spec:pgorm:sem:turso.transactions/test]
#[tokio::test]
async fn closures_commit_and_await_rollback() -> Result<(), Error> {
    let (pool, connections) = pool(RecyclingMethod::Fast).await;
    let mut db = acquire(&pool).await;
    db.transaction(async |txn| insert(txn, 1).await).await?;
    let failed = db
        .transaction(async |txn| {
            insert(txn, 2).await?;
            Err::<(), _>(Error::Custom("closure refused".into()))
        })
        .await;
    assert!(matches!(
        failed,
        Err(TransactionError::Transaction(Error::Custom(message))) if message == "closure refused"
    ));
    assert_eq!(ids(&db).await?, [1]);
    drop(db);
    assert_eq!(ids(&acquire(&pool).await).await?, [1]);
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    Ok(())
}

// [spec:pgorm:sem:turso.transactions/test]
#[tokio::test]
async fn nested_closures_commit_or_rollback_independently() -> Result<(), Error> {
    let (pool, _) = pool(RecyclingMethod::Fast).await;
    let mut db = acquire(&pool).await;
    db.transaction(async |outer| {
        insert(outer, 1).await?;
        outer
            .transaction(async |inner| insert(inner, 2).await)
            .await?;
        let rolled_back = outer
            .transaction(async |inner| {
                insert(inner, 3).await?;
                inner
                    .transaction(async |nested| insert(nested, 4).await)
                    .await?;
                Err::<(), _>(Error::Custom("rollback savepoint".into()))
            })
            .await;
        assert!(matches!(rolled_back, Err(TransactionError::Transaction(_))));
        assert_eq!(ids(outer).await?, [1, 2]);
        insert(outer, 5).await
    })
    .await?;
    assert_eq!(ids(&db).await?, [1, 2, 5]);
    Ok(())
}

// [spec:pgorm:sem:turso.transactions/test]
#[tokio::test]
async fn caught_native_abort_is_reported_without_retry() -> Result<(), Error> {
    let (pool, _) = pool(RecyclingMethod::Fast).await;
    let mut db = acquire(&pool).await;
    let mut attempts = 0;
    let outcome = db
        .transaction_with_retry(TransactionMode::Default, 3, async |txn| {
            attempts += 1;
            insert(txn, 1).await?;
            let duplicate = insert(txn, 1).await.unwrap_err();
            assert!(matches!(duplicate, Error::Postgres(error) if error.code() == Some(&SqlState::UNIQUE_VIOLATION)));
            Ok::<_, Error>(())
        })
        .await;
    assert_aborted(outcome);
    assert_eq!(attempts, 1);
    assert!(ids(&db).await?.is_empty());
    db.transaction(async |txn| insert(txn, 2).await).await?;
    assert_eq!(ids(&db).await?, [2]);
    Ok(())
}

// [spec:pgorm:sem:turso.transactions/test]
#[tokio::test]
async fn caught_savepoint_abort_preserves_outer_work() -> Result<(), Error> {
    let (pool, _) = pool(RecyclingMethod::Fast).await;
    let mut db = acquire(&pool).await;
    db.transaction(async |outer| {
        insert(outer, 1).await?;
        let failed = outer
            .transaction(async |inner| {
                insert(inner, 2).await?;
                assert!(inner.batch_execute("SELECT 1 / 0").await.is_err());
                Ok::<_, Error>(())
            })
            .await;
        assert_aborted(failed);
        assert_eq!(ids(outer).await?, [1]);
        insert(outer, 3).await
    })
    .await?;
    assert_eq!(ids(&db).await?, [1, 3]);
    Ok(())
}

// [spec:pgorm:sem:turso.transactions/test]
#[tokio::test]
async fn unsupported_modes_never_enter_the_closure() -> Result<(), Error> {
    let (pool, _) = pool(RecyclingMethod::Fast).await;
    let mut db = acquire(&pool).await;
    let modes = [
        TransactionMode::ReadWrite { isolation: None },
        TransactionMode::ReadWrite {
            isolation: Some(IsolationLevel::RepeatableRead),
        },
    ];
    for mode in modes {
        let mut calls = 0;
        let result = db
            .transaction_with(mode, async |_| {
                calls += 1;
                Ok::<_, Error>(())
            })
            .await;
        assert!(matches!(result, Err(TransactionError::Connection(_))));
        assert_eq!(calls, 0);
        assert!(ids(&db).await?.is_empty());
    }
    for isolation in [None, Some(IsolationLevel::Serializable)] {
        let mut calls = 0;
        let result = db
            .read_only_transaction(isolation, async |_| {
                calls += 1;
                Ok::<_, Error>(())
            })
            .await;
        assert!(matches!(result, Err(TransactionError::Connection(_))));
        assert_eq!(calls, 0);
    }
    let mut calls = 0;
    let result = db
        .snapshot(async |_| {
            calls += 1;
            Ok::<_, Error>(())
        })
        .await;
    assert!(matches!(result, Err(TransactionError::Connection(_))));
    assert_eq!(calls, 0);
    db.transaction_with(TransactionMode::Default, async |txn| insert(txn, 1).await)
        .await?;
    assert_eq!(ids(&db).await?, [1]);
    Ok(())
}

async fn abandon_closure(db: &mut DatabaseConnection) {
    let (sent, ready) = oneshot::channel();
    let transaction = db.transaction(async |txn| {
        insert(txn, 1).await?;
        sent.send(()).expect("signal completed transaction write");
        std::future::pending::<Result<(), Error>>().await
    });
    tokio::select! {
        result = transaction => panic!("unfinished transaction returned: {result:?}"),
        ready = ready => ready.expect("transaction write signal was dropped"),
    }
}

// [spec:pgorm:sem:turso.cleanup/test]
#[tokio::test]
async fn dropped_closure_rolls_back_before_later_work() -> Result<(), Error> {
    let (pool, _) = pool(RecyclingMethod::Fast).await;
    let mut db = acquire(&pool).await;
    abandon_closure(&mut db).await;
    assert!(timeout(LIMIT, ids(&db)).await.unwrap()?.is_empty());
    db.transaction(async |txn| insert(txn, 2).await).await?;
    assert_eq!(ids(&db).await?, [2]);
    Ok(())
}

// [spec:pgorm:sem:turso.cleanup/test]
#[tokio::test]
async fn dropped_nested_closure_recovers_before_outer_commit() -> Result<(), Error> {
    let (pool, _) = pool(RecyclingMethod::Fast).await;
    let mut db = acquire(&pool).await;
    db.transaction(async |outer| {
        insert(outer, 1).await?;
        let (sent, ready) = oneshot::channel();
        let nested = outer.transaction(async |inner| {
            insert(inner, 2).await?;
            sent.send(()).unwrap();
            std::future::pending::<Result<(), Error>>().await
        });
        tokio::select! {
            result = nested => panic!("unfinished savepoint returned: {result:?}"),
            ready = ready => ready.unwrap(),
        }
        assert_eq!(timeout(LIMIT, ids(outer)).await.unwrap()?, [1]);
        insert(outer, 3).await
    })
    .await?;
    assert_eq!(ids(&db).await?, [1, 3]);
    Ok(())
}

// [spec:pgorm:sem:turso.cleanup/test]
#[tokio::test]
async fn pending_savepoint_restore_precedes_cached_insert() -> Result<(), Error> {
    let (pool, _) = pool(RecyclingMethod::Fast).await;
    let mut db = acquire(&pool).await;
    db.transaction(async |outer| {
        insert(outer, 1).await?;
        let (sent, ready) = oneshot::channel();
        let nested = outer.transaction(async |inner| {
            insert(inner, 2).await?;
            let mut rows = Box::pin(inner.query_raw(MANY, std::iter::empty::<i32>()).await?);
            assert_eq!(rows.try_next().await?.unwrap().get::<_, i64>(0), 1);
            sent.send(rows)
                .unwrap_or_else(|_| panic!("test receiver was dropped"));
            std::future::pending::<Result<(), Error>>().await
        });
        let rows = tokio::select! {
            result = nested => panic!("unfinished savepoint returned: {result:?}"),
            rows = ready => rows.unwrap(),
        };
        // Dropping nested queued ROLLBACK TO, but the retained producer keeps
        // it pending. Poll the next operation far enough to select its cache
        // path while this boundary is guaranteed not to have completed.
        let mut pending = Box::pin(insert(outer, 3));
        assert!(futures::poll!(pending.as_mut()).is_pending());
        drop(rows);
        timeout(LIMIT, pending).await.unwrap()?;
        Ok::<_, Error>(())
    })
    .await?;
    assert_eq!(ids(&db).await?, [1, 3]);
    Ok(())
}

// [spec:pgorm:sem:turso.cleanup/test]
#[tokio::test]
async fn dropped_stream_is_cleaned_before_transaction_rollback() -> Result<(), Error> {
    let (pool, _) = pool(RecyclingMethod::Fast).await;
    let mut db = acquire(&pool).await;
    let outcome = db
        .transaction(async |txn| {
            insert(txn, 1).await?;
            let mut rows = Box::pin(txn.query_raw(MANY, std::iter::empty::<i32>()).await?);
            assert_eq!(rows.try_next().await?.unwrap().get::<_, i64>(0), 1);
            drop(rows);
            Err::<(), _>(Error::Custom("rollback after partial stream".into()))
        })
        .await;
    assert!(matches!(outcome, Err(TransactionError::Transaction(_))));
    assert!(timeout(LIMIT, ids(&db)).await.unwrap()?.is_empty());
    drop(db);
    assert!(ids(&acquire(&pool).await).await?.is_empty());
    Ok(())
}

// [spec:pgorm:sem:turso.cleanup/test]
#[tokio::test]
async fn dropped_transaction_cannot_escape_through_pool_handback() -> Result<(), Error> {
    for method in [RecyclingMethod::Fast, RecyclingMethod::Verified] {
        let (pool, _) = pool(method).await;
        let mut db = acquire(&pool).await;
        abandon_closure(&mut db).await;
        drop(db);
        let mut next = acquire(&pool).await;
        assert!(ids(&next).await?.is_empty());
        next.transaction(async |txn| insert(txn, 2).await).await?;
        assert_eq!(ids(&next).await?, [2]);
    }
    Ok(())
}

// [spec:pgorm:sem:turso.cleanup/test]
#[tokio::test]
async fn raw_transaction_handback_disposes_the_connection() -> Result<(), Error> {
    for method in [RecyclingMethod::Fast, RecyclingMethod::Verified] {
        let (pool, connections) = pool(method).await;
        let db = acquire(&pool).await;
        db.batch_execute("BEGIN; INSERT INTO tx_values VALUES(1)")
            .await?;
        drop(db);
        let next = acquire(&pool).await;
        assert_eq!(connections.load(Ordering::SeqCst), 2);
        assert!(ids(&next).await?.is_empty());
        insert(&next, 2).await?;
        assert_eq!(ids(&next).await?, [2]);
    }
    Ok(())
}

// [spec:pgorm:sem:turso.cleanup/test]
#[tokio::test]
async fn custom_recycling_must_leave_an_idle_connection() -> Result<(), Error> {
    for (sql, expected_connections) in [("SELECT 1", 1), ("BEGIN", 2)] {
        let (pool, connections) = pool(RecyclingMethod::Custom(sql.into())).await;
        let db = acquire(&pool).await;
        assert_eq!(connections.load(Ordering::SeqCst), expected_connections);
        insert(&db, 1).await?;
        assert_eq!(ids(&db).await?, [1]);
    }
    Ok(())
}

#[tokio::test]
async fn postgres_clean_recycling_is_explicitly_refused() {
    let (pool, _) = pool(RecyclingMethod::Clean).await;
    let mut db = acquire(&pool).await;
    let error = deadpool::managed::Manager::recycle(
        pool.0.manager(),
        &mut db.0,
        &pgorm_pool::Metrics::default(),
    )
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not support PostgreSQL Clean")
    );
}

// [spec:pgorm:sem:turso.cleanup/test]
#[tokio::test]
async fn active_stream_handback_disposes_before_next_borrower() -> Result<(), Error> {
    let (pool, connections) = pool(RecyclingMethod::Fast).await;
    let db = acquire(&pool).await;
    let mut rows = Box::pin(db.query_raw(MANY, std::iter::empty::<i32>()).await?);
    assert_eq!(rows.try_next().await?.unwrap().get::<_, i64>(0), 1);
    // This producer cannot finish in its bounded buffer while rows is retained.
    drop(db);
    let next = acquire(&pool).await;
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    insert(&next, 1).await?;
    assert_eq!(ids(&next).await?, [1]);
    let stopped = timeout(LIMIT, async {
        loop {
            match rows.try_next().await {
                Ok(Some(_)) => {}
                Err(error) => break error,
                Ok(None) => panic!("abandoned active producer reported successful EOF"),
            }
        }
    })
    .await
    .unwrap();
    assert!(stopped.is_closed() || stopped.code() == Some(&SqlState::QUERY_CANCELED));
    Ok(())
}
