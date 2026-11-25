//! Public constructors and lifecycle of experimental native pools.
#![cfg(feature = "turso")]

use futures::TryStreamExt;
use pgorm::{
    Connection, DecodeRaw, Error, Hook, HookConnection, HookError, ManagerConfig, PoolBuilder,
    PoolError, RecyclingMethod, Runtime, TimeoutType, Transact, connect, connect_with,
};
use pgorm_driver::{Config, NoTls, Statement, types::Type};
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

// [spec:pgorm:req:turso.pool+1/test]
// [spec:pgorm:req:conn.pool+7/test]
#[tokio::test]
async fn default_constructor_waits_until_first_acquisition() -> Result<(), Error> {
    let scratch = Scratch::new();
    let path = scratch.path().join("lazy.db");
    let config = path.to_str().expect("UTF-8 fixture path").parse()?;
    assert!(!path.exists(), "parsing a path must not open it");
    let pool = connect(config);
    assert_eq!(pool.status().size, 0);
    assert_eq!(pool.status().available, 0);
    assert!(!pool.is_closed());
    assert!(!path.exists(), "pool construction must not open the file");
    let db = pool.get().await?;
    assert!(path.is_file());
    assert_eq!(pool.status().size, 1);
    assert_eq!(db.query_one("SELECT 1", &[]).await?.get::<_, i32>(0), 1);
    Ok(())
}

