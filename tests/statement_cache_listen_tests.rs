#![allow(unused_imports, dead_code)]

//! Two pools on one database, as two processes would hold them: one given a
//! `statement_cache_channel` and one not. Whether a pool cleared is judged by
//! the insert its connection holds prepared ([`PREPARED_STATEMENT`]): a clear
//! closes it, and the next insert prepares another. The default an insert
//! writes cannot say, since PostgreSQL re-plans a kept statement whenever the
//! session's caches are reset; it shows only what a clear is for, a domain's
//! changed default written by the insert prepared again.
//!
//! A test that prepares a statement and then asks whether the pool cleared
//! waits for the listener's clears through [`ListenerClears`] first: the
//! clear its `LISTEN` makes lands after the server already shows the
//! `LISTEN`, and would throw a statement prepared in between away.

pub mod common;

pub use common::{TestContext, setup::*};

use std::{future::Future, time::Duration};

use pgorm::{
    Connection, ConnectionMut, DatabaseConnection, DatabasePool, Error, ManagerConfig, NoTls,
    RecyclingMethod, STATEMENT_CACHE_CHANNEL, StatementCacheSize, notify,
};
use pgorm_test_support::{ListenerClears, PREPARED_STATEMENT};
use pretty_assertions::assert_eq;

/// How long a change the listener makes is waited for.
const ARRIVAL: Duration = Duration::from_secs(10);

const INSERT: &str = "INSERT INTO line (id) VALUES ($1) RETURNING qty";

const SCHEMA: &str = "CREATE DOMAIN quantity AS integer DEFAULT 1; \
                      CREATE TABLE line (id integer PRIMARY KEY, qty quantity)";

const SET_DEFAULT: &str = "ALTER DOMAIN quantity SET DEFAULT 2";

/// How long the idle-timeout test's database lets a session idle.
const IDLE_TIMEOUT: &str = "2s";

fn manager(channel: Option<&str>) -> ManagerConfig {
    ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
        tag: None,
        statement_cache: StatementCacheSize::default(),
        statement_cache_channel: channel.map(str::to_owned),
    }
}

/// A one-connection pool on `ctx`'s database listening on the default
/// channel, so each checkout is the connection that ran the last insert.
fn listening_pool(ctx: &TestContext) -> Result<DatabasePool, Error> {
    let base_url = std::env::var("DATABASE_URL").expect("TestContext loaded DATABASE_URL");
    pgorm::connect_with(
        config(&base_url, ctx.db_name()),
        NoTls,
        manager(Some(STATEMENT_CACHE_CHANNEL)),
        |builder| builder.max_size(1),
    )
}

/// A pool on `ctx`'s database whose sessions never idle out, whatever the
/// database sets, for a test to watch the server through.
fn wakeful_pool(ctx: &TestContext) -> DatabasePool {
    let base_url = std::env::var("DATABASE_URL").expect("TestContext loaded DATABASE_URL");
    let mut config = config(&base_url, ctx.db_name());
    config.options("-c idle_session_timeout=0");
    pgorm::connect(config)
}

