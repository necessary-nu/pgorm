#![allow(unused_imports, dead_code)]

//! A foreign key or `CHECK` added `NOT VALID`, and a `CHECK` kept from
//! inheriting tables, against a live server.
//!
//! The render tests in pgorm-query settle where each clause is written and
//! that it parses. What only a server settles is what each means: a `NOT
//! VALID` constraint leaves the rows already there unchecked
//! (`convalidated` false) while holding every new row at once, until
//! `VALIDATE CONSTRAINT` checks the rest; and a `NO INHERIT` `CHECK` is not
//! copied to a table that inherits, where a plain one is.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Check, ColumnDef, Enforcement, Expr, Name, Table, TableForeignKey, TableKey,
};
use pgorm::{ConnectionTrait, entity::prelude::*};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

/// Each named constraint of `table` with whether it is valid and whether it
/// is kept from inheriting tables, sorted by name.
async fn constraints_of(
    db: &DatabaseConnection,
    table: &str,
) -> Result<Vec<(String, bool, bool)>, Error> {
    let rows = db
        .query_all(
            "SELECT conname::text, convalidated, connoinherit FROM pg_constraint \
             WHERE conrelid = $1::text::regclass AND contype IN ('c', 'f') ORDER BY conname",
            &[&table],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

fn constraint(name: &str, valid: bool, no_inherit: bool) -> (String, bool, bool) {
    (name.to_owned(), valid, no_inherit)
}

/// A foreign key and a `CHECK` added `NOT VALID` over rows that break them:
/// the action is taken and leaves them not valid, a new breaking row is
/// refused at once (`23503`, `23514`), `VALIDATE CONSTRAINT` refuses the old
/// ones the same way, and once they are mended it makes the constraint valid.
/// Without `NOT VALID` the same action is refused over the same rows, the
/// control.
// [spec:pgorm:req:sql.ddl.alter-table+15/test]    against a live server: a foreign key and a
// CHECK added NOT VALID skip the rows there, hold new rows, and validate once mended
#[pgorm_macros::test]
async fn not_valid_skips_the_rows_already_there() -> Result<(), Error> {
    let ctx = TestContext::new("not_valid_skips_existing_rows").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        "CREATE TABLE parent (id integer PRIMARY KEY); \
         INSERT INTO parent VALUES (1); \
         CREATE TABLE child (parent_id integer, v integer); \
         INSERT INTO child VALUES (2, -1)",
    )
    .await?;
    let mut parent = TableForeignKey::new(n("child"), n("parent_id"), n("parent"), n("id"));
    parent.name(n("child_parent"));
    let positive = Check::new(Expr::col(n("v")).gt(0)).name(n("child_v_positive"));

    for (checked, state) in [
        (
            Table::alter(n("child"))
                .add_foreign_key(parent.clone())
                .to_string(),
            SqlState::FOREIGN_KEY_VIOLATION,
        ),
        (
            Table::alter(n("child"))
                .add_check(positive.clone())
                .to_string(),
            SqlState::CHECK_VIOLATION,
        ),
    ] {
        let refused = db.batch_execute(&checked).await.expect_err(&checked);
        refused_with(&refused, &state);
    }

    let added = Table::alter(n("child"))
        .add_foreign_key(parent.not_valid())
        .add_check(positive.not_valid())
        .to_string();
    db.batch_execute(&added).await?;
    assert_eq!(
        constraints_of(&db, "child").await?,
        [
            constraint("child_parent", false, true),
            constraint("child_v_positive", false, false),
        ],
        "{added}"
    );

    let refused = db
        .execute("INSERT INTO child VALUES (3, 1)", &[])
        .await
        .expect_err("a new orphan");
    refused_with(&refused, &SqlState::FOREIGN_KEY_VIOLATION);
    let refused = db
        .execute("INSERT INTO child VALUES (1, -2)", &[])
        .await
        .expect_err("a new negative v");
    refused_with(&refused, &SqlState::CHECK_VIOLATION);

    for (name, state) in [
        ("child_parent", SqlState::FOREIGN_KEY_VIOLATION),
        ("child_v_positive", SqlState::CHECK_VIOLATION),
    ] {
        let validate = Table::alter(n("child"))
            .validate_constraint(n(name))
            .to_string();
        let refused = db.batch_execute(&validate).await.expect_err(&validate);
        refused_with(&refused, &state);
    }

    db.execute("UPDATE child SET parent_id = 1, v = 1", &[])
        .await?;
    db.batch_execute(
        &Table::alter(n("child"))
            .validate_constraint(n("child_parent"))
            .validate_constraint(n("child_v_positive"))
            .to_string(),
    )
    .await?;
    assert_eq!(
        constraints_of(&db, "child").await?,
        [
            constraint("child_parent", true, true),
            constraint("child_v_positive", true, false),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A constraint that is `NOT ENFORCED` is never valid, whatever else it says,
/// and `VALIDATE CONSTRAINT` refuses it (`55000`); added `NOT VALID` as well,
/// it is accepted and stays as it was.
// [spec:pgorm:req:sql.ddl.alter-table+15/test]    against a live server: NOT VALID beside NOT
// ENFORCED is taken, and the constraint cannot be validated
#[pgorm_macros::test]
async fn not_enforced_and_not_valid_never_validates() -> Result<(), Error> {
    let ctx = TestContext::new("not_valid_not_enforced").await;
    let db = ctx.db.get().await?;
    db.batch_execute("CREATE TABLE t (v integer); INSERT INTO t VALUES (-1)")
        .await?;

    db.batch_execute(
        &Table::alter(n("t"))
            .add_check(
                Check::new(Expr::col(n("v")).gt(0))
                    .name(n("t_v_positive"))
                    .enforcement(Enforcement::NotEnforced)
                    .not_valid(),
            )
            .to_string(),
    )
    .await?;
    let validate = Table::alter(n("t"))
        .validate_constraint(n("t_v_positive"))
        .to_string();
    let refused = db.batch_execute(&validate).await.expect_err(&validate);
    refused_with(&refused, &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE);
    assert_eq!(
        constraints_of(&db, "t").await?,
        [constraint("t_v_positive", false, false)]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A `CHECK` declared `NO INHERIT` — on a column, at table level, or added
/// after — is not copied to a table that inherits, where a plain one is, so
/// the child admits the row the parent refuses. A partitioned table refuses it
/// in either statement (`42P16`), its constraints always reaching its
/// partitions.
// [spec:pgorm:req:sql.ddl.create-table+16/test]    against a live server: a NO INHERIT CHECK
// holds the parent alone, and a partitioned table refuses one
#[pgorm_macros::test]
async fn no_inherit_check_stays_with_its_table() -> Result<(), Error> {
    let ctx = TestContext::new("no_inherit_check").await;
    let db = ctx.db.get().await?;

    let positive = |column: &str| Check::new(Expr::col(n(column)).gt(0));
    let parent = Table::create(n("parent"))
        .col(
            ColumnDef::new(n("a"))
                .integer()
                .check(positive("a").name(n("parent_a_positive")).no_inherit()),
        )
        .col(ColumnDef::new(n("b")).integer())
        .col(ColumnDef::new(n("c")).integer())
        .check(positive("b").name(n("parent_b_positive")).no_inherit())
        .check(positive("c").name(n("parent_c_positive")))
        .to_string();
    db.batch_execute(&parent).await?;
    db.batch_execute(
        &Table::alter(n("parent"))
            .add_check(
                Check::new(Expr::col(n("a")).lt(100))
                    .name(n("parent_a_small"))
                    .no_inherit(),
            )
            .to_string(),
    )
    .await?;
    db.batch_execute("CREATE TABLE child () INHERITS (parent)")
        .await?;

    assert_eq!(
        constraints_of(&db, "parent").await?,
        [
            constraint("parent_a_positive", true, true),
            constraint("parent_a_small", true, true),
            constraint("parent_b_positive", true, true),
            constraint("parent_c_positive", true, false),
        ],
        "{parent}"
    );
    assert_eq!(
        constraints_of(&db, "child").await?,
        [constraint("parent_c_positive", true, false)]
    );
    db.execute("INSERT INTO child VALUES (-1, -1, 1)", &[])
        .await?;
    let refused = db
        .execute("INSERT INTO parent VALUES (-1, -1, 1)", &[])
        .await
        .expect_err("a negative a on the parent");
    refused_with(&refused, &SqlState::CHECK_VIOLATION);

    let partitioned = |check: Check| {
        let mut create = Table::create(n("ranged"));
        create
            .col(ColumnDef::new(n("v")).integer())
            .check(check)
            .raw_suffix("PARTITION BY RANGE (v)");
        create.to_string()
    };
    let refused_create = partitioned(positive("v").no_inherit());
    let refused = db
        .batch_execute(&refused_create)
        .await
        .expect_err(&refused_create);
    refused_with(&refused, &SqlState::INVALID_TABLE_DEFINITION);
    db.batch_execute(&partitioned(positive("v"))).await?;
    let refused_add = Table::alter(n("ranged"))
        .add_check(positive("v").no_inherit().not_valid())
        .to_string();
    let refused = db
        .batch_execute(&refused_add)
        .await
        .expect_err(&refused_add);
    refused_with(&refused, &SqlState::INVALID_TABLE_DEFINITION);

    drop(db);
    ctx.delete().await;
    Ok(())
}
