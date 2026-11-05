#![allow(unused_imports, dead_code)]

pub mod common;

use std::{collections::BTreeMap, sync::Arc, time::Duration};

pub use common::{TestContext, bakery_chain::*, setup::*};
use pgorm::entity::prelude::*;
use pgorm::{
    Config, DatabaseMultiPool, DatabasePool, MakeTlsConnect, ManagerConfig, NoTls, PoolBuilder,
    PoolConfig, RecyclingMethod, Socket, StatementCacheSize, TlsConnect,
};
use pretty_assertions::assert_eq;

fn database_url() -> String {
    dotenvy::from_filename(".env.local").ok();
    dotenvy::from_filename(".env").ok();
    std::env::var("DATABASE_URL").expect("Enviroment variable 'DATABASE_URL' not set")
}

/// A pool against the maintenance database, which every server has and no test
/// owns. These tests only read session state, so they need no schema.
fn maintenance_config() -> pgorm::Config {
    common::setup::config(&database_url(), "postgres")
}

async fn session_setting<C>(db: &C, name: &str) -> Result<String, Error>
where
    C: ConnectionTrait,
{
    Ok(db
        .query_one("SELECT current_setting($1)", &[&name])
        .await?
        .get(0))
}

// [spec:pgorm:req:conn.pool+4/test]    connect() builds an untagged, default-sized, Fast-recycling pool
#[pgorm_macros::test]
pub async fn connect_builds_default_fast_pool() -> Result<(), Error> {
    let pool = pgorm::connect(maintenance_config());

    let status = pool.status();
    assert_eq!(
        status.max_size,
        PoolConfig::default().max_size,
        "connect() takes the default deadpool pool configuration"
    );
    assert_eq!(status.size, 0);
    assert_eq!(status.available, 0);

    assert!(
        pool.tag().starts_with("default-"),
        "an untagged manager falls back to a generated tag, got {:?}",
        pool.tag()
    );

    // RecyclingMethod::Fast runs no cleanup query, so session state set on one
    // checkout is still there when the same connection is handed back out.
    let conn = pool.get().await?;
    conn.batch_execute("SET application_name = 'pgorm-connect-fast'")
        .await?;
    drop(conn);

    let conn = pool.get().await?;
    assert_eq!(
        session_setting(&conn, "application_name").await?,
        "pgorm-connect-fast"
    );
    assert_eq!(pool.status().size, 1, "the one connection was reused");

    Ok(())
}

// [spec:pgorm:req:conn.pool+4/test]    connect_with_builder applies the closure before the pool is built
#[pgorm_macros::test]
pub async fn connect_with_builder_applies_closure() -> Result<(), Error> {
    let pool = pgorm::connect_with_builder(maintenance_config(), |builder| builder.max_size(2))?;

    assert_eq!(pool.status().max_size, 2);

    let first = pool.get().await?;
    let second = pool.get().await?;
    assert_eq!(pool.status().available, 0);

    let third = tokio::time::timeout(Duration::from_millis(250), pool.get()).await;
    assert!(
        third.is_err(),
        "a third checkout must block: the closure's max_size is the pool's real cap"
    );

    drop(first);

    let third = tokio::time::timeout(Duration::from_millis(250), pool.get())
        .await
        .expect("a released connection must unblock the waiter")?;
    assert_eq!(third.query_one("SELECT 1", &[]).await?.get::<_, i32>(0), 1);

    drop(second);

    Ok(())
}

// [spec:pgorm:req:conn.pool+4/test]    a builder the caller cannot build yields Error, not a panic
#[pgorm_macros::test]
pub async fn connect_with_builder_errs_on_build_failure() -> Result<(), Error> {
    // deadpool refuses to build a pool with timeouts but no runtime.
    let err = pgorm::connect_with_builder(maintenance_config(), |builder| {
        builder.wait_timeout(Some(Duration::from_secs(1)))
    })
    .expect_err("a wait timeout without a runtime cannot build");

    assert!(matches!(err, Error::Custom(_)), "{err:?}");
    assert!(
        err.to_string().contains("Timeouts require a runtime"),
        "the builder's own message is carried through, got {err}"
    );

    Ok(())
}

/// `connect_with` is generic over the TLS connector rather than fixed to
/// [`NoTls`], which is the whole of what makes a managed PostgreSQL reachable:
/// this compiles only if an arbitrary `MakeTlsConnect` — a
/// `tokio-postgres-rustls` or `tokio-postgres-openssl` connector — satisfies
/// its bounds. The test server speaks no TLS, so there is nothing to hand it
/// and the function is never called; that it type-checks is the assertion.
// [spec:pgorm:req:conn.pool+4/test]    any TLS connector, not just NoTls
fn connect_with_takes_any_tls_connector<T>(config: Config, tls: T) -> Result<DatabasePool, Error>
where
    T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
    T::Stream: Sync + Send,
    T::TlsConnect: Sync + Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    pgorm::connect_with(config, tls, ManagerConfig::default(), |builder| builder)
}