/// Poll `probe` until it answers `Some`, failing the test past [`ARRIVAL`].
async fn eventually<T, F, Fut>(what: &str, mut probe: F) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<T>, Error>>,
{
    let deadline = tokio::time::Instant::now() + ARRIVAL;
    loop {
        if let Some(answer) = probe().await? {
            return Ok(answer);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what} did not happen within {ARRIVAL:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The backend of this database's idle session whose last statement was the
/// listener's `LISTEN`, which is how it is told from the pool's connections.
async fn listener_backend(db: &DatabaseConnection) -> Result<Option<i32>, Error> {
    Ok(db
        .query_opt(
            "SELECT pid FROM pg_stat_activity WHERE datname = current_database() \
             AND query = $1 AND state = 'idle'",
            &[&format!("LISTEN \"{STATEMENT_CACHE_CHANNEL}\"")],
        )
        .await?
        .map(|row| row.get(0)))
}

/// Start `pool`'s listener and wait for the clear its `LISTEN` makes. Nothing
/// but a checkout starts the listener, so one is taken and given straight back.
async fn first_clear(pool: &DatabasePool, clears: &ListenerClears) -> Result<(), Error> {
    drop(pool.get().await?);
    clears.reach(1).await;
    Ok(())
}

/// Open `pool`'s listener and wait for its `LISTEN`, answering its backend.
async fn listening_backend(pool: &DatabasePool, watcher: &DatabasePool) -> Result<i32, Error> {
    drop(pool.get().await?);
    eventually("the listener's LISTEN", || async {
        listener_backend(&watcher.get().await?).await
    })
    .await
}

async fn backend_alive(watcher: &DatabasePool, pid: i32) -> Result<bool, Error> {
    Ok(watcher
        .get()
        .await?
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1)",
            &[&pid],
        )
        .await?
        .get(0))
}

/// Open a session that takes `ctx`'s database settings and wait until the
/// server ends it for idling. By then every session that has idled since
/// before it opened has idled as long, and would have been ended too.
async fn a_session_idles_out(ctx: &TestContext, watcher: &DatabasePool) -> Result<(), Error> {
    let base_url = std::env::var("DATABASE_URL").expect("TestContext loaded DATABASE_URL");
    let session = pgorm_test_support::connect(&base_url, ctx.db_name()).await?;
    let pid: i32 = session
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    eventually("the database's idle_session_timeout", || async {
        Ok((!backend_alive(watcher, pid).await?).then_some(()))
    })
    .await
}

/// The insert `db`'s session holds prepared, or `None` once a clear has
/// closed it.
async fn prepared_insert(db: &DatabaseConnection) -> Result<Option<String>, Error> {
    Ok(db
        .query_opt(PREPARED_STATEMENT, &[&INSERT])
        .await?
        .map(|row| row.get(0)))
}

/// Insert through `db`, answering the default written and the prepared
/// statement that wrote it.
async fn insert_on(db: &DatabaseConnection, id: i32) -> Result<(i32, String), Error> {
    let qty = db.query_one(INSERT, &[&id]).await?.get(0);
    let statement = prepared_insert(db).await?.expect("the insert is prepared");
    Ok((qty, statement))
}

async fn insert(pool: &DatabasePool, id: i32) -> Result<(i32, String), Error> {
    insert_on(&pool.get().await?, id).await
}

/// Once the listener has made `made` clears, insert through `pool`'s
/// connection, which the clear lent it with its prepared insert closed.
async fn insert_after_clears(
    pool: &DatabasePool,
    clears: &ListenerClears,
    made: usize,
    id: i32,
) -> Result<(i32, String), Error> {
    clears.reach(made).await;
    let db = pool.get().await?;
    assert_eq!(
        prepared_insert(&db).await?,
        None,
        "the clear closed the insert"
    );
    insert_on(&db, id).await
}

/// A notification on the channel clears the pool that listens there, and
/// only that pool: the listening pool's insert is closed, and prepared again
/// it writes the new default, while the pool without the setting keeps the
/// insert it prepared.
// [spec:pgorm:req:conn.pool.statement-cache.listen+1/test]    a notification, whatever its payload,
// clears the listening pool's statements, and a pool without the setting keeps its own
#[test]
fn only_the_listening_pool_clears() -> Result<(), Error> {
    let clears = ListenerClears::default();
    clears.run(async {
        let ctx = TestContext::new("statement_cache_listen_notified").await;
        let quiet = &ctx.db;
        quiet.get().await?.batch_execute(SCHEMA).await?;
        let listening = listening_pool(&ctx)?;
        first_clear(&listening, &clears).await?;

        let (qty, notified) = insert(&listening, 1).await?;
        assert_eq!(qty, 1);
        assert_eq!(insert(&listening, 2).await?.1, notified);
        let (qty, kept) = insert(quiet, 3).await?;
        assert_eq!(qty, 1);

        let db = quiet.get().await?;
        db.batch_execute(SET_DEFAULT).await?;
        notify(&db, STATEMENT_CACHE_CHANNEL, "the payload is not read").await?;
        drop(db);

        let (qty, prepared_again) = insert_after_clears(&listening, &clears, 2, 4).await?;
        assert_eq!(qty, 2, "the insert prepared again writes the new default");
        assert_ne!(prepared_again, notified);
        assert_eq!(
            insert(quiet, 5).await?.1,
            kept,
            "the quiet pool kept its insert"
        );

        drop(listening);
        ctx.delete().await;
        Ok(())
    })
}

/// A listener whose connection is terminated reconnects and clears once, for
/// whatever was notified while it was gone, with nothing notified at all.
// [spec:pgorm:req:conn.pool.statement-cache.listen+1/test]    a terminated listener reconnects and
// clears the pool's statements once it listens again
#[test]
fn a_reconnected_listener_clears_the_pool() -> Result<(), Error> {
    let clears = ListenerClears::default();
    clears.run(async {
        let ctx = TestContext::new("statement_cache_listen_reconnected").await;
        let watcher = &ctx.db;
        watcher.get().await?.batch_execute(SCHEMA).await?;
        let listening = listening_pool(&ctx)?;
        first_clear(&listening, &clears).await?;
        let lost = listening_backend(&listening, watcher).await?;

        let (qty, before) = insert(&listening, 1).await?;
        assert_eq!(qty, 1);
        let db = watcher.get().await?;
        db.batch_execute(SET_DEFAULT).await?;
        assert_eq!(
            insert(&listening, 2).await?.1,
            before,
            "unnotified, the pool keeps its insert"
        );
        db.execute("SELECT pg_terminate_backend($1)", &[&lost])
            .await?;
        drop(db);

        eventually("the reconnection", || async {
            Ok(listener_backend(&watcher.get().await?)
                .await?
                .filter(|pid| *pid != lost))
        })
        .await?;
        let (qty, after) = insert_after_clears(&listening, &clears, 2, 3).await?;
        assert_eq!(qty, 2);
        assert_ne!(after, before);

        drop(listening);
        ctx.delete().await;
        Ok(())
    })
}

/// A database that ends idle sessions does not end the listener's: once a
/// session opened after its `LISTEN` has idled out, the listener is the
/// backend it was, and the pool has not cleared, its connection holding the
/// insert it prepared before.
// [spec:pgorm:req:conn.pool.statement-cache.listen+1/test]    the listening session outlives the
// database's idle_session_timeout, and the pool does not clear
#[test]
fn an_idle_timeout_spares_the_listener() -> Result<(), Error> {
    let clears = ListenerClears::default();
    clears.run(async {
        let ctx = TestContext::new("statement_cache_listen_idle").await;
        let db = ctx.db.get().await?;
        db.batch_execute(SCHEMA).await?;
        db.batch_execute(&format!(
            "ALTER DATABASE \"{}\" SET idle_session_timeout = '{IDLE_TIMEOUT}'",
            ctx.db_name()
        ))
        .await?;
        drop(db);
        let watcher = wakeful_pool(&ctx);
        let listening = listening_pool(&ctx)?;
        // The pool's own connection takes the database's timeout as the
        // listener's does; it is kept from idling out so that only the
        // listener's session is under test.
        listening
            .get()
            .await?
            .batch_execute("SET idle_session_timeout = 0")
            .await?;
        first_clear(&listening, &clears).await?;
        let listener = listening_backend(&listening, &watcher).await?;

        let before = insert(&listening, 1).await?.1;
        a_session_idles_out(&ctx, &watcher).await?;

        assert_eq!(
            listener_backend(&watcher.get().await?).await?,
            Some(listener),
            "the listener's session outlives the timeout"
        );
        assert_eq!(clears.count(), 1, "the listener cleared only as it began");
        assert_eq!(
            insert(&listening, 2).await?.1,
            before,
            "nothing cleared the pool"
        );

        drop(listening);
        drop(watcher);
        ctx.delete().await;
        Ok(())
    })
}

// [spec:pgorm:req:conn.pool.statement-cache.listen+1/test]    closing the pool closes its listener's connection
#[pgorm_macros::test]
async fn closing_the_pool_closes_its_listener() -> Result<(), Error> {
    let ctx = TestContext::new("statement_cache_listen_closed").await;
    let listening = listening_pool(&ctx)?;
    let pid = listening_backend(&listening, &ctx.db).await?;

    listening.close();
    eventually("the listener's disconnection", || async {
        Ok((!backend_alive(&ctx.db, pid).await?).then_some(()))
    })
    .await?;

    drop(listening);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:conn.pool.statement-cache.listen+1/test]    connect_with refuses an empty channel and
// one past 63 bytes, before anything connects
#[test]
fn connect_with_refuses_an_unusable_channel() {
    let config: pgorm::Config = "postgres://postgres@localhost/unreached"
        .parse()
        .expect("a valid connection string");
    let build =
        |channel: &str| pgorm::connect_with(config.clone(), NoTls, manager(Some(channel)), |b| b);

    assert!(matches!(build(""), Err(Error::Custom(_))));
    assert!(matches!(build(&"c".repeat(64)), Err(Error::Custom(_))));
    assert!(build(&"c".repeat(63)).is_ok());
    assert!(
        pgorm::connect_with(config.clone(), NoTls, manager(None), |b| b).is_ok(),
        "the setting is off by default"
    );
}
