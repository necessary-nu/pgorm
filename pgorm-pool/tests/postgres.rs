use std::{
    collections::HashMap,
    env,
    num::NonZeroUsize,
    sync::{
        Arc, Once,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures::{TryStreamExt, future, pin_mut};
use pgorm_driver::{
    IsolationLevel,
    error::SqlState,
    types::{ToSql, Type},
};
use serde::{Deserialize, Serialize};

use pgorm_pool::{
    Client, ClientWrapper, GenericClient, ManagerConfig, Object, Pool, PoolConfig, RecyclingMethod,
    StatementCacheSize,
};

static DOTENV: Once = Once::new();

fn base_url() -> String {
    DOTENV.call_once(|| {
        dotenvy::from_filename(".env.local").ok();
        dotenvy::from_filename(".env").ok();
    });
    env::var("DATABASE_URL").expect(
        "environment variable 'DATABASE_URL' not set; it must hold a PostgreSQL server URL \
         without a database path, e.g. postgres://postgres:postgres@127.0.0.1:5432",
    )
}

fn test_config() -> pgorm_pool::Config {
    pgorm_pool::Config {
        url: Some(format!("{}/postgres", base_url())),
        ..Default::default()
    }
}

fn create_pool() -> Pool {
    test_config().create_pool(pgorm_driver::NoTls).unwrap()
}

/// A pool capped at a single connection, so every checkout after the first is
/// the same physical connection put through `Manager::recycle`.
fn single_connection_pool(recycling_method: RecyclingMethod) -> Pool {
    let mut cfg = test_config();
    cfg.manager = Some(ManagerConfig {
        recycling_method,
        tag: Default::default(),
        statement_cache: StatementCacheSize::default(),
        statement_cache_channel: None,
    });
    cfg.pool = Some(PoolConfig::new(1));
    cfg.create_pool(pgorm_driver::NoTls).unwrap()
}

/// A pool whose connections cache statements under `statement_cache`.
fn cache_size_pool(statement_cache: StatementCacheSize) -> Pool {
    let mut cfg = test_config();
    cfg.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
        tag: Default::default(),
        statement_cache,
        statement_cache_channel: None,
    });
    cfg.create_pool(pgorm_driver::NoTls).unwrap()
}

/// How many server-side prepared statements the current database session
/// parsed from `sql`. Counts `Parse` messages that are still live, so a cache hit shows up
/// as one statement where a re-prepare shows up as two.
async fn prepared_count(client: &Client, sql: &str) -> i64 {
    client
        .query_one(
            "SELECT count(*) FROM pg_prepared_statements WHERE statement = $1",
            &[&sql],
        )
        .await
        .unwrap()
        .get(0)
}

async fn session_setting(client: &Client, name: &str) -> String {
    client
        .query_one("SELECT current_setting($1)", &[&name])
        .await
        .unwrap()
        .get(0)
}

// [spec:pgorm:sem:conn.pool.statement-cache+6/test]    prepare_cached inserts on a miss and the statement is usable
#[tokio::test]
async fn basic() {
    let pool = create_pool();
    let client = pool.get().await.unwrap();
    let stmt = client.prepare_cached("SELECT 1 + 2").await.unwrap();
    let rows = client.query(&stmt, &[]).await.unwrap();
    let value: i32 = rows[0].get(0);
    assert_eq!(value, 3);
    assert_eq!(client.statement_cache.size(), 1);
}

// [spec:pgorm:sem:conn.pool.statement-cache+6/test]    prepare_typed_cached binds the declared parameter types
#[tokio::test]
async fn prepare_typed_cached() {
    let pool = create_pool();
    let client = pool.get().await.unwrap();
    let stmt = client
        .prepare_typed_cached("SELECT 1 + $1", &[Type::INT2])
        .await
        .unwrap();
    let rows = client.query(&stmt, &[&42i16]).await.unwrap();
    let value: i32 = rows[0].get(0);
    assert_eq!(value, 43i32);
}

#[tokio::test]
async fn prepare_typed_error() {
    let pool = create_pool();
    let client = pool.get().await.unwrap();
    let stmt = client
        .prepare_typed_cached("SELECT 1 + $1", &[Type::INT2])
        .await
        .unwrap();
    assert!(client.query(&stmt, &[&42i32]).await.is_err());
}

// [spec:pgorm:sem:conn.pool.statement-cache+6/test]    a statement cached inside a transaction lands in the client's cache
#[tokio::test]
async fn transaction_1() {
    let pool = create_pool();
    let mut client = pool.get().await.unwrap();
    {
        let txn = client.transaction().await.unwrap();
        let stmt = txn.prepare_cached("SELECT 1 + 2").await.unwrap();
        let rows = txn.query(&stmt, &[]).await.unwrap();
        let value: i32 = rows[0].get(0);
        txn.commit().await.unwrap();
        assert_eq!(value, 3);
    }
    assert_eq!(client.statement_cache.size(), 1);
}

