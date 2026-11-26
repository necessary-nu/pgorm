//! Connections that may only read, against a live server: the session the
//! read-only pool opens, the server's refusal of what the types cannot see,
//! read-only transactions, the type-only view, COPY and the metrics layer.
#![allow(unused_imports, dead_code)]

pub mod common;

use async_trait::async_trait;
use bytes::Bytes;
pub use common::TestContext;
use futures::TryStreamExt;
use pgorm::{
    CopyConnection, IsolationLevel, ManagerConfig, NoTls, RecyclingMethod, SelectModel,
    SelectorRaw, TransactionError, connect_read_only, connect_read_only_with,
    connect_read_only_with_builder,
    entity::prelude::*,
    metric::{InstrumentedPool, MetricsCollector, QueryContext},
    pgorm_query::{CopySource, CopyText, CopyToStatement, Query, Values},
    sql,
    tests_cfg::cake,
};
use pretty_assertions::assert_eq;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const SCHEMA: &str = sql!(
    "CREATE TABLE cake (id int PRIMARY KEY, name text NOT NULL);
     INSERT INTO cake (id, name) VALUES (1, 'Chocolate'), (2, 'Lemon');
     CREATE FUNCTION bake() RETURNS int LANGUAGE sql
         AS $$ INSERT INTO cake (id, name) VALUES (3, 'Carrot') RETURNING id $$;"
)
.as_str();

async fn transaction_read_only(db: &impl Connection) -> Result<String, Error> {
    Ok(db
        .query_one(sql!("SHOW transaction_read_only"), &[])
        .await?
        .get(0))
}

async fn default_read_only(db: &impl Connection) -> Result<String, Error> {
    Ok(db
        .query_one(sql!("SHOW default_transaction_read_only"), &[])
        .await?
        .get(0))
}

async fn isolation(db: &impl Connection) -> Result<String, Error> {
    Ok(db
        .query_one(sql!("SHOW transaction_isolation"), &[])
        .await?
        .get(0))
}

async fn deferrable(db: &impl Connection) -> Result<String, Error> {
    Ok(db
        .query_one(sql!("SHOW transaction_deferrable"), &[])
        .await?
        .get(0))
}

/// The rows a connection that may write sees, so a refusal is shown to have
/// written nothing.
async fn cakes(ctx: &TestContext) -> Result<u64, Error> {
    cake::Entity::find().count(&ctx.db.get().await?).await
}

async fn context(label: &str) -> Result<TestContext, Error> {
    let ctx = TestContext::new(label).await;
    ctx.db.get().await?.batch_execute(SCHEMA).await?;
    Ok(ctx)
}

// [spec:pgorm:req:read-only.pool/test]    a read-only pool's connections read through every builder and run in a session the server holds read-only
// [spec:pgorm:req:read-only.refusal/test]    what the types cannot see is refused by the server as a `ReadOnlyRefusal`, and nothing else is one
// [spec:pgorm:req:read-only.unchecked/test]    text of any kind runs through `unchecked`, the server refusing a write
// [spec:pgorm:def:read-only.kinds/test]    the reads pgorm builds and `sql!` reads run on a connection that may only read, run-time text through `unchecked`
// [spec:pgorm:def:read-only.traits/test]    a read-only connection runs every read builder
#[pgorm_macros::test]
async fn read_only_pool_reads_and_server_refuses_writes() -> Result<(), Error> {
    let ctx = context("read_only_pool").await?;
    let pool = connect_read_only(ctx.config())?;
    let ro = pool.get().await?;

    let all = cake::Entity::find()
        .order_by_asc(cake::Column::Id)
        .all(&ro)
        .await?;
    assert_eq!(all.len(), 2);
    assert_eq!(cake::Entity::find().count(&ro).await?, 2);
    assert_eq!(default_read_only(&ro).await?, "on");
    assert_eq!(transaction_read_only(&ro).await?, "on");

    let function = ro
        .query_one(sql!("SELECT bake()"), &[])
        .await
        .expect_err("a function that writes is refused");
    let refusal = function
        .read_only_refusal()
        .expect("the refusal is the server's 25006");
    assert_eq!(
        refusal.message,
        "cannot execute INSERT in a read-only transaction"
    );

    let locked = cake::Entity::find()
        .lock_exclusive()
        .all(&ro)
        .await
        .expect_err("a row lock is refused");
    assert!(locked.read_only_refusal().is_some(), "{locked}");

    let text = format!("INSERT INTO cake (id, name) VALUES ({}, 'Lime')", 4);
    let inserted = ro
        .unchecked()
        .execute(&text, &[])
        .await
        .expect_err("text that writes is refused by the server");
    assert!(inserted.read_only_refusal().is_some(), "{inserted}");

    let read = format!("SELECT id, name FROM cake WHERE id = {}", 2);
    let lemon = SelectorRaw::<SelectModel<cake::Model>>::from_statement(&read, Values(vec![]))
        .one(&ro.unchecked())
        .await?;
    assert_eq!(lemon.name, "Lemon");

    let missing = ro
        .query_one(sql!("SELECT * FROM no_such_table"), &[])
        .await
        .expect_err("the table does not exist");
    assert_eq!(missing.read_only_refusal(), None);
    assert_eq!(cakes(&ctx).await?, 2);

    drop(ro);
    drop(pool);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:read-only.pool/test]    the option is appended after the caller's, and survives `RESET ALL` and Clean recycling's `DISCARD ALL`
