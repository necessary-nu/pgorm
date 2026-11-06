#![allow(unused_imports, dead_code)]

//! PostgreSQL 18's `NOT ENFORCED` foreign keys and `CHECK` constraints
//! against a live server.
//!
//! The render tests in pgorm-query settle that each spelling parses. What only
//! a server settles is what the clause means — a constraint recorded in
//! `pg_constraint` with `conenforced` false and never checked — and what it
//! refuses around one.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Check, ColumnDef, ConstraintChange, Deferrability, Enforcement, Expr, ForeignKey,
    ForeignKeyAction, Name, Table, TableForeignKey,
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

/// One constraint as `pg_constraint` records it: its name, kind, whether it
/// is enforced and whether it is valid.
async fn constraints(
    db: &DatabaseConnection,
    table: &str,
) -> Result<Vec<(String, String, bool, bool)>, Error> {
    let rows = db
        .query_all(
            "SELECT conname::text, contype::text, conenforced, convalidated FROM pg_constraint \
             WHERE conrelid = $1::text::regclass AND contype IN ('c', 'f') ORDER BY conname",
            &[&table],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect())
}

fn row(name: &str, kind: &str, enforced: bool, valid: bool) -> (String, String, bool, bool) {
    (name.to_owned(), kind.to_owned(), enforced, valid)
}

async fn parent(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(
        "CREATE TABLE parent (id integer PRIMARY KEY); INSERT INTO parent VALUES (1), (2)",
    )
    .await?;
    Ok(())
}

