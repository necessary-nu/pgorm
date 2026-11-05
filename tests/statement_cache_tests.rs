#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, setup::*};
use pgorm::{ConnectionTrait, DatabaseConnection, Error, TransactionTrait};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

const SELECT_ALL: &str = "SELECT * FROM plan_check";

const NO_PARAMS: [&(dyn tokio_postgres::types::ToSql + Sync); 0] = [];

/// How many live server-side prepared statements this session holds for `sql`.
///
/// The view is session-local, so every query in a test has to run on the one
/// connection it took out of the pool. A statement prepared afresh per call is
/// closed when its handle drops, leaving nothing here; a cached one stays.
async fn prepared_count(db: &DatabaseConnection, sql: &str) -> Result<i64, Error> {
    let row = db
        .query_one(
            "SELECT count(*) FROM pg_prepared_statements WHERE statement = $1",
            &[&sql],
        )
        .await?;

    Ok(row.get(0))
}

// [spec:pgorm:sem:conn.pool.statement-cache+3/test]    one Parse per connection, not per call
// [spec:pgorm:def:conn.pool.conn-trait+9/test]    the text-carrying statement routes through the cache
#[pgorm_macros::test]
async fn repeated_queries_reuse_one_prepared_statement() -> Result<(), Error> {
    let ctx = TestContext::new("reuse_one_prepared_stmtcache").await;
    let db = ctx.db.get().await?;

    db.batch_execute("CREATE TABLE plan_check (a int4); INSERT INTO plan_check VALUES (1);")
        .await?;

    for _ in 0..3 {
        assert_eq!(db.query_all(SELECT_ALL, &[]).await?.len(), 1);
    }

    assert_eq!(
        prepared_count(&db, SELECT_ALL).await?,
        1,
        "three executions of one text prepare it once"
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.pool.statement-cache.invalidate+4/test]    autocommit: DDL that changes the result type is retried
#[pgorm_macros::test]
async fn added_column_reprepares_the_cached_plan() -> Result<(), Error> {
    let ctx = TestContext::new("added_column_reprepares_stmtcache").await;
    let db = ctx.db.get().await?;

    db.batch_execute("CREATE TABLE plan_check (a int4); INSERT INTO plan_check VALUES (1);")
        .await?;

    // Rows are never bound to a variable: a `Row` holds the statement it was
    // decoded against, so keeping one would keep the evicted statement alive
    // and the count below would see it.
    assert_eq!(db.query_all(SELECT_ALL, &[]).await?[0].len(), 1);
    assert_eq!(prepared_count(&db, SELECT_ALL).await?, 1);

    db.batch_execute("ALTER TABLE plan_check ADD COLUMN b text")
        .await?;

    let columns = db
        .query_all(SELECT_ALL, &[])
        .await
        .expect("the rejected plan is evicted and prepared again")[0]
        .len();

    assert_eq!(columns, 2, "the retry runs against the table as it is now");
    assert_eq!(
        prepared_count(&db, SELECT_ALL).await?,
        1,
        "the stale statement was closed, not left beside its replacement"
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.pool.statement-cache.invalidate+4/test]    exactly one retry, then the error
#[pgorm_macros::test]
async fn a_second_stale_plan_error_surfaces() -> Result<(), Error> {
    let ctx = TestContext::new("second_stale_plan_stmtcache").await;
    let db = ctx.db.get().await?;

    // A sequence counts the attempts because `nextval` survives the rollback
    // the raised exception performs.
    db.batch_execute(
        "CREATE SEQUENCE attempts;
         CREATE FUNCTION always_stale() RETURNS int LANGUAGE plpgsql AS $$
         BEGIN
             PERFORM nextval('attempts');
             RAISE EXCEPTION 'cached plan must not change result type'
                 USING ERRCODE = '0A000';
         END $$;",
    )
    .await?;

    let error = db
        .query_all("SELECT always_stale()", &[])
        .await
        .expect_err("a statement that fails the same way twice fails the caller");

    match &error {
        Error::Postgres(error) => {
            assert_eq!(error.code(), Some(&SqlState::FEATURE_NOT_SUPPORTED))
        }
        other => panic!("expected Error::Postgres, got {other:?}"),
    }

    let attempts: i64 = db
        .query_one("SELECT last_value FROM attempts", &[])
        .await?
        .get(0);
    assert_eq!(attempts, 2, "one retry, not a loop");

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:conn.pool.conn-trait+9/test]    a transaction routes through its connection's cache
#[pgorm_macros::test]
async fn transaction_shares_the_connection_cache() -> Result<(), Error> {
    let ctx = TestContext::new("txn_shares_cache_stmtcache").await;
    let mut db = ctx.db.get().await?;

    db.batch_execute("CREATE TABLE plan_check (a int4); INSERT INTO plan_check VALUES (1);")
        .await?;
    assert_eq!(db.query_all(SELECT_ALL, &[]).await?.len(), 1);

    db.transaction(async |txn| {
        assert_eq!(txn.query_all(SELECT_ALL, &[]).await?.len(), 1);
        Ok::<_, Error>(())
    })
    .await
    .expect("the transaction commits");

    assert_eq!(
        prepared_count(&db, SELECT_ALL).await?,
        1,
        "the transaction reused the statement its connection had prepared"
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.pool.statement-cache.invalidate+4/test]    ROLLBACK does not discard a Parse
#[pgorm_macros::test]
async fn rolled_back_transaction_keeps_its_statements() -> Result<(), Error> {
    let ctx = TestContext::new("rolled_back_keeps_stmtcache").await;
    let mut db = ctx.db.get().await?;

    db.batch_execute("CREATE TABLE plan_check (a int4); INSERT INTO plan_check VALUES (1);")
        .await?;

    let outcome = db
        .transaction(async |txn| {
            assert_eq!(txn.query_all(SELECT_ALL, &[]).await?.len(), 1);
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);

    assert_eq!(
        db.query_all(SELECT_ALL, &[]).await?.len(),
        1,
        "the entry the transaction left in the cache still names a live statement"
    );
    assert_eq!(prepared_count(&db, SELECT_ALL).await?, 1);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:conn.pool.conn-trait+9/test]    the simple-query path prepares nothing
#[pgorm_macros::test]
async fn batch_execute_prepares_nothing() -> Result<(), Error> {
    let ctx = TestContext::new("batch_prepares_nothing_stmtcache").await;
    let db = ctx.db.get().await?;

    db.batch_execute("CREATE TABLE plan_check (a int4)").await?;

    assert_eq!(
        prepared_count(&db, "CREATE TABLE plan_check (a int4)").await?,
        0,
        "batch_execute goes through the simple-query protocol"
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

/// The SQLSTATE `error` carries, for a `Result` a test expected to fail.
#[track_caller]
fn sqlstate<T: std::fmt::Debug>(result: Result<T, Error>) -> SqlState {
    match result.expect_err("expected a database error") {
        Error::Postgres(error) => error
            .code()
            .cloned()
            .expect("expected the error to carry a SQLSTATE"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

// [spec:pgorm:req:conn.pool.statement-cache.invalidate+4/test]    in a transaction: the original error, not the retry's
#[pgorm_macros::test]
async fn a_tx_stale_plan_surfaces_the_original_error() -> Result<(), Error> {
    let ctx = TestContext::new("tx_stale_plan_stmtcache").await;
    let mut db = ctx.db.get().await?;

    db.batch_execute("CREATE TABLE plan_check (a int4); INSERT INTO plan_check VALUES (1);")
        .await?;
    assert_eq!(db.query_all(SELECT_ALL, &[]).await?[0].len(), 1);

    db.batch_execute("ALTER TABLE plan_check ADD COLUMN b text")
        .await?;

    let outcome = db
        .transaction(async |txn| {
            let code = sqlstate(txn.query_all(SELECT_ALL, &[]).await);
            assert_eq!(
                code,
                SqlState::FEATURE_NOT_SUPPORTED,
                "the caller is told the plan went stale, not that the transaction it \
                 aborted is aborted (25P02)"
            );
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);

    // The eviction happened even though the retry did not, so the caller's own
    // re-run prepares afresh and succeeds.
    let columns = db
        .query_all(SELECT_ALL, &[])
        .await
        .expect("the evicted key is prepared again on the next call")[0]
        .len();
    assert_eq!(columns, 2, "the fresh statement sees the added column");
    assert_eq!(
        prepared_count(&db, SELECT_ALL).await?,
        1,
        "the stale statement was closed, not left beside its replacement"
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.pool.statement-cache.invalidate+4/test]    a nested transaction fences the rejection
#[pgorm_macros::test]
async fn nested_tx_stale_plan_leaves_the_outer_alive() -> Result<(), Error> {
    let ctx = TestContext::new("nested_stale_plan_stmtcache").await;
    let mut db = ctx.db.get().await?;

    db.batch_execute(
        "CREATE TABLE plan_check (a int4); INSERT INTO plan_check VALUES (1);
         CREATE TABLE witness (a int4);",
    )
    .await?;
    assert_eq!(db.query_all(SELECT_ALL, &[]).await?[0].len(), 1);

    db.batch_execute("ALTER TABLE plan_check ADD COLUMN b text")
        .await?;

    db.transaction(async |outer| {
        outer.execute("INSERT INTO witness VALUES (7)", &[]).await?;

        let outcome = outer
            .transaction(async |nested| {
                let code = sqlstate(nested.query_all(SELECT_ALL, &[]).await);
                assert_eq!(code, SqlState::FEATURE_NOT_SUPPORTED);
                roll_back::<()>()
            })
            .await;
        assert_rolled_back(outcome);

        // Only the savepoint's subtransaction was aborted, so the outer one is
        // live and the evicted key prepares again inside it.
        let columns = outer
            .query_all(SELECT_ALL, &[])
            .await
            .expect("ROLLBACK TO SAVEPOINT left the outer transaction usable")[0]
            .len();
        assert_eq!(columns, 2);
        Ok::<_, Error>(())
    })
    .await
    .expect("the outer transaction commits");

    let witnessed: i64 = db
        .query_one("SELECT count(*) FROM witness", &[])
        .await?
        .get(0);
    assert_eq!(witnessed, 1, "the outer transaction's work committed");

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.pool.statement-cache.invalidate+4/test]    the iterator-parameter methods, in a transaction
#[pgorm_macros::test]
async fn iterator_path_stale_plan_inside_a_transaction() -> Result<(), Error> {
    let ctx = TestContext::new("tx_stale_iter_stmtcache").await;
    let mut db = ctx.db.get().await?;

    db.batch_execute("CREATE TABLE plan_check (a int4); INSERT INTO plan_check VALUES (1);")
        .await?;
    assert_eq!(db.query_all(SELECT_ALL, &[]).await?[0].len(), 1);

    db.batch_execute("ALTER TABLE plan_check ADD COLUMN b text")
        .await?;

    let outcome = db
        .transaction(async |txn| {
            let code = sqlstate(txn.execute_raw(SELECT_ALL, NO_PARAMS).await);
            assert_eq!(
                code,
                SqlState::FEATURE_NOT_SUPPORTED,
                "the path that never retried still must not report 25P02"
            );
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);

    db.transaction(async |txn| {
        let stream = txn
            .query_raw(SELECT_ALL, NO_PARAMS)
            .await
            .expect("the evicted key prepares again in a fresh transaction");
        drop(stream);
        Ok::<_, Error>(())
    })
    .await
    .expect("the fresh transaction commits");

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:conn.pool.statement-cache.invalidate+4/test]    autocommit control: the iterator path recovers one call later
#[pgorm_macros::test]
async fn iterator_path_stale_plan_recovers_next_call() -> Result<(), Error> {
    let ctx = TestContext::new("iter_stale_next_call_stmtcache").await;
    let db = ctx.db.get().await?;

    db.batch_execute("CREATE TABLE plan_check (a int4); INSERT INTO plan_check VALUES (1);")
        .await?;
    assert_eq!(db.query_all(SELECT_ALL, &[]).await?[0].len(), 1);

    db.batch_execute("ALTER TABLE plan_check ADD COLUMN b text")
        .await?;

    let code = sqlstate(db.execute_raw(SELECT_ALL, NO_PARAMS).await);
    assert_eq!(code, SqlState::FEATURE_NOT_SUPPORTED);

    assert_eq!(
        db.execute_raw(SELECT_ALL, NO_PARAMS)
            .await
            .expect("the second call prepares the evicted key again"),
        1
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}