// [spec:pgorm:sem:conn.pool.statement-cache+6/test]    a statement cached on the client is reusable inside its transaction
#[tokio::test]
async fn transaction_2() {
    let pool = create_pool();
    let mut client = pool.get().await.unwrap();
    let stmt = client.prepare_cached("SELECT 1 + 2").await.unwrap();
    {
        let txn = client.transaction().await.unwrap();
        let rows = txn.query(&stmt, &[]).await.unwrap();
        let value: i32 = rows[0].get(0);
        txn.commit().await.unwrap();
        assert_eq!(value, 3);
    }
    assert_eq!(client.statement_cache.size(), 1);
}

#[tokio::test]
async fn transaction_pipeline() {
    let pool = create_pool();
    let mut client = pool.get().await.unwrap();
    let stmt = client.prepare_cached("SELECT 1 + $1").await.unwrap();
    let txn = client.transaction().await.unwrap();
    let mut futures = vec![];
    for i in 0..100i32 {
        let stmt = stmt.clone();
        let txn = &txn;
        futures.push(async move {
            let rows = txn.query(&stmt, &[&i]).await.unwrap();
            let value: i32 = rows[0].get(0);
            value
        });
    }
    let results = future::join_all(futures).await;
    for (i, &result) in results.iter().enumerate() {
        assert_eq!(result, (i as i32) + 1);
    }
}

#[tokio::test]
async fn transaction_builder() {
    let pool = create_pool();
    let mut client = pool.get().await.unwrap();
    let txn = client
        .build_transaction()
        .isolation_level(IsolationLevel::ReadUncommitted)
        .read_only(true)
        .deferrable(true)
        .start()
        .await
        .unwrap();
    let rows = txn.query("SELECT 1 + 2", &[]).await.unwrap();
    let value: i32 = rows[0].get(0);
    assert_eq!(value, 3);
    txn.commit().await.unwrap();
}

#[tokio::test]
async fn generic_client() {
    let pool = create_pool();
    let client = pool.get().await.unwrap();
    _use_generic_client(&**client);
}

// [spec:pgorm:sem:conn.pool.recycle+2/test]    every recycling method keeps handing out usable connections
#[tokio::test]
async fn recycling_methods() {
    let recycling_methods = vec![
        RecyclingMethod::Fast,
        RecyclingMethod::Verified,
        RecyclingMethod::Clean,
        RecyclingMethod::Custom("DISCARD ALL;".to_string()),
    ];
    let mut cfg = test_config();
    for recycling_method in recycling_methods {
        cfg.manager = Some(ManagerConfig {
            recycling_method,
            tag: Default::default(),
            statement_cache: StatementCacheSize::default(),
            statement_cache_channel: None,
        });
        let pool = cfg.create_pool(pgorm_driver::NoTls).unwrap();
        for _ in 0usize..20usize {
            let client = pool.get().await.unwrap();
            let rows = client.query("SELECT 1 + 2", &[]).await.unwrap();
            let value: i32 = rows[0].get(0);
            assert_eq!(value, 3);
        }
    }
}

fn _use_generic_client(_client: &impl pgorm_driver::GenericClient) {
    // nop
}

// [spec:pgorm:sem:conn.pool.statement-cache+6/test]    per-connection clear()
#[tokio::test]
async fn statement_cache_clear() {
    let pool = create_pool();
    let client = pool.get().await.unwrap();
    assert!(client.statement_cache.size() == 0);
    client.prepare_cached("SELECT 1;").await.unwrap();
    assert!(client.statement_cache.size() == 1);
    client.statement_cache.clear();
    assert!(client.statement_cache.size() == 0);
}

// [spec:pgorm:sem:conn.pool.statement-cache+6/test]    the manager-level registry clears every live cache
// [spec:pgorm:sem:conn.pool.lifecycle+5/test]    each created connection gets a fresh cache, registered with the manager
#[tokio::test]
async fn statement_caches_clear() {
    let pool = create_pool();
    // prepare 1st client
    let client0 = pool.get().await.unwrap();
    assert!(client0.statement_cache.size() == 0);
    client0.prepare_cached("SELECT 1;").await.unwrap();
    assert!(client0.statement_cache.size() == 1);
    // prepare 2nd client
    let client1 = pool.get().await.unwrap();
    assert!(client1.statement_cache.size() == 0);
    client1.prepare_cached("SELECT 1;").await.unwrap();
    assert!(client1.statement_cache.size() == 1);
    // clear statement cache using manager
    pool.manager().statement_caches.clear();
    assert!(client0.statement_cache.size() == 0);
    assert!(client1.statement_cache.size() == 0);
}