/// A `NOT ENFORCED` foreign key or `CHECK` is recorded, never valid, and lets
/// through every row that breaks it; an explicit `ENFORCED` is the default.
/// A foreign key that is not enforced has no referential triggers, so its
/// `ON DELETE` action never fires either.
// [spec:pgorm:req:sql.ddl.enforcement/test]    NOT ENFORCED is recorded and not
// checked, at column and table level, and ENFORCED is the default
#[pgorm_macros::test]
async fn not_enforced_constraints_hold_no_row() -> Result<(), Error> {
    let ctx = TestContext::new("not_enforced_hold_no_row").await;
    let db = ctx.db.get().await?;
    parent(&db).await?;

    let create = Table::create(n("child"))
        .col(ColumnDef::new(n("parent_id")).integer())
        .col(
            ColumnDef::new(n("amount")).integer().check(
                Check::new(Expr::col(n("amount")).gt(0))
                    .name(n("amount positive"))
                    .enforcement(Enforcement::NotEnforced),
            ),
        )
        .col(ColumnDef::new(n("other_id")).integer())
        .foreign_key(
            ForeignKey::create(n("child"), n("parent_id"), n("parent"), n("id"))
                .name(n("child parent"))
                .on_delete(ForeignKeyAction::Cascade)
                .enforcement(Enforcement::NotEnforced)
                .to_owned(),
        )
        .foreign_key(
            ForeignKey::create(n("child"), n("other_id"), n("parent"), n("id"))
                .name(n("child other"))
                .enforcement(Enforcement::Enforced)
                .to_owned(),
        )
        .check(
            Check::new(Expr::col(n("amount")).lt(1000))
                .name(n("amount small"))
                .enforcement(Enforcement::Enforced),
        )
        .to_string();
    db.batch_execute(&create).await?;
    assert_eq!(
        constraints(&db, "child").await?,
        [
            row("amount positive", "c", false, false),
            row("amount small", "c", true, true),
            row("child other", "f", true, true),
            row("child parent", "f", false, false),
        ]
    );

    db.batch_execute("INSERT INTO child VALUES (99, -5, 2), (1, 3, 2)")
        .await?;
    db.batch_execute("DELETE FROM parent WHERE id = 1").await?;
    let kept = db
        .query_all("SELECT parent_id FROM child ORDER BY parent_id", &[])
        .await?;
    assert_eq!(
        kept.iter().map(|row| row.get(0)).collect::<Vec<i32>>(),
        [1, 99],
        "a NOT ENFORCED foreign key neither refuses an orphan nor cascades"
    );
    let enforced = db
        .batch_execute("INSERT INTO child VALUES (2, 5000, 2)")
        .await
        .expect_err("an ENFORCED CHECK holds");
    refused_with(&enforced, &SqlState::CHECK_VIOLATION);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `ALTER CONSTRAINT ... ENFORCED` checks every row a foreign key let through
/// while it was not enforced, refusing the statement while one breaks it, and
/// `NOT ENFORCED` leaves it unchecked and not valid again. A constraint added
/// `NOT ENFORCED` to a table already holding rows that break it is accepted.
// [spec:pgorm:req:sql.ddl.alter-table+15/test]    a foreign key and a CHECK are
// added NOT ENFORCED, and ALTER CONSTRAINT moves the foreign key between the two
// [spec:pgorm:req:sql.ddl.enforcement/test]    ENFORCED validates as it goes, and
// NOT ENFORCED clears validity
#[pgorm_macros::test]
async fn alter_constraint_enforces_a_foreign_key() -> Result<(), Error> {
    let ctx = TestContext::new("not_enforced_alter").await;
    let db = ctx.db.get().await?;
    parent(&db).await?;
    db.batch_execute("CREATE TABLE child (parent_id integer); INSERT INTO child VALUES (99)")
        .await?;

    let added = Table::alter(n("child"))
        .add_foreign_key(
            TableForeignKey::new(n("child"), n("parent_id"), n("parent"), n("id"))
                .name(n("child parent"))
                .enforcement(Enforcement::NotEnforced)
                .to_owned(),
        )
        .add_check(
            Check::new(Expr::col(n("parent_id")).lt(10))
                .name(n("small parent"))
                .enforcement(Enforcement::NotEnforced),
        )
        .to_string();
    assert_eq!(
        added,
        [
            r#"ALTER TABLE "child" ADD CONSTRAINT "child parent""#,
            r#"FOREIGN KEY ("parent_id") REFERENCES "parent" ("id") NOT ENFORCED,"#,
            r#"ADD CONSTRAINT "small parent" CHECK ("parent_id" < 10) NOT ENFORCED"#,
        ]
        .join(" ")
    );
    db.batch_execute(&added).await?;

    let enforce = Table::alter(n("child"))
        .alter_constraint(n("child parent"), ConstraintChange::Enforced)
        .to_string();
    let orphan = db
        .batch_execute(&enforce)
        .await
        .expect_err("the orphan is checked");
    refused_with(&orphan, &SqlState::FOREIGN_KEY_VIOLATION);

    db.batch_execute("UPDATE child SET parent_id = 1").await?;
    db.batch_execute(&enforce).await?;
    assert_eq!(
        constraints(&db, "child").await?,
        [
            row("child parent", "f", true, true),
            row("small parent", "c", false, false),
        ]
    );
    let refused = db
        .batch_execute("INSERT INTO child VALUES (42)")
        .await
        .expect_err("an enforced foreign key holds");
    refused_with(&refused, &SqlState::FOREIGN_KEY_VIOLATION);

    db.batch_execute(
        &Table::alter(n("child"))
            .alter_constraint(n("child parent"), ConstraintChange::NotEnforced)
            .to_string(),
    )
    .await?;
    db.batch_execute("INSERT INTO child VALUES (42)").await?;
    assert_eq!(
        constraints(&db, "child").await?.first(),
        Some(&row("child parent", "f", false, false))
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// On PostgreSQL 19 `ALTER CONSTRAINT` moves a `CHECK` between enforced and
/// not, as it moves a foreign key; 18 refuses a `CHECK`'s (`42809`), which
/// `enforcement_refusals_by_sqlstate` holds in the default build.
/// `ENFORCED` checks the rows already there, refusing the statement while one
/// breaks the condition (`23514`), and the constraint is valid once they
/// pass; `NOT ENFORCED` leaves it unchecked and not valid again.
// [spec:pgorm:req:sql.ddl.alter-table+15/test]    ENFORCED and NOT ENFORCED apply
// to a CHECK as to a foreign key, from PostgreSQL 19
// [spec:pgorm:req:sql.ddl.enforcement/test]    ALTER CONSTRAINT moves a CHECK
// between the two on 19, ENFORCED checking the rows as it goes
// [spec:pgorm:req:sql.target+2/test]    19's answer, built and run under pg-19 alone
#[cfg(feature = "pg-19")]
#[pgorm_macros::test]
async fn alter_constraint_enforces_a_check() -> Result<(), Error> {
    let ctx = TestContext::new("not_enforced_alter_check").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        &Table::create(n("reading"))
            .col(ColumnDef::new(n("amount")).integer())
            .check(
                Check::new(Expr::col(n("amount")).gt(0))
                    .name(n("positive"))
                    .enforcement(Enforcement::NotEnforced),
            )
            .to_string(),
    )
    .await?;
    db.batch_execute("INSERT INTO reading VALUES (-5)").await?;

    let alter = |change| {
        Table::alter(n("reading"))
            .alter_constraint(n("positive"), change)
            .to_string()
    };
    let broken = db
        .batch_execute(&alter(ConstraintChange::Enforced))
        .await
        .expect_err("the row already there is checked");
    refused_with(&broken, &SqlState::CHECK_VIOLATION);
    assert_eq!(
        constraints(&db, "reading").await?,
        [row("positive", "c", false, false)]
    );

    db.batch_execute("UPDATE reading SET amount = 5").await?;
    for (change, enforced) in [
        (ConstraintChange::Enforced, true),
        (ConstraintChange::NotEnforced, false),
    ] {
        db.batch_execute(&alter(change)).await?;
        assert_eq!(
            constraints(&db, "reading").await?,
            [row("positive", "c", enforced, enforced)]
        );
        match db.batch_execute("INSERT INTO reading VALUES (-1)").await {
            Err(error) if enforced => refused_with(&error, &SqlState::CHECK_VIOLATION),
            inserted => assert_eq!(inserted.is_ok(), !enforced, "{inserted:?}"),
        }
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// What the server refuses around enforcement, by SQLSTATE: altering a key's
/// enforcement, or on PostgreSQL 18 a `CHECK`'s (19 takes it:
/// `alter_constraint_enforces_a_check`), validating a constraint that is not
/// enforced, and deferring a `CHECK` whether or not it is enforced. A foreign
/// key that is not enforced may still be deferrable.
// [spec:pgorm:req:sql.ddl.enforcement/test]    the refusals around NOT ENFORCED,
// and a NOT ENFORCED foreign key taking deferrability
// [spec:pgorm:req:sql.target+2/test]    18's refusal of a CHECK's enforcement,
// held in the default build only
#[pgorm_macros::test]
async fn enforcement_refusals_by_sqlstate() -> Result<(), Error> {
    let ctx = TestContext::new("not_enforced_refusals").await;
    let db = ctx.db.get().await?;
    parent(&db).await?;
    db.batch_execute(
        &Table::create(n("child"))
            .col(ColumnDef::new(n("parent_id")).integer())
            .foreign_key(
                ForeignKey::create(n("child"), n("parent_id"), n("parent"), n("id"))
                    .name(n("child parent"))
                    .deferrability(Deferrability::DeferrableInitiallyDeferred)
                    .enforcement(Enforcement::NotEnforced)
                    .to_owned(),
            )
            .check(
                Check::new(Expr::col(n("parent_id")).gt(0))
                    .name(n("positive"))
                    .enforcement(Enforcement::NotEnforced),
            )
            .to_string(),
    )
    .await?;
    let deferred = db
        .query_one(
            "SELECT condeferrable, condeferred, conenforced FROM pg_constraint \
             WHERE conname = 'child parent'",
            &[],
        )
        .await?;
    assert_eq!(
        (
            deferred.get::<_, bool>(0),
            deferred.get::<_, bool>(1),
            deferred.get::<_, bool>(2)
        ),
        (true, true, false)
    );

    let alter = || Table::alter(n("child"));
    let refused = async |sql: String, state: &SqlState| {
        let error = db.batch_execute(&sql).await.expect_err(&sql);
        refused_with(&error, state);
    };
    for change in [ConstraintChange::Enforced, ConstraintChange::NotEnforced] {
        #[cfg(not(feature = "pg-19"))]
        refused(
            alter().alter_constraint(n("positive"), change).to_string(),
            &SqlState::WRONG_OBJECT_TYPE,
        )
        .await;
        refused(
            Table::alter(n("parent"))
                .alter_constraint(n("parent_pkey"), change)
                .to_string(),
            &SqlState::WRONG_OBJECT_TYPE,
        )
        .await;
    }
    for name in ["child parent", "positive"] {
        refused(
            alter().validate_constraint(n(name)).to_string(),
            &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE,
        )
        .await;
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}
