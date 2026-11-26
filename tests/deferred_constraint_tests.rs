#![allow(unused_imports, dead_code)]

//! `DEFERRABLE` foreign keys against a live PostgreSQL server.
//!
//! The render tests in pgorm-query settle that the three spellings parse. What
//! only a server settles is *when* the check runs, which is the whole content
//! of the clause: a cycle of rows that no single statement can complete is the
//! one observation separating `DEFERRABLE INITIALLY DEFERRED` from the default,
//! and `SET CONSTRAINTS` reaching a constraint at all is the one separating
//! `DEFERRABLE INITIALLY IMMEDIATE` from it. Both are transaction-scoped, so
//! neither shows up in rendered text.

use crate::common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{ColumnDef, Deferrability, ForeignKey, Name, Table};
use pgorm::{Connection, ConnectionMut, Transact, TransactionError, entity::prelude::*};

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("deferred_constraint_tests").await;
    let mut db = ctx.db.get().await?;

    an_immediate_key_refuses_the_cycle(&db).await?;
    a_deferred_key_admits_the_cycle(&mut db).await?;
    an_initially_immediate_key_can_still_be_deferred(&mut db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

/// A table of nodes each pointing at the next, with the self-reference added
/// as a separate constraint so the check timing is the only thing that varies.
async fn cycle_table(
    db: &DatabaseConnection,
    table: &str,
    deferrability: Deferrability,
) -> Result<(), Error> {
    let create = Table::create(Name::runtime(table))
        .col(ColumnDef::new(Name::runtime("id")).integer())
        .primary_key(Name::runtime("id"))
        .col(ColumnDef::new(Name::runtime("next")).integer())
        .to_string();
    db.batch_execute(&create).await?;

    let key = ForeignKey::create(
        Name::runtime(table),
        Name::runtime("next"),
        Name::runtime(table),
        Name::runtime("id"),
    )
    .name(Name::runtime(format!("{table}_next_fk")))
    .deferrability(deferrability)
    .to_string();
    db.batch_execute(&key).await
}

/// The two inserts that only a deferred check accepts: each row names the
/// other, so whichever lands first names a row that does not exist yet.
async fn insert_cycle<C: ConnectionMut>(db: &C, table: &str) -> Result<(), Error> {
    db.execute(
        &format!("INSERT INTO {table} (id, next) VALUES (1, 2)"),
        &[],
    )
    .await?;
    db.execute(
        &format!("INSERT INTO {table} (id, next) VALUES (2, 1)"),
        &[],
    )
    .await?;
    Ok(())
}

/// The control: rendered without the clause, the key is `NOT DEFERRABLE` by
/// the server's own default, and the first insert of the pair fails.
// [spec:pgorm:req:sql.ddl.foreign-key+11/test]
// [spec:pgorm:req:sql.scope+25/test]
async fn an_immediate_key_refuses_the_cycle(db: &DatabaseConnection) -> Result<(), Error> {
    cycle_table(db, "immediate_node", Deferrability::NotDeferrable).await?;

    let refused = insert_cycle(db, "immediate_node").await;
    let message = refused
        .expect_err("a NOT DEFERRABLE key admitted a forward reference")
        .to_string();
    assert!(
        message.contains("foreign key") || message.contains("violates"),
        "unexpected refusal: {message}"
    );

    Ok(())
}

/// `INITIALLY DEFERRED`: the same two statements commit, because the check
/// runs once at the end rather than after each.
// [spec:pgorm:req:sql.ddl.foreign-key+11/test]
// [spec:pgorm:req:sql.scope+25/test]
async fn a_deferred_key_admits_the_cycle(db: &mut DatabaseConnection) -> Result<(), Error> {
    cycle_table(
        db,
        "deferred_node",
        Deferrability::DeferrableInitiallyDeferred,
    )
    .await?;

    db.transaction(async |txn| insert_cycle(&txn, "deferred_node").await)
        .await
        .expect("the deferred cycle commits");

    let row = db
        .query_one("SELECT count(*) FROM deferred_node", &[])
        .await?;
    assert_eq!(row.get::<_, i64>(0), 2, "both rows survived the commit");

    Ok(())
}

/// `INITIALLY IMMEDIATE` is the third state, not a synonym for the default: it
/// refuses the cycle on its own, and accepts it once the transaction moves the
/// check with `SET CONSTRAINTS`. A `NOT DEFERRABLE` key cannot be moved that
/// way at all, so the pair of answers below is what tells them apart.
// [spec:pgorm:req:sql.ddl.foreign-key+11/test]
// [spec:pgorm:req:sql.scope+25/test]
async fn an_initially_immediate_key_can_still_be_deferred(
    db: &mut DatabaseConnection,
) -> Result<(), Error> {
    cycle_table(
        db,
        "immediate_deferrable_node",
        Deferrability::DeferrableInitiallyImmediate,
    )
    .await?;

    let refused = db
        .transaction(async |txn| insert_cycle(&txn, "immediate_deferrable_node").await)
        .await;
    assert!(
        matches!(refused, Err(TransactionError::Transaction(_))),
        "INITIALLY IMMEDIATE checked at commit rather than per statement"
    );

    db.transaction(async |txn| {
        txn.execute(
            "SET CONSTRAINTS immediate_deferrable_node_next_fk DEFERRED",
            &[],
        )
        .await?;
        insert_cycle(&txn, "immediate_deferrable_node").await
    })
    .await
    .expect("the cycle commits once the check is deferred");

    let row = db
        .query_one("SELECT count(*) FROM immediate_deferrable_node", &[])
        .await?;
    assert_eq!(row.get::<_, i64>(0), 2);

    Ok(())
}