// [spec:pgorm:req:conn.pool+4/test]    connect_with reaches the ManagerConfig the shorthands fix
#[pgorm_macros::test]
pub async fn connect_with_applies_manager_config() -> Result<(), Error> {
    // The three settings none of the shorthands can express: a caller-chosen
    // tag, a recycling method other than Fast, and a statement cache that is
    // not the default bound.
    let pool = pgorm::connect_with(
        maintenance_config(),
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Clean,
            tag: Some("connect-with-reader".to_owned()),
            statement_cache: StatementCacheSize::Disabled,
            statement_cache_channel: None,
        },
        |builder| builder.max_size(1),
    )?;

    assert_eq!(
        pool.tag().as_str(),
        "connect-with-reader",
        "the tag comes from the ManagerConfig, not a generated default"
    );
    assert_eq!(
        pool.status().max_size,
        1,
        "the builder closure shapes the pool, as it does for connect_with_builder"
    );

    // Capped at one connection, so the second checkout is the first one put
    // through Manager::recycle.
    let conn = pool.get().await?;
    assert_eq!(conn.query_one("SELECT 1", &[]).await?.get::<_, i32>(0), 1);
    conn.batch_execute("SET application_name = 'pgorm-connect-with'")
        .await?;
    assert_eq!(
        session_setting(&conn, "application_name").await?,
        "pgorm-connect-with"
    );
    drop(conn);

    let conn = pool.get().await?;
    assert_ne!(
        session_setting(&conn, "application_name").await?,
        "pgorm-connect-with",
        "RecyclingMethod::Clean runs RESET ALL; Fast, which the shorthands fix, would not"
    );

    // StatementCacheSize::Disabled: each execution prepares its own statement
    // and closes it when the last handle drops, so nothing outlives the call.
    const SQL: &str = "SELECT 'connect-with-uncached'";
    drop(conn.query_one(SQL, &[]).await?);
    drop(conn.query_one(SQL, &[]).await?);
    let live: i64 = conn
        .query_one(
            "SELECT count(*) FROM pg_prepared_statements WHERE statement = $1",
            &[&SQL],
        )
        .await?
        .get(0);
    assert_eq!(live, 0, "a disabled cache holds no statement open");

    Ok(())
}

// [spec:pgorm:req:conn.pool+4/test]    connect_with fails like every other caller-input entry point
#[pgorm_macros::test]
pub async fn connect_with_errs_on_build_failure() -> Result<(), Error> {
    let err = pgorm::connect_with(
        maintenance_config(),
        NoTls,
        ManagerConfig::default(),
        |builder| builder.wait_timeout(Some(Duration::from_secs(1))),
    )
    .expect_err("a wait timeout without a runtime cannot build");

    assert!(matches!(err, Error::Custom(_)), "{err:?}");

    Ok(())
}

// [spec:pgorm:sem:conn.pool.multi+3/test]    one pool per builder entry, tagged and keyed by its map key
#[pgorm_macros::test]
pub async fn connect_multi_builds_tagged_pools() -> Result<(), Error> {
    let mut builders: BTreeMap<String, Box<dyn Fn(PoolBuilder) -> PoolBuilder>> = BTreeMap::new();
    builders.insert("reader".to_owned(), Box::new(|builder| builder.max_size(2)));
    builders.insert("writer".to_owned(), Box::new(|builder| builder.max_size(3)));

    let pools: DatabaseMultiPool =
        pgorm::connect_multi_with_builder(maintenance_config(), builders)?;

    assert_eq!(
        pools.iter().map(|(tag, _)| tag).collect::<Vec<_>>(),
        vec!["reader", "writer"],
        "the result is keyed by tag"
    );

    for (tag, pool) in pools.iter() {
        assert_eq!(
            pool.tag().as_str(),
            tag,
            "each pool carries its own map key as tag"
        );
    }

    let reader = pools.get("reader").expect("reader pool");
    let writer = pools.get("writer").expect("writer pool");

    assert_eq!(
        reader.status().max_size,
        2,
        "each entry's own builder shapes its own pool"
    );
    assert_eq!(writer.status().max_size, 3);
    assert_eq!(
        pools
            .status()
            .into_iter()
            .map(|(tag, status)| (tag, status.max_size))
            .collect::<Vec<_>>(),
        vec![("reader", 2), ("writer", 3)],
        "status reports every pool by tag"
    );

    // Selection is explicit by tag only: an unknown key resolves to nothing,
    // there is no fallback onto a sibling pool.
    assert!(pools.get("missing").is_none());

    // Every pool clones the one shared config, so they reach the same database.
    for (_, pool) in pools.iter() {
        let conn = pool.get().await?;
        let database: String = conn
            .query_one("SELECT current_database()", &[])
            .await?
            .get(0);
        assert_eq!(database, "postgres");
    }

    // Each pool uses RecyclingMethod::Fast, which runs no cleanup query.
    let conn = reader.get().await?;
    conn.batch_execute("SET application_name = 'pgorm-multi-fast'")
        .await?;
    drop(conn);

    let conn = reader.get().await?;
    assert_eq!(
        session_setting(&conn, "application_name").await?,
        "pgorm-multi-fast"
    );
    drop(conn);

    pools.close();
    assert!(
        pools.iter().all(|(_, pool)| pool.is_closed()),
        "close closes every pool"
    );

    Ok(())
}

// [spec:pgorm:sem:conn.pool.multi+3/test]    one unbuildable entry fails the whole construction
#[pgorm_macros::test]
pub async fn connect_multi_errs_on_build_failure() -> Result<(), Error> {
    let mut builders: BTreeMap<String, Box<dyn Fn(PoolBuilder) -> PoolBuilder>> = BTreeMap::new();
    builders.insert("reader".to_owned(), Box::new(|builder| builder.max_size(2)));
    builders.insert(
        "writer".to_owned(),
        Box::new(|builder| builder.wait_timeout(Some(Duration::from_secs(1)))),
    );

    let err = pgorm::connect_multi_with_builder(maintenance_config(), builders)
        .expect_err("one unbuildable entry sinks the whole map");

    assert!(matches!(err, Error::Custom(_)), "{err:?}");

    Ok(())
}
