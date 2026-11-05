#![allow(unused_imports, dead_code)]

//! PostgreSQL 18's `NOT NULL` constraint against a live server: named, kept
//! from inheriting tables, added `NOT VALID` and validated later.
//!
//! The render tests in pgorm-query settle that each spelling parses. What only
//! a server settles is what each becomes — the `pg_constraint` row of kind `n`
//! under the name given or one derived, `connoinherit` and `convalidated` —
//! and what it refuses around one.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    ColumnDef, ConstraintChange, Expr, Name, NotNullConstraint, Table, TableKey,
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

/// One `NOT NULL` constraint as `pg_constraint` records it.
#[derive(Debug, PartialEq, Eq)]
struct Recorded {
    column: String,
    name: String,
    valid: bool,
    no_inherit: bool,
}

fn recorded(column: &str, name: &str, valid: bool, no_inherit: bool) -> Recorded {
    Recorded {
        column: column.to_owned(),
        name: name.to_owned(),
        valid,
        no_inherit,
    }
}

/// Every `NOT NULL` constraint on `table`, by the column it holds to.
async fn not_nulls(db: &DatabaseConnection, table: &str) -> Result<Vec<Recorded>, Error> {
    let rows = db
        .query_all(
            "SELECT a.attname::text, c.conname::text, c.convalidated, c.connoinherit \
             FROM pg_constraint c JOIN pg_attribute a \
               ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1] \
             WHERE c.conrelid = $1::text::regclass AND c.contype = 'n' ORDER BY a.attnum",
            &[&table],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| Recorded {
            column: row.get(0),
            name: row.get(1),
            valid: row.get(2),
            no_inherit: row.get(3),
        })
        .collect())
}