// [spec:pgorm:req:conn.pool.statement-cache.clear+2/test]    an idle connection is cleared, on the client
// and the server, before it is next lent
#[tokio::test]
async fn clear_before_reuse_clears_an_idle_connection() {
    const PROBE: &str = "SELECT 'cleared while idle'";
    let pool = single_connection_pool(RecyclingMethod::Fast);

    let client = pool.get().await.unwrap();
    drop(client.prepare_cached(PROBE).await.unwrap());
    assert_eq!(prepared_count(&client, PROBE).await, 1);
    drop(client);

    pool.manager().statement_caches.clear_before_reuse();

    let client = pool.get().await.unwrap();
    assert_eq!(client.statement_cache.size(), 0);
    assert_eq!(
        prepared_count(&client, PROBE).await,
        0,
        "the cleared statement is closed on the server, not only forgotten"
    );
}

// [spec:pgorm:req:conn.pool.statement-cache.clear+2/test]    a connection checked out at the clear keeps
// its statements until it is returned, and is cleared before it is lent again
#[tokio::test]
async fn clear_before_reuse_waits_for_a_return() {
    const PROBE: &str = "SELECT 'cleared once returned'";
    let pool = single_connection_pool(RecyclingMethod::Fast);

    let client = pool.get().await.unwrap();
    let statement = client.prepare_cached(PROBE).await.unwrap();
    pool.manager().statement_caches.clear_before_reuse();

    assert_eq!(client.statement_cache.size(), 1);
    let rows = client.query(&statement, &[]).await.unwrap();
    assert_eq!(rows[0].get::<_, &str>(0), "cleared once returned");
    drop((rows, statement, client));

    let client = pool.get().await.unwrap();
    assert_eq!(client.statement_cache.size(), 0);
    assert_eq!(prepared_count(&client, PROBE).await, 0);
}

// [spec:pgorm:req:conn.pool.statement-cache.clear+2/test]    a connection created after a clear is not
// cleared by it, and one several clears behind is cleared once
#[tokio::test]
async fn clear_before_reuse_clears_each_connection_once() {
    let pool = single_connection_pool(RecyclingMethod::Fast);
    let caches = &pool.manager().statement_caches;
    caches.clear_before_reuse();

    let client = pool.get().await.unwrap();
    client.prepare_cached("SELECT 1;").await.unwrap();
    drop(client);
    let client = pool.get().await.unwrap();
    assert_eq!(
        client.statement_cache.size(),
        1,
        "created after the clear, the connection is already current with it"
    );

    caches.clear_before_reuse();
    caches.clear_before_reuse();
    drop(client);
    let client = pool.get().await.unwrap();
    assert_eq!(client.statement_cache.size(), 0);
    client.prepare_cached("SELECT 1;").await.unwrap();
    drop(client);
    let client = pool.get().await.unwrap();
    assert_eq!(
        client.statement_cache.size(),
        1,
        "two clears behind, the connection was cleared once and is current"
    );
}

// [spec:pgorm:sem:conn.pool.lifecycle+5/test]    dropping a ClientWrapper aborts its connection task
#[tokio::test]
async fn client_wrapper_drop_aborts_conn_task() {
    struct AbortWitness(Arc<AtomicBool>);

    impl Drop for AbortWitness {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let (client, connection) =
        pgorm_driver::connect(&format!("{}/postgres", base_url()), pgorm_driver::NoTls)
            .await
            .unwrap();
    let _driver = tokio::spawn(async move {
        let _ = connection.await;
    });

    let aborted = Arc::new(AtomicBool::new(false));
    let witness = AbortWitness(aborted.clone());
    // Stands in for the connection driver task. It never resolves on its own,
    // so the witness can only fire because the task was aborted.
    let conn_task = tokio::spawn(async move {
        let _witness = witness;
        std::future::pending::<()>().await;
    });

    let wrapper = ClientWrapper::new(client, conn_task);
    let rows = wrapper.query("SELECT 1 + 2", &[]).await.unwrap();
    assert_eq!(rows[0].get::<_, i32>(0), 3);
    assert!(!aborted.load(Ordering::SeqCst));

    drop(wrapper);

    // `JoinHandle::abort` only schedules the cancellation; the runtime drops
    // the task's future afterwards.
    for _ in 0..200 {
        if aborted.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        aborted.load(Ordering::SeqCst),
        "dropping a ClientWrapper must abort its connection task"
    );
}

// [spec:pgorm:sem:conn.pool.lifecycle+5/test]    recycle refuses a client whose is_closed() is true
#[tokio::test]
async fn recycle_rejects_closed_connection() {
    let pool = single_connection_pool(RecyclingMethod::Fast);

    let client = pool.get().await.unwrap();
    let pid: i32 = client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);

    // Kill the pooled backend from an unrelated pool. The pid is matched with
    // the database as well, so nothing but the session this test holds can be
    // the one terminated.
    let executioner = create_pool();
    let executioner = executioner.get().await.unwrap();
    let terminated = executioner
        .query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
             WHERE pid = $1 AND datname = current_database()",
            &[&pid],
        )
        .await
        .unwrap();
    assert_eq!(terminated.len(), 1);

    for _ in 0..200 {
        if client.is_closed() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        client.is_closed(),
        "the terminated backend must close the client"
    );

    // Hand the dead connection back to the pool it came from.
    drop(client);

    let client = pool.get().await.unwrap();
    let new_pid: i32 = client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    assert_ne!(
        new_pid, pid,
        "recycle must reject the closed client and the pool create a fresh one"
    );
}