#[pgorm_macros::test]
async fn read_only_option_outlasts_callers_and_discard() -> Result<(), Error> {
    let ctx = context("read_only_options").await?;
    let mut config = ctx.config();
    config.options("-c default_transaction_read_only=off -c statement_timeout=12345");
    let pool = connect_read_only_with(
        config,
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Clean,
            ..ManagerConfig::default()
        },
        |builder| builder.max_size(1),
    )?;

    {
        let ro = pool.get().await?;
        assert_eq!(default_read_only(&ro).await?, "on");
        let timeout: String = ro
            .query_one(sql!("SHOW statement_timeout"), &[])
            .await?
            .get(0);
        assert_eq!(timeout, "12345ms");
        ro.unchecked().execute("RESET ALL", &[]).await?;
        assert_eq!(default_read_only(&ro).await?, "on");
    }
    // Clean recycling ran `DISCARD ALL` as the connection went back.
    let ro = pool.get().await?;
    assert_eq!(default_read_only(&ro).await?, "on");

    let built = connect_read_only_with_builder(ctx.config(), |builder| builder.max_size(1))?;
    assert_eq!(default_read_only(&built.get().await?).await?, "on");

    drop(ro);
    drop(pool);
    drop(built);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:read-only.transactions/test]    `read_only_transaction` opens `READ ONLY` at its isolation, `snapshot` the deferrable one, each handle running reads and the server refusing a write, a savepoint inside of the same kind
#[pgorm_macros::test]
async fn read_only_transactions_on_a_writable_connection() -> Result<(), Error> {
    let ctx = context("read_only_transactions").await?;
    let mut db = ctx.db.get().await?;

    db.read_only_transaction(Some(IsolationLevel::RepeatableRead), async |tx| {
        assert_eq!(transaction_read_only(tx).await?, "on");
        assert_eq!(isolation(tx).await?, "repeatable read");
        assert_eq!(cake::Entity::find().count(tx).await?, 2);
        let refused = tx
            .transaction(async |savepoint| {
                assert_eq!(transaction_read_only(savepoint).await?, "on");
                savepoint
                    .unchecked()
                    .execute("DELETE FROM cake", &[])
                    .await
                    .map(drop)
            })
            .await;
        match refused {
            Err(TransactionError::Transaction(error)) => {
                assert!(error.read_only_refusal().is_some(), "{error}");
            }
            other => panic!("the savepoint's delete was refused, not {other:?}"),
        }
        assert_eq!(cake::Entity::find().count(tx).await?, 2);
        Ok::<_, Error>(())
    })
    .await?;

    db.snapshot(async |tx| {
        assert_eq!(transaction_read_only(tx).await?, "on");
        assert_eq!(isolation(tx).await?, "serializable");
        assert_eq!(deferrable(tx).await?, "on");
        Ok::<_, Error>(())
    })
    .await?;

    assert_eq!(transaction_read_only(&db).await?, "off");
    assert_eq!(cakes(&ctx).await?, 2);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:read-only.transactions/test]    a read-only connection's transactions, plain, configured and the snapshot, are read-only
#[pgorm_macros::test]
async fn a_read_only_connection_opens_read_only_transactions() -> Result<(), Error> {
    let ctx = context("read_only_connection_tx").await?;
    let pool = connect_read_only(ctx.config())?;
    let mut ro = pool.get().await?;

    ro.transaction(async |tx| {
        assert_eq!(transaction_read_only(tx).await?, "on");
        tx.transaction(async |savepoint| {
            assert_eq!(cake::Entity::find().count(savepoint).await?, 2);
            Ok::<_, Error>(())
        })
        .await?;
        Ok::<_, Error>(())
    })
    .await?;
    ro.read_only_transaction(Some(IsolationLevel::Serializable), async |tx| {
        assert_eq!(isolation(tx).await?, "serializable");
        Ok::<_, Error>(())
    })
    .await?;
    ro.snapshot(async |tx| {
        assert_eq!(deferrable(tx).await?, "on");
        Ok::<_, Error>(())
    })
    .await?;

    drop(ro);
    drop(pool);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:read-only.downgrade/test]    `read_only()` reads through every builder by its type alone, the session untouched