/// Each spelling of a column's `NOT NULL` is one catalog constraint: unnamed
/// under the name the server derives, named under its own, and `NO INHERIT`
/// where the column said so. A column that set its `NOT NULL` several times
/// is created with the one constraint its last name gave, which a second
/// clause naming it differently would have been refused for.
// [spec:pgorm:req:sql.ddl.column-def+13/test]    every spelling of a column's NOT
// NULL is one pg_constraint row, under its own name or a derived one, and a
// table-level NOT VALID in CREATE TABLE is created valid
#[pgorm_macros::test]
async fn named_not_null_reaches_the_catalog() -> Result<(), Error> {
    let ctx = TestContext::new("not_null_named_catalog").await;
    let db = ctx.db.get().await?;

    let create = Table::create(n("reading"))
        .col(ColumnDef::new(n("plain")).integer().not_null())
        .col(
            ColumnDef::new(n("named"))
                .integer()
                .not_null_named(n("named present")),
        )
        .col(ColumnDef::new(n("kept")).integer().not_null_no_inherit())
        .col(
            ColumnDef::new(n("both"))
                .integer()
                .not_null()
                .default(0)
                .not_null_named(n("first"))
                .not_null_no_inherit()
                .not_null_named(n("both present")),
        )
        .to_string();
    db.batch_execute(&create).await?;
    db.batch_execute(
        &Table::alter(n("reading"))
            .add_column(
                ColumnDef::new(n("added"))
                    .integer()
                    .default(1)
                    .not_null_named(n("added present")),
            )
            .to_string(),
    )
    .await?;

    assert_eq!(
        not_nulls(&db, "reading").await?,
        [
            recorded("plain", "reading_plain_not_null", true, false),
            recorded("named", "named present", true, false),
            recorded("kept", "reading_kept_not_null", true, true),
            recorded("both", "both present", true, true),
            recorded("added", "added present", true, false),
        ]
    );
    let refused = db
        .batch_execute("INSERT INTO reading (plain, named, kept) VALUES (1, NULL, 1)")
        .await
        .expect_err("a named NOT NULL holds");
    refused_with(&refused, &SqlState::NOT_NULL_VIOLATION);

    // The table-level spelling the builder leaves out of CREATE TABLE adds
    // nothing there: its NOT VALID is created valid.
    db.batch_execute("CREATE TABLE fresh (a integer, NOT NULL a NOT VALID)")
        .await?;
    assert_eq!(
        not_nulls(&db, "fresh").await?,
        [recorded("a", "fresh_a_not_null", true, false)]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A `NOT NULL` added `NOT VALID` holds new rows at once and leaves the rows
/// already there until it is validated, which a remaining null refuses.
/// `SET NOT NULL`, by contrast, validates the not-valid constraint it finds.
// [spec:pgorm:req:sql.ddl.alter-table+15/test]    NOT VALID leaves existing rows
// for VALIDATE CONSTRAINT, and SET NOT NULL validates what it finds
#[pgorm_macros::test]
async fn not_valid_not_null_waits_for_validation() -> Result<(), Error> {
    let ctx = TestContext::new("not_null_not_valid").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        "CREATE TABLE ledger (id integer, memo text, tag text); \
         INSERT INTO ledger VALUES (1, NULL, NULL)",
    )
    .await?;

    let added = Table::alter(n("ledger"))
        .add_not_null(
            NotNullConstraint::new(n("memo"))
                .name(n("memo present"))
                .not_valid(),
        )
        .add_not_null(NotNullConstraint::new(n("tag")).not_valid())
        .to_string();
    assert_eq!(
        added,
        [
            r#"ALTER TABLE "ledger" ADD CONSTRAINT "memo present" NOT NULL "memo" NOT VALID,"#,
            r#"ADD NOT NULL "tag" NOT VALID"#,
        ]
        .join(" ")
    );
    db.batch_execute(&added).await?;
    assert_eq!(
        not_nulls(&db, "ledger").await?,
        [
            recorded("memo", "memo present", false, false),
            recorded("tag", "ledger_tag_not_null", false, false),
        ]
    );

    let new_null = db
        .batch_execute("INSERT INTO ledger VALUES (2, NULL, 'x')")
        .await
        .expect_err("a not-valid constraint holds new rows");
    refused_with(&new_null, &SqlState::NOT_NULL_VIOLATION);

    let validate = Table::alter(n("ledger"))
        .validate_constraint(n("memo present"))
        .to_string();
    let early = db
        .batch_execute(&validate)
        .await
        .expect_err("a null is still there");
    refused_with(&early, &SqlState::NOT_NULL_VIOLATION);

    db.batch_execute("UPDATE ledger SET memo = '', tag = ''")
        .await?;
    db.batch_execute(&validate).await?;
    db.batch_execute(
        &Table::alter(n("ledger"))
            .modify_column(ColumnDef::new(n("tag")).not_null())
            .to_string(),
    )
    .await?;
    assert_eq!(
        not_nulls(&db, "ledger").await?,
        [
            recorded("memo", "memo present", true, false),
            recorded("tag", "ledger_tag_not_null", true, false),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A `NO INHERIT` constraint stays with the table that declared it, and
/// `ALTER CONSTRAINT` moves a constraint either way: kept from children, a
/// child keeps the copy it had as its own; passed on, a child that lacked it
/// takes it.
// [spec:pgorm:req:sql.ddl.alter-table+15/test]    ALTER CONSTRAINT INHERIT and NO
// INHERIT move a NOT NULL to and from an inheriting table
// [spec:pgorm:req:sql.ddl.column-def+13/test]    a NO INHERIT NOT NULL does not
// reach a child table
#[pgorm_macros::test]
async fn no_inherit_keeps_not_null_from_children() -> Result<(), Error> {
    let ctx = TestContext::new("not_null_no_inherit").await;
    let db = ctx.db.get().await?;

    db.batch_execute(
        &Table::create(n("parent"))
            .col(
                ColumnDef::new(n("shared"))
                    .integer()
                    .not_null_named(n("shared present")),
            )
            .col(
                ColumnDef::new(n("own"))
                    .integer()
                    .not_null_named(n("own present"))
                    .not_null_no_inherit(),
            )
            .to_string(),
    )
    .await?;
    db.batch_execute(
        &Table::create(n("child"))
            .raw_suffix("INHERITS (parent)")
            .to_string(),
    )
    .await?;
    let inherited = async |table: &str| -> Result<Vec<(String, bool, i16)>, Error> {
        let rows = db
            .query_all(
                "SELECT conname::text, conislocal, coninhcount FROM pg_constraint \
                 WHERE conrelid = $1::text::regclass AND contype = 'n' ORDER BY conname",
                &[&table],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| (row.get(0), row.get(1), row.get(2)))
            .collect())
    };
    assert_eq!(
        inherited("child").await?,
        [("shared present".to_owned(), false, 1)]
    );

    db.batch_execute(
        &Table::alter(n("parent"))
            .alter_constraint(n("shared present"), ConstraintChange::NoInherit)
            .alter_constraint(n("own present"), ConstraintChange::Inherit)
            .to_string(),
    )
    .await?;
    assert_eq!(
        inherited("child").await?,
        [
            ("own present".to_owned(), false, 1),
            ("shared present".to_owned(), true, 0),
        ]
    );
    assert_eq!(
        not_nulls(&db, "parent").await?,
        [
            recorded("shared", "shared present", true, true),
            recorded("own", "own present", true, false),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// What the server refuses around a `NOT NULL` constraint is its own
/// knowledge — what already exists on the column, which kind a name holds,
/// whether the table is partitioned — and it refuses each by name.
// [spec:pgorm:req:sql.ddl.alter-table+15/test]    the refusals around adding,
// validating and altering a NOT NULL, by SQLSTATE
// [spec:pgorm:req:sql.ddl.column-def+13/test]    NO INHERIT on a partitioned
// table is refused, and DROP NOT NULL drops a constraint whatever its name
#[pgorm_macros::test]
async fn not_null_refusals_by_sqlstate() -> Result<(), Error> {
    let ctx = TestContext::new("not_null_refusals").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        &Table::create(n("account"))
            .col(
                ColumnDef::new(n("id"))
                    .integer()
                    .not_null_named(n("id present")),
            )
            .col(ColumnDef::new(n("email")).text())
            .col(ColumnDef::new(n("note")).text())
            .unique(TableKey::new(n("email")).name(n("email key")))
            .check(Expr::col(n("id")).gt(0))
            .to_string(),
    )
    .await?;
    db.batch_execute(
        &Table::alter(n("account"))
            .add_not_null(
                NotNullConstraint::new(n("note"))
                    .name(n("note present"))
                    .not_valid(),
            )
            .to_string(),
    )
    .await?;

    let alter = || Table::alter(n("account"));
    let refused = async |sql: String, state: &SqlState| {
        let error = db.batch_execute(&sql).await.expect_err(&sql);
        refused_with(&error, state);
    };

    for clash in [
        alter().add_not_null(NotNullConstraint::new(n("id")).name(n("other name"))),
        alter().add_not_null(NotNullConstraint::new(n("id")).no_inherit()),
        alter().add_not_null(NotNullConstraint::new(n("note"))),
    ] {
        refused(
            clash.to_string(),
            &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE,
        )
        .await;
    }
    db.batch_execute(
        &alter()
            .add_not_null(NotNullConstraint::new(n("id")).name(n("id present")))
            .add_not_null(NotNullConstraint::new(n("id")).not_valid())
            .to_string(),
    )
    .await?;

    refused(
        alter().validate_constraint(n("email key")).to_string(),
        &SqlState::WRONG_OBJECT_TYPE,
    )
    .await;
    refused(
        alter().validate_constraint(n("missing")).to_string(),
        &SqlState::UNDEFINED_OBJECT,
    )
    .await;
    for other in ["email key", "account_id_check"] {
        refused(
            alter()
                .alter_constraint(n(other), ConstraintChange::NoInherit)
                .to_string(),
            &SqlState::WRONG_OBJECT_TYPE,
        )
        .await;
    }
    refused(
        alter()
            .add_not_null(NotNullConstraint::new(n("missing")))
            .to_string(),
        &SqlState::UNDEFINED_COLUMN,
    )
    .await;

    refused(
        Table::create(n("partitioned"))
            .col(ColumnDef::new(n("id")).integer().not_null_no_inherit())
            .raw_suffix("PARTITION BY RANGE (id)")
            .to_string(),
        &SqlState::FEATURE_NOT_SUPPORTED,
    )
    .await;
    db.batch_execute(
        &Table::create(n("partitioned"))
            .col(
                ColumnDef::new(n("id"))
                    .integer()
                    .not_null_named(n("partitioned present")),
            )
            .raw_suffix("PARTITION BY RANGE (id)")
            .to_string(),
    )
    .await?;
    refused(
        Table::alter(n("partitioned"))
            .alter_constraint(n("partitioned present"), ConstraintChange::NoInherit)
            .to_string(),
        &SqlState::FEATURE_NOT_SUPPORTED,
    )
    .await;

    db.batch_execute(
        &alter()
            .modify_column(ColumnDef::new(n("id")).null())
            .to_string(),
    )
    .await?;
    assert_eq!(
        not_nulls(&db, "account").await?,
        [recorded("note", "note present", false, false)]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