// [spec:pgorm:sem:conn.pool.lifecycle+5/test]    detach unregisters the connection's statement cache
#[tokio::test]
async fn detach_unregisters_statement_cache() {
    let pool = create_pool();

    let pooled = pool.get().await.unwrap();
    pooled.prepare_cached("SELECT 1;").await.unwrap();
    assert_eq!(pooled.statement_cache.size(), 1);

    // Taking an object out of the pool runs `Manager::detach`.
    let detached = Object::take(pooled);
    assert_eq!(detached.statement_cache.size(), 1);

    let registered = pool.get().await.unwrap();
    registered.prepare_cached("SELECT 1;").await.unwrap();
    assert_eq!(registered.statement_cache.size(), 1);

    pool.manager().statement_caches.clear();

    assert_eq!(
        registered.statement_cache.size(),
        0,
        "a still-pooled connection's cache is reachable from the registry"
    );
    assert_eq!(
        detached.statement_cache.size(),
        1,
        "a detached connection's cache is no longer in the registry"
    );
}

// [spec:pgorm:sem:conn.pool.recycle+2/test]    Fast runs no query, so session state survives recycling
#[tokio::test]
async fn recycle_fast_keeps_session_state() {
    let pool = single_connection_pool(RecyclingMethod::Fast);

    let client = pool.get().await.unwrap();
    client
        .batch_execute("SET application_name = 'pgorm-recycle-fast'")
        .await
        .unwrap();
    drop(client);

    let client = pool.get().await.unwrap();
    assert_eq!(
        session_setting(&client, "application_name").await,
        "pgorm-recycle-fast",
        "Fast only checks is_closed(); it runs no cleanup query"
    );
}

// [spec:pgorm:sem:conn.pool.recycle+2/test]    Clean's RESET ALL wipes session state on reuse
#[tokio::test]
async fn recycle_clean_resets_session_state() {
    let pool = single_connection_pool(RecyclingMethod::Clean);

    let client = pool.get().await.unwrap();
    client
        .batch_execute("SET application_name = 'pgorm-recycle-clean'")
        .await
        .unwrap();
    assert_eq!(
        session_setting(&client, "application_name").await,
        "pgorm-recycle-clean"
    );
    drop(client);

    let client = pool.get().await.unwrap();
    assert_ne!(
        session_setting(&client, "application_name").await,
        "pgorm-recycle-clean",
        "Clean runs RESET ALL as part of its recycling sequence"
    );
}

// [spec:pgorm:sem:conn.pool.recycle+2/test]    Clean omits DEALLOCATE ALL, so cached statements survive
#[tokio::test]
async fn recycle_clean_keeps_cached_statements() {
    let pool = single_connection_pool(RecyclingMethod::Clean);

    let client = pool.get().await.unwrap();
    let stmt = client.prepare_cached("SELECT 1 + 2").await.unwrap();
    drop(client);

    let client = pool.get().await.unwrap();
    assert_eq!(client.statement_cache.size(), 1);
    let rows = client.query(&stmt, &[]).await.unwrap();
    assert_eq!(
        rows[0].get::<_, i32>(0),
        3,
        "the server-side statement must outlive Clean's recycling sequence"
    );
}

// [spec:pgorm:sem:conn.pool.recycle+2/test]    Custom runs the caller's SQL, here one that does deallocate
#[tokio::test]
async fn recycle_custom_discard_all_deallocates() {
    let pool = single_connection_pool(RecyclingMethod::Custom("DISCARD ALL;".to_string()));

    let client = pool.get().await.unwrap();
    let stmt = client.prepare_cached("SELECT 1 + 2").await.unwrap();
    drop(client);

    let client = pool.get().await.unwrap();
    assert_eq!(
        client.statement_cache.size(),
        1,
        "recycling never touches the client-side cache"
    );
    let err = client
        .query(&stmt, &[])
        .await
        .expect_err("DISCARD ALL deallocates the server-side statement");
    assert_eq!(
        err.code(),
        Some(&SqlState::INVALID_SQL_STATEMENT_NAME),
        "this is the deallocation Clean deliberately avoids"
    );
}

