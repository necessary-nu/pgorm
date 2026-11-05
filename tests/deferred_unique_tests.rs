#![allow(unused_imports, dead_code)]

//! `DEFERRABLE` unique and primary-key constraints against a live PostgreSQL
//! server.
//!
//! The render tests in pgorm-query hold each spelling to libpg_query's
//! `Constraint` node. What only a server settles is *when* the uniqueness
//! check runs — per row, at the end of the statement, or at `COMMIT` — and
//! that is the whole content of the clause. Each case below makes a state that
//! one timing admits and another refuses, beside the control that shows the
//! refusal, so a clause that rendered and was then ignored still fails.
//!
//! It also holds the two refusals the API is shaped around: PostgreSQL takes
//! no `DEFERRABLE` on a `CHECK` constraint or on a standalone
//! `CREATE UNIQUE INDEX`, and a deferrable constraint cannot arbitrate an
//! `ON CONFLICT`.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    ColumnDef, Deferrability, Index, Name, OnConflict, Query, Table, TableKey, Unique,
};
use pgorm::{ConnectionTrait, TransactionTrait, entity::prelude::*};
use tokio_postgres::error::SqlState;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("deferred_unique_tests").await;
    let mut db = ctx.db.get().await?;

    a_deferred_unique_key_admits_a_transient_duplicate(&mut db).await?;
    set_constraints_immediate_fires_the_check_early(&mut db).await?;
    a_deferred_primary_key_is_checked_at_commit(&mut db).await?;
    an_initially_immediate_key_checks_at_statement_end(&db).await?;
    a_deferrable_key_cannot_arbitrate_on_conflict(&db).await?;
    check_constraints_and_standalone_indexes_refuse_deferral(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// `CREATE TABLE <table> (id int, position int NOT NULL, PRIMARY KEY (id)[,
/// <unique>])` over positions 1 and 2, the position's uniqueness, if any,
/// carried by `unique`.
async fn slots(
    db: &DatabaseConnection,
    table: &str,
    unique: Option<TableKey<Unique>>,
) -> Result<(), Error> {
    let mut create = Table::create(Name::runtime(table));
    create
        .col(ColumnDef::new(Name::runtime("id")).integer())
        .col(
            ColumnDef::new(Name::runtime("position"))
                .integer()
                .not_null(),
        )
        .primary_key(Name::runtime("id"));
    if let Some(unique) = unique {
        create.unique(unique);
    }
    let create = create.to_string();
    db.batch_execute(&create).await?;
    db.execute(
        &format!("INSERT INTO {table} (id, position) VALUES (1, 1), (2, 2)"),
        &[],
    )
    .await?;
    Ok(())
}

/// Moves slot 2 onto position 1, which duplicates it until slot 1 moves off.
async fn collide<C: ConnectionTrait>(db: &C, table: &str) -> Result<u64, Error> {
    db.execute(
        &format!("UPDATE {table} SET position = 1 WHERE id = 2"),
        &[],
    )
    .await
}

/// A `UNIQUE (…) DEFERRABLE INITIALLY DEFERRED` lets a transaction
/// hold a duplicate between statements — the swap of two positions, which no
/// single `UPDATE` ordering can do through a per-row check — and refuses one
/// that is still there at `COMMIT`. The control, the same column with no
/// clause, refuses the first half of the swap on the spot.
// [spec:pgorm:req:sql.ddl.deferrability+4/test]    against a live server: an initially
// deferred unique key is checked at COMMIT
// [spec:pgorm:req:sql.ddl.create-table+16/test]
// [spec:pgorm:req:sql.scope+23/test]
async fn a_deferred_unique_key_admits_a_transient_duplicate(
    db: &mut DatabaseConnection,
) -> Result<(), Error> {
    let position = || TableKey::new(Name::runtime("position"));
    slots(db, "immediate_slot", Some(position())).await?;
    let refused = collide(db, "immediate_slot")
        .await
        .expect_err("the default key refuses the duplicate per row");
    refused_with(&refused, &SqlState::UNIQUE_VIOLATION);

    slots(
        db,
        "deferred_slot",
        Some(position().deferrability(Deferrability::DeferrableInitiallyDeferred)),
    )
    .await?;

    db.transaction(async |txn| {
        collide(&txn, "deferred_slot").await?;
        txn.execute("UPDATE deferred_slot SET position = 2 WHERE id = 1", &[])
            .await
    })
    .await
    .expect("the swap commits");
    let row = db
        .query_one("SELECT position FROM deferred_slot WHERE id = 1", &[])
        .await?;
    assert_eq!(row.get::<_, i32>(0), 2, "the swap committed");

    let refused = commit_refused(
        db.transaction(async |txn| {
            txn.execute("UPDATE deferred_slot SET position = 2 WHERE id = 2", &[])
                .await
        })
        .await,
    );
    refused_with(&refused, &SqlState::UNIQUE_VIOLATION);

    Ok(())
}

/// `SET CONSTRAINTS <name> IMMEDIATE` runs a deferred check on the spot, at
/// the statement that says so, rather than waiting for `COMMIT`. The key is
/// named, so `SET CONSTRAINTS` has a name to reach it by.
// [spec:pgorm:req:sql.ddl.deferrability+4/test]    against a live server: SET CONSTRAINTS moves
// a deferrable key's check
// [spec:pgorm:req:sql.ddl.create-table+16/test]
async fn set_constraints_immediate_fires_the_check_early(
    db: &mut DatabaseConnection,
) -> Result<(), Error> {
    let create = Table::create(Name::runtime("named_slot"))
        .col(ColumnDef::new(Name::runtime("id")).integer())
        .primary_key(Name::runtime("id"))
        .col(
            ColumnDef::new(Name::runtime("position"))
                .integer()
                .not_null(),
        )
        .unique(
            TableKey::new(Name::runtime("position"))
                .name(Name::runtime("named_slot_position"))
                .deferrability(Deferrability::DeferrableInitiallyDeferred),
        )
        .to_string();
    db.batch_execute(&create).await?;
    db.execute(
        "INSERT INTO named_slot (id, position) VALUES (1, 1), (2, 2)",
        &[],
    )
    .await?;

    let outcome = db
        .transaction(async |txn| {
            collide(&txn, "named_slot").await?;
            let refused = txn
                .execute("SET CONSTRAINTS named_slot_position IMMEDIATE", &[])
                .await
                .expect_err("the deferred duplicate is checked when made immediate");
            refused_with(&refused, &SqlState::UNIQUE_VIOLATION);
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);

    Ok(())
}

/// A primary key takes the same clause: two rows swap ids inside a
/// transaction, and a duplicate id left at `COMMIT` is refused.
// [spec:pgorm:req:sql.ddl.deferrability+4/test]    against a live server: a deferred primary key
// behaves as a deferred unique key does
// [spec:pgorm:req:sql.scope+23/test]
async fn a_deferred_primary_key_is_checked_at_commit(
    db: &mut DatabaseConnection,
) -> Result<(), Error> {
    let create = Table::create(Name::runtime("seat"))
        .col(ColumnDef::new(Name::runtime("id")).integer().not_null())
        .col(ColumnDef::new(Name::runtime("holder")).text().not_null())
        .primary_key(
            TableKey::new(Name::runtime("id"))
                .deferrability(Deferrability::DeferrableInitiallyDeferred),
        )
        .to_string();
    db.batch_execute(&create).await?;
    db.execute(
        "INSERT INTO seat (id, holder) VALUES (1, 'a'), (2, 'b')",
        &[],
    )
    .await?;

    db.transaction(async |txn| {
        txn.execute("UPDATE seat SET id = 1 WHERE holder = 'b'", &[])
            .await?;
        txn.execute("UPDATE seat SET id = 2 WHERE holder = 'a'", &[])
            .await
    })
    .await
    .expect("the swap commits");
    let row = db
        .query_one("SELECT holder FROM seat WHERE id = 1", &[])
        .await?;
    assert_eq!(row.get::<_, String>(0), "b", "the ids swapped");

    let refused = commit_refused(
        db.transaction(async |txn| txn.execute("UPDATE seat SET id = 1", &[]).await)
            .await,
    );
    refused_with(&refused, &SqlState::UNIQUE_VIOLATION);

    Ok(())
}

/// `DEFERRABLE INITIALLY IMMEDIATE` is not the default under another name.
/// A `NOT DEFERRABLE` unique key is checked row by row, so shifting every
/// position up by one collides with the next row before it has moved; an
/// initially-immediate one is checked once the statement ends, when the
/// positions are distinct again. The key here is added by `ALTER TABLE`'s
/// `ADD UNIQUE (…)`.
// [spec:pgorm:req:sql.ddl.deferrability+4/test]    against a live server: INITIALLY IMMEDIATE is
// checked at the end of the statement, NOT DEFERRABLE per row
// [spec:pgorm:req:sql.ddl.alter-table+15/test]
async fn an_initially_immediate_key_checks_at_statement_end(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let shift = |table: &str| format!("UPDATE {table} SET position = position + 1");

    slots(
        db,
        "row_checked_slot",
        Some(TableKey::new(Name::runtime("position"))),
    )
    .await?;
    let refused = db
        .execute(&shift("row_checked_slot"), &[])
        .await
        .expect_err("slot 1 lands on slot 2 before slot 2 moves");
    refused_with(&refused, &SqlState::UNIQUE_VIOLATION);

    slots(db, "statement_checked_slot", None).await?;
    let alter = Table::alter(Name::runtime("statement_checked_slot"))
        .add_unique(
            TableKey::new(Name::runtime("position"))
                .deferrability(Deferrability::DeferrableInitiallyImmediate),
        )
        .to_string();
    assert!(alter.ends_with("DEFERRABLE INITIALLY IMMEDIATE"), "{alter}");
    db.batch_execute(&alter).await?;
    assert_eq!(db.execute(&shift("statement_checked_slot"), &[]).await?, 2);

    Ok(())
}

/// A deferrable key cannot arbitrate `ON CONFLICT`: the server cannot tell
/// whether a row conflicts while the check that would say so may still be
/// pending (`55000`). This holds for `INITIALLY IMMEDIATE` as well as
/// `INITIALLY DEFERRED`, and it is the cost of the clause — an upsert over the
/// same column works against the undeferrable control.
// [spec:pgorm:req:sql.ddl.deferrability+4/test]    against a live server: a deferrable key is
// refused as an ON CONFLICT arbiter
async fn a_deferrable_key_cannot_arbitrate_on_conflict(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let position = || TableKey::new(Name::runtime("position"));
    slots(db, "upsert_slot", Some(position())).await?;
    slots(
        db,
        "deferrable_upsert_slot",
        Some(position().deferrability(Deferrability::DeferrableInitiallyImmediate)),
    )
    .await?;

    let upsert = |table: &'static str| {
        Query::insert()
            .into_table(Name::runtime(table))
            .columns([Name::runtime("id"), Name::runtime("position")])
            .values_panic([3.into(), 1.into()])
            .on_conflict(OnConflict::column(Name::runtime("position")).do_nothing())
            .to_string()
    };

    assert_eq!(
        db.execute(upsert("upsert_slot").as_str(), &[]).await?,
        0,
        "the conflict on position 1 is absorbed"
    );
    let refused = db
        .execute(upsert("deferrable_upsert_slot").as_str(), &[])
        .await
        .expect_err("a deferrable arbiter");
    refused_with(&refused, &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE);

    Ok(())
}

/// The two positions the API offers no deferrability in, and why: a table
/// `CHECK` refuses the clause in the grammar (`0A000`), a column `CHECK`
/// refuses it as misplaced (`42601`), and `CREATE UNIQUE INDEX` has no place
/// for it at all (`42601`). None of the three is spelled by the builder, so
/// the SQL here is written out by hand.
// [spec:pgorm:req:sql.ddl.deferrability+4/test]    against a live server: CHECK and a standalone
// index refuse deferrability
async fn check_constraints_and_standalone_indexes_refuse_deferral(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    for (sql, state) in [
        (
            "CREATE TABLE checked (n int, CHECK (n > 0) DEFERRABLE)",
            SqlState::FEATURE_NOT_SUPPORTED,
        ),
        (
            "CREATE TABLE checked (n int CHECK (n > 0) DEFERRABLE)",
            SqlState::SYNTAX_ERROR,
        ),
        (
            "CREATE UNIQUE INDEX slot_position ON upsert_slot (position) DEFERRABLE",
            SqlState::SYNTAX_ERROR,
        ),
    ] {
        let refused = db.batch_execute(sql).await.expect_err(sql);
        refused_with(&refused, &state);
    }

    Ok(())
}
