#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::entity::prelude::*;
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

const NO_PARAMS: [&(dyn tokio_postgres::types::ToSql + Sync); 0] = [];

const TWO_TABLES: &str = "CREATE TABLE batch_left (id int primary key);
     CREATE TABLE batch_right (id int primary key);";

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("execute_unprepared_tests").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    execute_raw_statements(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

pub async fn execute_raw_statements(db: &DatabaseConnection) -> Result<(), Error> {
    use insert_default::*;

    db.execute_raw(
        "INSERT INTO insert_default (id) VALUES (1), (2), (3), (4), (5)",
        NO_PARAMS,
    )
    .await?;

    db.execute_raw("DELETE FROM insert_default WHERE id % 2 = 0", NO_PARAMS)
        .await?;

    assert_eq!(
        Entity::find().all(db).await?,
        [Model { id: 1 }, Model { id: 3 }, Model { id: 5 }]
    );

    Ok(())
}

async fn ids<C: ConnectionTrait>(db: &C, table: &str) -> Result<Vec<i32>, Error> {
    let rows = db
        .query_all(&format!("SELECT id FROM {table} ORDER BY id"), &[])
        .await?;

    Ok(rows.iter().map(|row| row.get(0)).collect())
}

// [spec:pgorm:def:conn.pool.conn-trait+10/test]    multi-statement string on a pooled connection
#[pgorm_macros::test]
async fn batch_execute_on_connection() -> Result<(), Error> {
    let ctx = TestContext::new("batch_execute_conn_txbatch").await;
    let db = ctx.db.get().await?;

    db.batch_execute(&format!(
        "{TWO_TABLES}
         INSERT INTO batch_left (id) VALUES (1), (2);
         INSERT INTO batch_right (id) VALUES (3);"
    ))
    .await?;

    assert_eq!(ids(&db, "batch_left").await?, [1, 2]);
    assert_eq!(ids(&db, "batch_right").await?, [3]);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:conn.pool.conn-trait+10/test]    multi-statement string inside a transaction
#[pgorm_macros::test]
async fn batch_execute_in_transaction() -> Result<(), Error> {
    let ctx = TestContext::new("batch_execute_txn_txbatch").await;
    let mut db = ctx.db.get().await?;

    db.transaction(async |txn| {
        txn.batch_execute(&format!(
            "{TWO_TABLES}
             INSERT INTO batch_left (id) VALUES (4);
             INSERT INTO batch_right (id) VALUES (5), (6);"
        ))
        .await?;

        assert_eq!(ids(&txn, "batch_left").await?, [4]);
        Ok::<_, Error>(())
    })
    .await
    .expect("the transaction commits");

    assert_eq!(ids(&db, "batch_left").await?, [4]);
    assert_eq!(ids(&db, "batch_right").await?, [5, 6]);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:conn.pool.conn-trait+10/test]    the rejection batch_execute exists to lift
#[pgorm_macros::test]
async fn multi_statement_string_needs_batch_execute() -> Result<(), Error> {
    let ctx = TestContext::new("batch_execute_two_commands_txbatch").await;
    let db = ctx.db.get().await?;

    let error = db
        .execute(TWO_TABLES, &[])
        .await
        .expect_err("a prepared statement carries exactly one command");

    match &error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(&SqlState::SYNTAX_ERROR)),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }

    db.batch_execute(TWO_TABLES).await?;

    assert!(ids(&db, "batch_left").await?.is_empty());
    assert!(ids(&db, "batch_right").await?.is_empty());

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:conn.pool.conn-trait+10/test]    one implicit transaction per string
#[pgorm_macros::test]
async fn batch_execute_unwinds_at_first_failure() -> Result<(), Error> {
    let ctx = TestContext::new("batch_execute_implicit_txn_txbatch").await;
    let db = ctx.db.get().await?;

    db.batch_execute(TWO_TABLES).await?;

    db.batch_execute(
        "INSERT INTO batch_left (id) VALUES (7);
         INSERT INTO batch_left (id) VALUES (7);
         INSERT INTO batch_right (id) VALUES (8);",
    )
    .await
    .expect_err("the second insert violates the primary key");

    assert!(
        ids(&db, "batch_left").await?.is_empty(),
        "the statement before the failing one is discarded with it"
    );
    assert!(ids(&db, "batch_right").await?.is_empty());

    drop(db);
    ctx.delete().await;

    Ok(())
}
