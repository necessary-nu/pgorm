#![allow(unused_imports, dead_code)]

//! A transaction whose closure caught a failed statement's error and then
//! returned `Ok`: PostgreSQL answers its `COMMIT` with a rollback, and pgorm
//! reports that rather than a commit.

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
use pgorm::{TransactionError, TransactionMode, entity::prelude::*};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

async fn insert_bakery<C>(db: &C, name: &str) -> Result<(), Error>
where
    C: ConnectionTrait,
{
    bakery::ActiveModel {
        name: set(name),
        profit_margin: set(10.0),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(())
}

async fn bakeries<C>(db: &C) -> Result<Vec<String>, Error>
where
    C: ConnectionTrait,
{
    Ok(Bakery::find()
        .order_by_asc(bakery::Column::Name)
        .all(db)
        .await?
        .into_iter()
        .map(|bakery| bakery.name)
        .collect())
}

/// Run a statement that fails, and swallow its error, as a closure that
/// catches it does.
async fn fail_and_catch<C>(db: &C)
where
    C: ConnectionTrait,
{
    let refused = db.batch_execute("SELECT 1 / 0").await;
    assert!(refused.is_err(), "the division fails");
}

/// The `TransactionAborted` a transaction's outcome must be, holding the
/// server's `25P02` as its source.
// [spec:pgorm:def:error.model+14/test]    TransactionAborted renders pgorm's account and sources the server's refusal
fn assert_aborted<T: std::fmt::Debug>(outcome: Result<T, TransactionError<Error>>) {
    match outcome {
        Err(TransactionError::Connection(error @ Error::TransactionAborted(_))) => {
            assert_eq!(
                error.to_string(),
                "the transaction was rolled back, not committed: a statement that failed inside it had aborted it",
            );
            let refusal = std::error::Error::source(&error)
                .and_then(|source| source.downcast_ref::<tokio_postgres::Error>())
                .and_then(tokio_postgres::Error::as_db_error)
                .map(|refusal| refusal.code().clone());
            assert_eq!(refusal, Some(SqlState::IN_FAILED_SQL_TRANSACTION));
        }
        other => panic!("expected Connection(TransactionAborted), got {other:?}"),
    }
}

// [spec:pgorm:sem:conn.tx.aborted/test]    the COMMIT of an aborted transaction is an error, nothing in it stands, and the connection carries on
#[pgorm_macros::test]
pub async fn an_aborted_transaction_reports_its_rollback() -> Result<(), Error> {
    let ctx = TestContext::new("aborted_transaction_rollback").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    let outcome = db
        .transaction(async |txn| {
            insert_bakery(&txn, "Lost Bakery").await?;
            fail_and_catch(&txn).await;
            Ok::<_, Error>(())
        })
        .await;
    assert_aborted(outcome);
    assert_eq!(bakeries(&db).await?, Vec::<String>::new());

    db.transaction(async |txn| insert_bakery(&txn, "Kept Bakery").await)
        .await
        .expect("the connection is left idle, so the next transaction commits");
    assert_eq!(bakeries(&db).await?, ["Kept Bakery"]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:conn.tx.aborted/test]    transaction_with and transaction_with_retry, which does not replay it
#[pgorm_macros::test]
pub async fn an_aborted_configured_transaction_reports_its_rollback() -> Result<(), Error> {
    let ctx = TestContext::new("aborted_configured_transaction").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    let mode = TransactionMode::ReadWrite {
        isolation: Some(pgorm::IsolationLevel::Serializable),
    };
    let outcome = db
        .transaction_with(mode, async |txn| {
            insert_bakery(&txn, "Lost Bakery").await?;
            fail_and_catch(&txn).await;
            Ok::<_, Error>(())
        })
        .await;
    assert_aborted(outcome);

    let mut attempts = 0;
    let outcome = db
        .transaction_with_retry(mode, 3, async |txn| {
            attempts += 1;
            insert_bakery(&txn, "Lost Bakery").await?;
            fail_and_catch(&txn).await;
            Ok::<_, Error>(())
        })
        .await;
    assert_aborted(outcome);
    assert_eq!(attempts, 1, "an aborted transaction is not retryable");
    assert_eq!(bakeries(&db).await?, Vec::<String>::new());

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:conn.tx.aborted/test]    an aborted savepoint is rolled back to, and the transaction around it commits
#[pgorm_macros::test]
pub async fn an_aborted_savepoint_rolls_back_alone() -> Result<(), Error> {
    let ctx = TestContext::new("aborted_savepoint").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;

    db.transaction(async |txn| {
        insert_bakery(&txn, "Before Bakery").await?;
        let savepoint = txn
            .transaction(async |savepoint| {
                insert_bakery(&savepoint, "Lost Bakery").await?;
                fail_and_catch(&savepoint).await;
                Ok::<_, Error>(())
            })
            .await;
        assert_aborted(savepoint);
        insert_bakery(&txn, "After Bakery").await?;
        Ok::<_, Error>(())
    })
    .await
    .expect("the transaction around the aborted savepoint commits");
    assert_eq!(bakeries(&db).await?, ["After Bakery", "Before Bakery"]);

    let outcome = db
        .transaction(async |txn| {
            insert_bakery(&txn, "Lost Bakery").await?;
            txn.transaction(async |savepoint| {
                fail_and_catch(&savepoint).await;
                Ok::<_, Error>(())
            })
            .await?;
            Ok::<_, Error>(())
        })
        .await;
    match outcome {
        Err(TransactionError::Transaction(Error::TransactionAborted(_))) => {}
        other => panic!("`?` carries the savepoint's TransactionAborted out, got {other:?}"),
    }
    assert_eq!(bakeries(&db).await?, ["After Bakery", "Before Bakery"]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:conn.tx.aborted/test]    a COMMIT that fails is still the server's error, not an aborted transaction
#[pgorm_macros::test]
pub async fn a_failed_commit_is_still_its_own_error() -> Result<(), Error> {
    let ctx = TestContext::new("aborted_failed_commit").await;
    create_tables(&ctx.db).await?;
    let mut db = ctx.db.get().await?;
    db.batch_execute("CREATE TABLE deferred_key (id integer UNIQUE DEFERRABLE INITIALLY DEFERRED)")
        .await?;

    let outcome = db
        .transaction(async |txn| {
            txn.batch_execute("INSERT INTO deferred_key VALUES (1), (1)")
                .await?;
            Ok::<_, Error>(())
        })
        .await;
    match outcome {
        Err(TransactionError::Connection(Error::Postgres(error))) => assert_eq!(
            error.as_db_error().map(|error| error.code()),
            Some(&SqlState::UNIQUE_VIOLATION),
        ),
        other => panic!("expected the COMMIT's unique violation, got {other:?}"),
    }
    let rows = db
        .query_one("SELECT count(*) FROM deferred_key", &[])
        .await?;
    assert_eq!(rows.get::<_, i64>(0), 0);

    drop(db);
    ctx.delete().await;
    Ok(())
}