// [spec:pgorm:sem:conn.pool.statement-cache+6/test]    the cache key is (query text, parameter types)
#[tokio::test]
async fn statement_cache_keys_include_param_types() {
    const SQL: &str = "SELECT $1::int8";

    let pool = create_pool();
    let client = pool.get().await.unwrap();

    let untyped = client.prepare_cached(SQL).await.unwrap();
    assert_eq!(client.statement_cache.size(), 1);

    let typed = client
        .prepare_typed_cached(SQL, &[Type::INT8])
        .await
        .unwrap();
    assert_eq!(
        client.statement_cache.size(),
        2,
        "the same query text under different parameter types is a different key"
    );

    assert_eq!(
        client
            .query_one(&untyped, &[&7i64])
            .await
            .unwrap()
            .get::<_, i64>(0),
        7
    );
    assert_eq!(
        client
            .query_one(&typed, &[&8i64])
            .await
            .unwrap()
            .get::<_, i64>(0),
        8
    );

    assert!(client.statement_cache.remove(SQL, &[]).is_some());
    assert_eq!(client.statement_cache.size(), 1);
    assert!(client.statement_cache.remove(SQL, &[Type::INT8]).is_some());
    assert_eq!(client.statement_cache.size(), 0);
}

// [spec:pgorm:req:conn.pool.statement-cache.bound+2/test]    the default bound
#[test]
fn statement_cache_is_bounded_by_default() {
    assert_eq!(
        StatementCacheSize::default(),
        StatementCacheSize::Bounded(NonZeroUsize::new(256).expect("256 is not zero"))
    );
}