// [spec:pgorm:req:turso.pool+1/test]
#[tokio::test]
async fn opening_and_hook_failures_reach_first_acquisition() -> Result<(), Error> {
    let scratch = Scratch::new();
    let path = scratch.path().join("missing").join("database.db");
    let pool = connect(path.to_str().expect("UTF-8 fixture path").parse()?);
    assert_eq!(pool.status().size, 0);
    assert!(!path.exists());
    let opening = pool
        .get()
        .await
        .expect_err("first acquisition opens the missing path");
    assert!(matches!(opening, Error::Pool(_)));

    let calls = Arc::new(AtomicUsize::new(0));
    let called = Arc::clone(&calls);
    let hook = Hook::sync_fn(move |_, _| {
        called.fetch_add(1, Ordering::SeqCst);
        Err(HookError::message("creation hook refused"))
    });
    let pool = connect_with(
        ":memory:".parse()?,
        NoTls,
        ManagerConfig::default(),
        |builder| builder.post_create(hook),
    )?;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(pool.status().size, 0);
    let failed = pool
        .get()
        .await
        .expect_err("post-create failure must refuse acquisition");
    assert!(matches!(failed, Error::Pool(_)));
    assert!(failed.to_string().contains("creation hook refused"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    Ok(())
}

// [spec:pgorm:req:turso.pool+1/test]
#[tokio::test]
async fn unsupported_options_are_refused_before_file_creation() -> Result<(), Error> {
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
        let error = connect_with(
            path.to_str().expect("UTF-8 fixture path").parse()?,
            NoTls,
            manager,
            |builder| {
                built = true;
                builder
            },
        )
        .expect_err("unsupported native manager setting must be refused");
        assert!(matches!(error, Error::Custom(_)), "{error:?}");
        assert!(!path.exists());
        assert!(!built, "unsupported settings precede builder side effects");
    }
    Ok(())
}

// [spec:pgorm:req:turso.pool+1/test]
#[tokio::test]
async fn borrowers_share_memory_while_separate_pools_are_isolated() -> Result<(), Error> {
    let config: Config = ":memory:".parse()?;
    let pool = connect(config.clone());
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
    let separate = connect(":memory:".parse()?);
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
    let cloned = connect(config);
    let isolated_clone = cloned.get().await?;
    isolated_clone
        .batch_execute("CREATE TABLE shared_values(id INTEGER)")
        .await?;
    assert_eq!(
        isolated_clone
            .query_one("SELECT count(*) FROM shared_values", &[])
            .await?
            .get::<_, i64>(0),
        0,
        "a cloned configuration must not share another pool's memory database"
    );
    Ok(())
}

// [spec:pgorm:req:turso.pool+1/test]
#[tokio::test]
async fn hooks_reuse_and_clear_typed_cache() -> Result<(), Error> {
    let statements = Arc::new(Mutex::new(Vec::new()));
    let created = cached_hook(Arc::clone(&statements));
    let recycled = cached_hook(Arc::clone(&statements));
    let pool = connect_with(
        ":memory:".parse()?,
        NoTls,
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
    )?;
    assert_eq!(pool.tag().as_str(), "native-reader");
    assert_eq!(pool.status().max_size, 1);
    assert_eq!(
        statements.lock().expect("creation hook has not run").len(),
        0
    );
    let first = pool.get().await?;
    assert_eq!(typed_value(&first, 11).await?, 11);
    drop(first);
    let second = pool.get().await?;
    assert_eq!(typed_value(&second, 12).await?, 12);
    {
        let recorded = statements.lock().expect("read reused metadata");
        assert_eq!(recorded.len(), 2);
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
    assert_eq!(recorded.len(), 3);
    assert!(!std::ptr::eq(
        recorded[0].columns().as_ptr(),
        recorded[2].columns().as_ptr()
    ));
    Ok(())
}

// [spec:pgorm:req:turso.pool+1/test]
#[tokio::test]
async fn configured_wait_timeout_preserves_pool_progress() -> Result<(), Error> {
    let configure: fn(PoolBuilder) -> PoolBuilder = |builder| {
        builder
            .max_size(1)
            .runtime(Runtime::Tokio1)
            .wait_timeout(Some(Duration::from_millis(25)))
    };
    let pool = connect_with(
        ":memory:".parse()?,
        NoTls,
        ManagerConfig::default(),
        configure,
    )?;
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

// [spec:pgorm:req:turso.pool+1/test]
#[tokio::test]
async fn builder_validation_and_zero_capacity_remain_lazy() -> Result<(), Error> {
    let scratch = Scratch::new();
    let path = scratch.path().join("zero.db");
    let zero = connect_with(
        path.to_str().expect("UTF-8 fixture path").parse()?,
        NoTls,
        ManagerConfig::default(),
        |builder| {
            builder
                .max_size(0)
                .runtime(Runtime::Tokio1)
                .wait_timeout(Some(Duration::from_millis(25)))
        },
    )?;
    assert_eq!(zero.status().size, 0);
    assert!(!path.exists());
    let refused = timeout(LIMIT, zero.get())
        .await
        .expect("configured wait timeout must finish");
    assert!(matches!(
        refused,
        Err(Error::Pool(PoolError::Timeout(TimeoutType::Wait)))
    ));
    assert!(!path.exists(), "zero capacity never creates a client");
    let missing_runtime = connect_with(
        ":memory:".parse()?,
        NoTls,
        ManagerConfig::default(),
        |builder| builder.wait_timeout(Some(Duration::from_secs(1))),
    )
    .expect_err("a timeout requires an explicit pool runtime");
    assert!(matches!(missing_runtime, Error::Custom(_)));
    assert!(
        missing_runtime
            .to_string()
            .contains("Timeouts require a runtime")
    );
    Ok(())
}

// [spec:pgorm:req:turso.pool+1/test]
#[tokio::test]
async fn listener_refusal_leaves_the_native_pool_usable() -> Result<(), Error> {
    let pool = connect_with(
        ":memory:".parse()?,
        NoTls,
        ManagerConfig::default(),
        |builder| builder.max_size(1),
    )?;
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

// [spec:pgorm:req:turso.pool+1/test]
#[tokio::test]
async fn pool_close_keeps_a_checked_out_connection_usable() -> Result<(), Error> {
    let pool = connect(":memory:".parse()?);
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

// [spec:pgorm:req:turso.pool+1/test]
#[tokio::test]
async fn committed_file_data_survives_pool_reopen() -> Result<(), Error> {
    let scratch = Scratch::new();
    let path = scratch.path().join("persistent.db");
    let pool = connect(path.to_str().expect("UTF-8 fixture path").parse()?);
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
    let reopened = connect(path.to_str().expect("UTF-8 fixture path").parse()?);
    let db = reopened.get().await?;
    assert_eq!(
        db.query_one("SELECT value FROM durable", &[])
            .await?
            .get::<_, i64>(0),
        9_007_199_254_740_993
    );
    Ok(())
}

// [spec:pgorm:req:turso.pool+1/test]
#[tokio::test]
async fn a_creation_hook_cannot_leave_a_transaction_open() -> Result<(), Error> {
    let hook = Hook::async_fn(|client, _| {
        Box::pin(async move {
            client
                .batch_execute("BEGIN")
                .await
                .map_err(HookError::Backend)
        })
    });
    let pool = connect_with(
        ":memory:".parse()?,
        NoTls,
        ManagerConfig::default(),
        |builder| builder.post_create(hook),
    )?;
    assert_eq!(pool.status().size, 0);
    let error = pool
        .get()
        .await
        .expect_err("creation hook must leave an idle connection");
    assert!(matches!(error, Error::Pool(_)));
    assert!(error.to_string().contains("open transaction"));
    Ok(())
}

// [spec:pgorm:req:turso.pool+1/test]
#[tokio::test]
async fn retained_hook_stream_refuses_without_waiting() -> Result<(), Error> {
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
    let pool = connect_with(
        ":memory:".parse()?,
        NoTls,
        ManagerConfig::default(),
        |builder| builder.post_create(hook),
    )?;
    assert!(retained.lock().expect("hook has not run").is_none());
    let error = timeout(LIMIT, pool.get())
        .await
        .expect("acquisition must not wait behind a retained hook stream")
        .expect_err("unfinished hook work must refuse acquisition");
    assert!(matches!(error, Error::Pool(_)));
    assert!(error.to_string().contains("unfinished work"));
    drop(retained.lock().expect("release the retained stream").take());
    Ok(())
}
