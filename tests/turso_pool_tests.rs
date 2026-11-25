//! Public constructors and lifecycle of experimental native pools.
#![cfg(feature = "turso")]

use futures::TryStreamExt;
use pgorm::{
    Connection, DecodeRaw, Error, Hook, HookConnection, HookError, ManagerConfig, PoolBuilder,
    PoolError, RecyclingMethod, Runtime, TimeoutType, Transact, connect_turso, connect_turso_with,
};
use pgorm_driver::{Statement, types::Type};
use pgorm_query::Values;
use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(5);
const TYPED_SQL: &str = "SELECT $1";
const MANY: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT x::bigint FROM n";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("pgorm-native-pool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).expect("create an isolated native pool test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn typed_value(db: &impl Connection, value: i32) -> Result<i32, Error> {
    (TYPED_SQL, Values(vec![value.into()]))
        .into_tuple::<i32>()
        .one(db)
        .await
}

fn cached_hook(statements: Arc<Mutex<Vec<Statement>>>) -> Hook {
    Hook::async_fn(move |client, _| {
        let statements = Arc::clone(&statements);
        Box::pin(async move {
            let statement = client
                .prepare_typed_cached(TYPED_SQL, &[Type::INT4])
                .await
                .map_err(HookError::Backend)?;
            let db = HookConnection::new(client);
            assert_eq!(
                typed_value(&db, 7)
                    .await
                    .map_err(|error| HookError::message(error.to_string()))?,
                7
            );
            assert_eq!(client.statement_cache.size(), 1);
            statements
                .lock()
                .expect("record hook metadata")
                .push(statement);
            Ok(())
        })
    })
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn default_constructor_returns_an_open_usable_pool() -> Result<(), Error> {
    let pool = connect_turso(":memory:").await?;
    assert_eq!(
        pool.status().size,
        1,
        "construction establishes a connection"
    );
    assert_eq!(pool.status().available, 1);
    assert!(!pool.is_closed());
    let db = pool.get().await?;
    assert_eq!(db.query_one("SELECT 1", &[]).await?.get::<_, i32>(0), 1);
    Ok(())
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn opening_and_post_create_failures_reach_the_constructor() {
    let scratch = Scratch::new();
    let opening = connect_turso(scratch.path().join("missing").join("database.db"))
        .await
        .expect_err("opening in a missing directory must fail eagerly");
    assert!(matches!(opening, Error::Postgres(_) | Error::Pool(_)));

    let calls = Arc::new(AtomicUsize::new(0));
    let called = Arc::clone(&calls);
    let hook = Hook::sync_fn(move |_, _| {
        called.fetch_add(1, Ordering::SeqCst);
        Err(HookError::message("creation hook refused"))
    });
    let failed = connect_turso_with(":memory:", ManagerConfig::default(), |builder| {
        builder.post_create(hook)
    })
    .await
    .expect_err("post-create failure must not yield a pool");
    assert!(matches!(failed, Error::Pool(_)));
    assert!(failed.to_string().contains("creation hook refused"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn unsupported_options_are_refused_before_file_creation() {
    let scratch = Scratch::new();
    let configurations = [
        ManagerConfig {
            recycling_method: RecyclingMethod::Clean,
            ..ManagerConfig::default()
        },
        ManagerConfig {
            statement_cache_channel: Some("cache_events".into()),
            ..ManagerConfig::default()
        },
        ManagerConfig {
            statement_cache_channel: Some(String::new()),
            ..ManagerConfig::default()
        },
        ManagerConfig {
            statement_cache_channel: Some("x".repeat(64)),
            ..ManagerConfig::default()
        },
    ];
    for (index, manager) in configurations.into_iter().enumerate() {
        let path = scratch.path().join(format!("refused-{index}.db"));
        let mut built = false;
        let error = connect_turso_with(&path, manager, |builder| {
            built = true;
            builder
        })
        .await
        .expect_err("unsupported native manager setting must be refused");
        assert!(matches!(error, Error::Custom(_)), "{error:?}");
        assert!(!path.exists());
        assert!(!built, "unsupported settings precede builder side effects");
    }
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn borrowers_share_memory_while_separate_pools_are_isolated() -> Result<(), Error> {
    let pool = connect_turso_with(":memory:", ManagerConfig::default(), |builder| {
        builder.max_size(2)
    })
    .await?;
    let first = pool.get().await?;
    let second = pool.get().await?;
    assert_eq!(pool.status().size, 2);
    first
        .batch_execute(
            "CREATE TABLE shared_values(id INTEGER); INSERT INTO shared_values VALUES(1)",
        )
        .await?;
    assert_eq!(
        second
            .query_one("SELECT id FROM shared_values", &[])
            .await?
            .get::<_, i32>(0),
        1
    );
    second
        .execute("INSERT INTO shared_values VALUES(2)", &[])
        .await?;
    assert_eq!(
        first
            .query_one("SELECT count(*) FROM shared_values", &[])
            .await?
            .get::<_, i64>(0),
        2
    );
    let separate = connect_turso(":memory:").await?;
    let isolated = separate.get().await?;
    isolated
        .batch_execute("CREATE TABLE shared_values(id INTEGER)")
        .await?;
    assert_eq!(
        isolated
            .query_one("SELECT count(*) FROM shared_values", &[])
            .await?
            .get::<_, i64>(0),
        0
    );
    Ok(())
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn hooks_reuse_and_clear_typed_cache() -> Result<(), Error> {
    let statements = Arc::new(Mutex::new(Vec::new()));
    let created = cached_hook(Arc::clone(&statements));
    let recycled = cached_hook(Arc::clone(&statements));
    let pool = connect_turso_with(
        ":memory:",
        ManagerConfig {
            tag: Some("native-reader".into()),
            recycling_method: RecyclingMethod::Verified,
            ..ManagerConfig::default()
        },
        |builder| {
            builder
                .max_size(1)
                .post_create(created)
                .post_recycle(recycled)
        },
    )
    .await?;
    assert_eq!(pool.tag().as_str(), "native-reader");
    assert_eq!(pool.status().max_size, 1);
    assert_eq!(statements.lock().expect("read creation hook").len(), 1);
    let first = pool.get().await?;
    assert_eq!(typed_value(&first, 11).await?, 11);
    drop(first);
    let second = pool.get().await?;
    assert_eq!(typed_value(&second, 12).await?, 12);
    {
        let recorded = statements.lock().expect("read reused metadata");
        assert_eq!(recorded.len(), 3);
        assert!(
            recorded.windows(2).all(|pair| {
                std::ptr::eq(pair[0].columns().as_ptr(), pair[1].columns().as_ptr())
            })
        );
    }
    pool.clear_statement_caches();
    drop(second);
    let third = pool.get().await?;
    assert_eq!(typed_value(&third, 13).await?, 13);
    let recorded = statements.lock().expect("read cleared metadata");
    assert_eq!(recorded.len(), 4);
    assert!(!std::ptr::eq(
        recorded[0].columns().as_ptr(),
        recorded[3].columns().as_ptr()
    ));
    Ok(())
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn configured_wait_timeout_preserves_pool_progress() -> Result<(), Error> {
    let configure: fn(PoolBuilder) -> PoolBuilder = |builder| {
        builder
            .max_size(1)
            .runtime(Runtime::Tokio1)
            .wait_timeout(Some(Duration::from_millis(25)))
    };
    let pool = connect_turso_with(":memory:", ManagerConfig::default(), configure).await?;
    let held = pool.get().await?;
    let refused = timeout(LIMIT, pool.get())
        .await
        .expect("configured wait timeout must finish");
    assert!(matches!(
        refused,
        Err(Error::Pool(PoolError::Timeout(TimeoutType::Wait)))
    ));
    drop(held);
    let next = timeout(LIMIT, pool.get())
        .await
        .expect("returning the client unblocks acquisition")?;
    assert_eq!(typed_value(&next, 42).await?, 42);
    Ok(())
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn invalid_builders_return_errors_without_waiting() {
    let zero = timeout(
        LIMIT,
        connect_turso_with(":memory:", ManagerConfig::default(), |builder| {
            builder.max_size(0)
        }),
    )
    .await
    .expect("zero-capacity pool must not wait for an impossible checkout")
    .expect_err("zero-capacity pool must fail eager initialization");
    assert!(matches!(zero, Error::Custom(_)));
    let missing_runtime = connect_turso_with(":memory:", ManagerConfig::default(), |builder| {
        builder.wait_timeout(Some(Duration::from_secs(1)))
    })
    .await
    .expect_err("a timeout requires an explicit pool runtime");
    assert!(matches!(missing_runtime, Error::Custom(_)));
    assert!(
        missing_runtime
            .to_string()
            .contains("Timeouts require a runtime")
    );
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn listener_refusal_leaves_the_native_pool_usable() -> Result<(), Error> {
    let pool = connect_turso_with(":memory:", ManagerConfig::default(), |builder| {
        builder.max_size(1)
    })
    .await?;
    let held = pool.get().await?;
    let result = timeout(LIMIT, pool.listener(NonZeroUsize::MIN))
        .await
        .expect("unsupported native listener must refuse immediately");
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("native pool must not return a notification listener"),
    };
    match error {
        Error::Postgres(error) => {
            assert!(error.code().is_none());
            assert!(
                error
                    .to_string()
                    .contains("native notification connections")
            );
        }
        other => panic!("expected native unsupported error, got {other:?}"),
    }
    assert_eq!(pool.status().size, 1);
    assert_eq!(typed_value(&held, 9).await?, 9);
    Ok(())
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn pool_close_keeps_a_checked_out_connection_usable() -> Result<(), Error> {
    let pool = connect_turso(":memory:").await?;
    let mut held = pool.get().await?;
    pool.close();
    assert!(pool.is_closed());
    assert!(matches!(
        pool.get().await,
        Err(Error::Pool(PoolError::Closed))
    ));
    held.batch_execute("CREATE TABLE kept(id INTEGER)").await?;
    held.transaction(async |txn| {
        txn.execute("INSERT INTO kept VALUES(7)", &[]).await?;
        Ok::<_, Error>(())
    })
    .await?;
    assert_eq!(
        held.query_one("SELECT id FROM kept", &[])
            .await?
            .get::<_, i32>(0),
        7
    );
    Ok(())
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn committed_file_data_survives_pool_reopen() -> Result<(), Error> {
    let scratch = Scratch::new();
    let path = scratch.path().join("persistent.db");
    let pool = connect_turso(&path).await?;
    let mut db = pool.get().await?;
    db.batch_execute("CREATE TABLE durable(value BIGINT)")
        .await?;
    db.transaction(async |txn| {
        let stored = (
            "INSERT INTO durable VALUES($1) RETURNING value",
            Values(vec![9_007_199_254_740_993i64.into()]),
        )
            .into_tuple::<i64>()
            .one(txn)
            .await?;
        assert_eq!(stored, 9_007_199_254_740_993);
        Ok::<_, Error>(())
    })
    .await?;
    drop(db);
    pool.close();
    drop(pool);
    assert!(path.is_file());
    let reopened = connect_turso(path).await?;
    let db = reopened.get().await?;
    assert_eq!(
        db.query_one("SELECT value FROM durable", &[])
            .await?
            .get::<_, i64>(0),
        9_007_199_254_740_993
    );
    Ok(())
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn a_creation_hook_cannot_leave_a_transaction_open() {
    let hook = Hook::async_fn(|client, _| {
        Box::pin(async move {
            client
                .batch_execute("BEGIN")
                .await
                .map_err(HookError::Backend)
        })
    });
    let error = connect_turso_with(":memory:", ManagerConfig::default(), |builder| {
        builder.post_create(hook)
    })
    .await
    .expect_err("creation hook must leave an idle connection");
    assert!(matches!(error, Error::Custom(_)));
    assert!(error.to_string().contains("open transaction"));
}

// [spec:pgorm:req:turso.pool/test]
#[tokio::test]
async fn retained_hook_stream_refuses_without_waiting() {
    let retained = Arc::new(Mutex::new(None));
    let output = Arc::clone(&retained);
    let hook = Hook::async_fn(move |client, _| {
        let output = Arc::clone(&output);
        Box::pin(async move {
            let mut rows = Box::pin(
                client
                    .query_raw(MANY, std::iter::empty::<i32>())
                    .await
                    .map_err(HookError::Backend)?,
            );
            assert!(rows.try_next().await.map_err(HookError::Backend)?.is_some());
            *output.lock().expect("retain the hook's active stream") = Some(rows);
            Ok(())
        })
    });
    let error = timeout(
        LIMIT,
        connect_turso_with(":memory:", ManagerConfig::default(), |builder| {
            builder.post_create(hook)
        }),
    )
    .await
    .expect("eager health must not queue behind a retained hook stream")
    .expect_err("unfinished hook work must refuse pool construction");
    assert!(matches!(error, Error::Custom(_)));
    assert!(error.to_string().contains("unfinished work"));
    drop(retained.lock().expect("release the retained stream").take());
}