// [spec:pgorm:req:conn.pool.statement-cache.bound+2/test]    a full cache evicts to make room
#[tokio::test]
async fn bounded_cache_evicts_to_make_room() {
    let pool = cache_size_pool(StatementCacheSize::Bounded(
        NonZeroUsize::new(2).expect("2 is not zero"),
    ));
    let client = pool.get().await.unwrap();

    for arity in 1..=4usize {
        let placeholders = (1..=arity)
            .map(|n| format!("${n}::int8"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = client
            .prepare_cached(&format!("SELECT {placeholders}"))
            .await
            .unwrap();
    }

    assert_eq!(
        client.statement_cache.size(),
        2,
        "four texts through a cache of two leaves two"
    );
}

// [spec:pgorm:req:conn.pool.statement-cache.bound+2/test]    Disabled prepares afresh every time
#[tokio::test]
async fn disabled_cache_stores_nothing() {
    const SQL: &str = "SELECT 'uncacheable'";

    let pool = cache_size_pool(StatementCacheSize::Disabled);
    let client = pool.get().await.unwrap();

    let first = client.prepare_cached(SQL).await.unwrap();
    let second = client.prepare_cached(SQL).await.unwrap();

    assert_eq!(client.statement_cache.size(), 0);
    assert_eq!(
        prepared_count(&client, SQL).await,
        2,
        "each prepare_cached parsed its own statement"
    );

    drop((first, second));
}

// [spec:pgorm:sem:conn.pool.statement-cache+6/test]    a hit returns the cached statement without preparing again
#[tokio::test]
async fn statement_cache_hit_avoids_reparse() {
    const CACHED: &str = "SELECT 'cached-parse'";
    const UNCACHED: &str = "SELECT 'uncached-parse'";

    let pool = create_pool();
    let client = pool.get().await.unwrap();

    let first = client.prepare_cached(CACHED).await.unwrap();
    let second = client.prepare_cached(CACHED).await.unwrap();
    assert_eq!(client.statement_cache.size(), 1);
    assert_eq!(
        prepared_count(&client, CACHED).await,
        1,
        "the second prepare_cached is a hit, not a second Parse"
    );

    // The uncached path is the control: every prepare parses again.
    let third = client.prepare(UNCACHED).await.unwrap();
    let fourth = client.prepare(UNCACHED).await.unwrap();
    assert_eq!(prepared_count(&client, UNCACHED).await, 2);
    assert_eq!(client.statement_cache.size(), 1);

    drop((first, second, third, fourth));
}

// [spec:pgorm:sem:conn.pool.statement-cache+6/test]    the registry removes one statement across every live cache
#[tokio::test]
async fn statement_caches_remove_one_statement() {
    let pool = create_pool();
    let client0 = pool.get().await.unwrap();
    let client1 = pool.get().await.unwrap();

    for client in [&client0, &client1] {
        client.prepare_cached("SELECT 1;").await.unwrap();
        client.prepare_cached("SELECT 2;").await.unwrap();
        assert_eq!(client.statement_cache.size(), 2);
    }

    pool.manager().statement_caches.remove("SELECT 1;", &[]);

    assert_eq!(client0.statement_cache.size(), 1);
    assert_eq!(client1.statement_cache.size(), 1);
    assert!(client0.statement_cache.remove("SELECT 1;", &[]).is_none());
    assert!(client0.statement_cache.remove("SELECT 2;", &[]).is_some());
}

// [spec:pgorm:sem:conn.pool.statement-cache+6/test]    tokio-postgres's own statement paths never consult it
#[tokio::test]
async fn plain_query_bypasses_statement_cache() {
    let pool = create_pool();
    let client = pool.get().await.unwrap();

    for _ in 0..3 {
        let rows = client.query("SELECT 1 + 2", &[]).await.unwrap();
        assert_eq!(rows[0].get::<_, i32>(0), 3);
    }
    let _ = client.execute("SELECT 1 + 2", &[]).await.unwrap();
    let _ = client.query_one("SELECT 1 + 2", &[]).await.unwrap();
    client.batch_execute("SELECT 1 + 2").await.unwrap();

    assert_eq!(
        client.statement_cache.size(),
        0,
        "only prepare_cached / prepare_typed_cached populate the cache"
    );
}

// [spec:pgorm:sem:conn.pool.statement-cache+6/test]    nested transactions and savepoints share the owning client's cache
#[tokio::test]
async fn savepoint_shares_client_statement_cache() {
    let pool = create_pool();
    let mut client = pool.get().await.unwrap();

    {
        let mut txn = client.transaction().await.unwrap();
        txn.prepare_cached("SELECT 1;").await.unwrap();

        {
            let mut nested = txn.transaction().await.unwrap();
            nested.prepare_cached("SELECT 2;").await.unwrap();

            {
                let savepoint = nested.savepoint("shared_cache_probe").await.unwrap();
                savepoint.prepare_cached("SELECT 3;").await.unwrap();
                assert_eq!(savepoint.statement_cache.size(), 3);
            }

            assert_eq!(nested.statement_cache.size(), 3);
        }

        txn.commit().await.unwrap();
    }

    assert_eq!(
        client.statement_cache.size(),
        3,
        "every nesting level writes through to the connection's one cache"
    );
}

/// Savepoint names carrying every character that could end the identifier and
/// start a statement of its own. Each is used as a savepoint name and must
/// come back as nothing but a name.
const HOSTILE_SAVEPOINT_NAMES: &[&str] = &[
    r#"x"; DROP TABLE savepoint_injection_probe; --"#,
    r#"x"; DROP TABLE savepoint_injection_probe; /*"#,
    "x; DROP TABLE savepoint_injection_probe",
    // A bare delimiter, which doubling is the whole answer to.
    r#"""#,
    // Case and whitespace a folded identifier would not preserve.
    "Mixed Case Point",
];

// [spec:pgorm:req:conn.pool.savepoint-name+4/test]    a savepoint name is an
// identifier, not SQL: a name holding a statement terminator names a savepoint
// that releases and rolls back normally, and the table it tried to drop lives
#[tokio::test]
async fn savepoint_name_cannot_smuggle_sql() {
    let pool = create_pool();
    let mut client = pool.get().await.unwrap();

    // Temporary, because these tests share the `postgres` database with every
    // other run on the server: a table of the session's own cannot be seen,
    // filled or dropped by a concurrent run of this test.
    client
        .batch_execute("CREATE TEMPORARY TABLE savepoint_injection_probe (id int)")
        .await
        .unwrap();

    {
        let mut txn = client.transaction().await.unwrap();

        for name in HOSTILE_SAVEPOINT_NAMES {
            // Released: the savepoint's own work survives into the outer
            // transaction, so the name round-tripped through SAVEPOINT and
            // RELEASE as one identifier.
            {
                let released = txn.savepoint(*name).await.unwrap();
                let _ = released
                    .execute("INSERT INTO savepoint_injection_probe VALUES (1)", &[])
                    .await
                    .unwrap();
                released.commit().await.unwrap();
            }

            // Rolled back: ROLLBACK TO reaches the same savepoint, so only
            // this insert is undone.
            {
                let rolled_back = txn.savepoint(*name).await.unwrap();
                let _ = rolled_back
                    .execute("INSERT INTO savepoint_injection_probe VALUES (2)", &[])
                    .await
                    .unwrap();
                rolled_back.rollback().await.unwrap();
            }

            // The table the name tried to drop is still there, and holds
            // exactly the released insert.
            let ones: i64 = txn
                .query_one(
                    "SELECT count(*) FROM savepoint_injection_probe WHERE id = 1",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(ones, 1, "{name:?} did not round-trip as one savepoint");

            let twos: i64 = txn
                .query_one(
                    "SELECT count(*) FROM savepoint_injection_probe WHERE id = 2",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(twos, 0, "{name:?} left a rolled-back row behind");

            let _ = txn
                .execute("DELETE FROM savepoint_injection_probe", &[])
                .await
                .unwrap();
        }

        txn.commit().await.unwrap();
    }

    // Outside the transaction too: nothing any of those names said was ever
    // executed as a statement.
    let surviving: i64 = client
        .query_one("SELECT count(*) FROM savepoint_injection_probe", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(surviving, 0);

    client
        .batch_execute("DROP TABLE savepoint_injection_probe")
        .await
        .unwrap();
}

// [spec:pgorm:req:conn.pool.savepoint-name+4/test]    a NUL byte is refused
// client-side, and the transaction it was asked of is still usable
#[tokio::test]
async fn savepoint_name_with_a_nul_is_refused() {
    let pool = create_pool();
    let mut client = pool.get().await.unwrap();
    let mut txn = client.transaction().await.unwrap();

    let refused = txn.savepoint("point\0name").await;
    assert!(
        refused.is_err(),
        "a NUL byte in a savepoint name was not refused"
    );
    drop(refused);

    // Refused before anything was sent, so the transaction is untouched.
    let answer: i32 = txn.query_one("SELECT 1", &[]).await.unwrap().get(0);
    assert_eq!(answer, 1);

    {
        let nested = txn.savepoint("point_name").await.unwrap();
        let answer: i32 = nested.query_one("SELECT 2", &[]).await.unwrap().get(0);
        assert_eq!(answer, 2);
    }

    txn.commit().await.unwrap();
}

/// Exercises the whole `GenericClient` surface that delegates to
/// `pgorm_driver`, so it can be run against both implementors.
async fn exercise_generic_client<C>(client: &C)
where
    C: GenericClient,
{
    const SQL: &str = "SELECT $1::int4 + 1";

    let prepared = client.prepare(SQL).await.unwrap();
    assert_eq!(
        client
            .query_one(&prepared, &[&1i32])
            .await
            .unwrap()
            .get::<_, i32>(0),
        2
    );

    let typed = client.prepare_typed(SQL, &[Type::INT4]).await.unwrap();
    assert_eq!(client.query(&typed, &[&2i32]).await.unwrap().len(), 1);

    let cached = client.prepare_cached(SQL).await.unwrap();
    assert_eq!(
        client
            .query_opt(&cached, &[&3i32])
            .await
            .unwrap()
            .unwrap()
            .get::<_, i32>(0),
        4
    );

    let typed_cached = client
        .prepare_typed_cached(SQL, &[Type::INT4])
        .await
        .unwrap();
    assert_eq!(client.execute(&typed_cached, &[&4i32]).await.unwrap(), 1);

    let params: [&(dyn ToSql + Sync); 1] = [&5i32];
    assert_eq!(client.execute_raw(&prepared, params).await.unwrap(), 1);

    let params: [&(dyn ToSql + Sync); 1] = [&6i32];
    let stream = client.query_raw(&prepared, params).await.unwrap();
    pin_mut!(stream);
    assert_eq!(
        stream.try_next().await.unwrap().unwrap().get::<_, i32>(0),
        7
    );
    assert!(stream.try_next().await.unwrap().is_none());

    // The only multi-statement method: it goes through the simple protocol.
    client.batch_execute("SELECT 1; SELECT 2;").await.unwrap();
}

// [spec:pgorm:def:conn.pool.generic-client+1/test]    one statement surface over both implementors, Client and Transaction
#[tokio::test]
async fn generic_client_covers_client_and_txn() {
    let pool = create_pool();
    let mut client = pool.get().await.unwrap();

    exercise_generic_client(&client).await;

    let mut txn = GenericClient::transaction(&mut client).await.unwrap();
    exercise_generic_client(&txn).await;

    txn.batch_execute("CREATE TEMP TABLE generic_client_probe (id int)")
        .await
        .unwrap();

    {
        let mut nested = GenericClient::transaction(&mut txn).await.unwrap();
        exercise_generic_client(&nested).await;
        let _ = nested
            .execute("INSERT INTO generic_client_probe VALUES (1)", &[])
            .await
            .unwrap();

        {
            let deeper = GenericClient::transaction(&mut nested).await.unwrap();
            let _ = deeper
                .execute("INSERT INTO generic_client_probe VALUES (2)", &[])
                .await
                .unwrap();
        }
    }

    // A nested transaction is a savepoint, so dropping it rolls back only its
    // own work and leaves the outer transaction usable.
    let remaining: i64 = txn
        .query_one("SELECT count(*) FROM generic_client_probe", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(remaining, 0);

    txn.commit().await.unwrap();
}

// [spec:pgorm:def:conn.pool.generic-client+1/test]    cached prepares route through the wrapper types onto one shared cache
#[tokio::test]
async fn generic_client_cached_prepare_shares_cache() {
    let pool = create_pool();
    let mut client = pool.get().await.unwrap();

    GenericClient::prepare_cached(&client, "SELECT 1;")
        .await
        .unwrap();
    assert_eq!(client.statement_cache.size(), 1);

    {
        let txn = GenericClient::transaction(&mut client).await.unwrap();
        GenericClient::prepare_cached(&txn, "SELECT 2;")
            .await
            .unwrap();
        GenericClient::prepare_typed_cached(&txn, "SELECT $1::int4", &[Type::INT4])
            .await
            .unwrap();
        assert_eq!(txn.statement_cache.size(), 3);
        txn.commit().await.unwrap();
    }

    assert_eq!(
        client.statement_cache.size(),
        3,
        "the transaction writes into the owning client's cache, not its own"
    );
}

#[derive(Debug, Deserialize, Serialize)]
struct EnvConfig {
    #[serde(default)]
    pg: pgorm_pool::Config,
}

impl EnvConfig {
    pub fn from_env_with_prefix(prefix: &str) -> Self {
        let cfg = config::Config::builder()
            .add_source(config::Environment::with_prefix(prefix).separator("__"))
            .build()
            .unwrap();
        cfg.try_deserialize::<Self>().unwrap()
    }
}

struct Env {
    backup: HashMap<String, Option<String>>,
}

impl Env {
    pub fn new() -> Self {
        Self {
            backup: HashMap::new(),
        }
    }
    pub fn set(&mut self, name: &str, value: &str) {
        self.backup.insert(name.to_string(), env::var(name).ok());
        unsafe { env::set_var(name, value) };
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for (name, value) in self.backup.iter() {
            println!("setting {} = {:?}", name, value);
            match value {
                Some(val) => unsafe { env::set_var(name.as_str(), val) },
                None => unsafe { env::remove_var(name.as_str()) },
            }
        }
    }
}

#[test]
fn config_from_env() {
    // This test must not use "PG" as prefix as this can cause the other
    // tests which also use the "PG" prefix to fail.
    let mut env = Env::new();
    env.set("ENV_TEST__PG__HOST", "pg.example.com");
    env.set("ENV_TEST__PG__PORT", "5433");
    env.set("ENV_TEST__PG__USER", "john_doe");
    env.set("ENV_TEST__PG__PASSWORD", "topsecret");
    env.set("ENV_TEST__PG__DBNAME", "example");
    env.set("ENV_TEST__PG__POOL__MAX_SIZE", "42");
    env.set("ENV_TEST__PG__POOL__TIMEOUTS__WAIT__SECS", "1");
    env.set("ENV_TEST__PG__POOL__TIMEOUTS__WAIT__NANOS", "0");
    env.set("ENV_TEST__PG__POOL__TIMEOUTS__CREATE__SECS", "2");
    env.set("ENV_TEST__PG__POOL__TIMEOUTS__CREATE__NANOS", "0");
    env.set("ENV_TEST__PG__POOL__TIMEOUTS__RECYCLE__SECS", "3");
    env.set("ENV_TEST__PG__POOL__TIMEOUTS__RECYCLE__NANOS", "0");
    let cfg = EnvConfig::from_env_with_prefix("ENV_TEST");
    // `pgorm_driver::Config` does not provide any read access to its
    // internals, so we can only check if the environment was actually read
    // correctly.
    assert_eq!(cfg.pg.host, Some("pg.example.com".to_string()));
    assert_eq!(cfg.pg.port, Some(5433));
    assert_eq!(cfg.pg.user, Some("john_doe".to_string()));
    assert_eq!(cfg.pg.password, Some("topsecret".to_string()));
    assert_eq!(cfg.pg.dbname, Some("example".to_string()));
    let pool_cfg = cfg.pg.get_pool_config();
    assert_eq!(pool_cfg.max_size, 42);
    assert_eq!(pool_cfg.timeouts.wait, Some(Duration::from_secs(1)));
    assert_eq!(pool_cfg.timeouts.create, Some(Duration::from_secs(2)));
    assert_eq!(pool_cfg.timeouts.recycle, Some(Duration::from_secs(3)));
}

#[test]
fn config_url() {
    let mut cfg = pgorm_pool::Config {
        url: Some("postgresql://zombie@localhost/deadpool".into()),
        ..Default::default()
    };
    {
        let pg_cfg = cfg.get_pg_config().unwrap();
        assert_eq!(pg_cfg.get_dbname(), Some("deadpool"));
        assert_eq!(pg_cfg.get_user(), Some("zombie"));
        assert_eq!(
            pg_cfg.get_hosts(),
            &[pgorm_driver::config::Host::Tcp("localhost".into())]
        );
    }
    // now apply some overrides
    {
        cfg.dbname = Some("livepool".into());
        cfg.host = Some("remotehost".into());
        cfg.user = Some("human".into());
        let pg_cfg = cfg.get_pg_config().unwrap();
        assert_eq!(pg_cfg.get_dbname(), Some("livepool"));
        assert_eq!(pg_cfg.get_user(), Some("human"));
        assert_eq!(
            pg_cfg.get_hosts(),
            &[
                pgorm_driver::config::Host::Tcp("localhost".into()),
                pgorm_driver::config::Host::Tcp("remotehost".into()),
            ]
        );
    }
}