#[pgorm_macros::test]
async fn read_only_borrows_a_connection_by_type_alone() -> Result<(), Error> {
    let ctx = context("read_only_view").await?;
    let db = ctx.db.get().await?;
    let reader = db.read_only();

    assert_eq!(cake::Entity::find().count(&reader).await?, 2);
    let lemon = cake::Entity::find_by_id(2).one(&reader).await?;
    assert_eq!(lemon.name, "Lemon");
    assert_eq!(transaction_read_only(&reader).await?, "off");

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:read-only.copy/test]    COPY TO runs on a read-only connection, and one over a write's RETURNING is refused by its server
#[pgorm_macros::test]
async fn copy_out_reads_and_writing_copy_is_refused() -> Result<(), Error> {
    let ctx = context("read_only_copy").await?;
    let pool = connect_read_only(ctx.config())?;
    let mut ro = pool.get().await?;

    let chunks: Vec<Bytes> = ro
        .copy_out(&CopyToStatement::new(cake::Entity, CopyText))
        .await?
        .try_collect()
        .await?;
    assert_eq!(chunks.concat(), b"1\tChocolate\n2\tLemon\n");

    let insert = Query::insert()
        .into_table(cake::Entity)
        .columns([cake::Column::Id, cake::Column::Name])
        .values_panic([5.into(), "Plum".into()])
        .returning_col(cake::Column::Id)
        .to_owned();
    let copy = CopyToStatement::new(CopySource::try_from(insert)?, CopyText);
    let refused = match ro.copy_out(&copy).await {
        Ok(stream) => stream
            .try_collect::<Vec<Bytes>>()
            .await
            .expect_err("the server refuses the insert"),
        Err(error) => error,
    };
    assert!(refused.read_only_refusal().is_some(), "{refused}");
    assert_eq!(cakes(&ctx).await?, 2);

    drop(ro);
    drop(pool);
    ctx.delete().await;
    Ok(())
}

#[derive(Clone, Debug, Default)]
struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    fn push(&self, event: String) {
        self.0.lock().expect("the events lock").push(event);
    }

    fn all(&self) -> Vec<String> {
        self.0.lock().expect("the events lock").clone()
    }
}

#[async_trait]
impl MetricsCollector for Events {
    async fn record_query_success(&self, query: QueryContext<'_>, _: Duration, _: Option<u64>) {
        self.push(format!("ok:{}", query.operation()));
    }

    async fn record_query_error(&self, query: QueryContext<'_>, _: Duration, _: &Error) {
        self.push(format!("error:{}", query.operation()));
    }

    async fn record_connection_acquired(&self, _: Duration) {
        self.push("acquired".to_owned());
    }

    async fn record_connection_error(&self, _: Duration, _: &Error) {
        self.push("acquire_failed".to_owned());
    }

    async fn record_transaction_begin(&self, _: Duration) {
        self.push("begin".to_owned());
    }

    async fn record_transaction_commit(&self, _: Duration) {
        self.push("commit".to_owned());
    }

    async fn record_transaction_rollback(&self, _: Duration) {
        self.push("rollback".to_owned());
    }
}

// [spec:pgorm:req:read-only.metrics/test]    an instrumented read-only pool lends instrumented read-only connections and transactions that report as the others do
#[pgorm_macros::test]
async fn an_instrumented_read_only_pool_stays_read_only() -> Result<(), Error> {
    let ctx = context("read_only_metrics").await?;
    let events = Events::default();
    let pool = InstrumentedPool::new(connect_read_only(ctx.config())?, events.clone());
    let mut ro = pool.get().await?;

    assert_eq!(cake::Entity::find().count(&ro).await?, 2);
    let refused = ro
        .unchecked()
        .execute("DELETE FROM cake", &[])
        .await
        .expect_err("the server refuses the delete");
    assert!(refused.read_only_refusal().is_some(), "{refused}");
    ro.read_only_transaction(None, async |tx| {
        assert_eq!(transaction_read_only(tx).await?, "on");
        Ok::<_, Error>(())
    })
    .await?;
    ro.snapshot(async |tx| tx.unchecked().execute("SELECT 1", &[]).await.map(drop))
        .await?;

    assert_eq!(
        events.all(),
        [
            "acquired",
            "ok:query_opt",
            "error:execute",
            "begin",
            "ok:query_one",
            "commit",
            "begin",
            "ok:execute",
            "commit",
        ]
    );
    assert_eq!(cakes(&ctx).await?, 2);

    drop(ro);
    drop(pool);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:read-only.turso/test]    a read-only `:memory:` target is refused before anything opens
#[test]
fn read_only_memory_target_is_refused_early() {
    let config = ":memory:".parse().expect("`:memory:` is a native target");
    let Err(refused) = connect_read_only(config) else {
        panic!("a read-only `:memory:` pool was built");
    };
    let Error::Postgres(driver) = &refused else {
        panic!("the refusal is the driver's, not {refused:?}");
    };
    assert_eq!(
        driver.to_string(),
        "native Turso does not support a read-only `:memory:` database"
    );
}
