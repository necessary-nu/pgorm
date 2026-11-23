#![allow(unused_imports, dead_code)]

//! A RETURNING list reading a written row's old and new versions, against a
//! live PostgreSQL 19 server.
//!
//! The render tests in pgorm-query hold the spelling to the parser's
//! `ColumnRef` and `ReturningOption`. Only a server settles what each version
//! holds per statement kind, that a rename reaches the same rows and hides
//! the keyword, that a relation of the statement called `old` or `new` takes
//! the name without complaint while a clashing rename is refused, and that a
//! version reference outside RETURNING names nothing. Each case compares the
//! values a statement returns with the row the table held before and after
//! it, so a version that rendered as the other one still fails.

pub mod common;
use ReturningRow::{New, Old};
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Asterisk, Expr, FromItem, IntoNamedTable, Name, OnConflict, Query, ReturningClause,
    ReturningRow, UpdateStatement, Values, alias,
};
use pgorm::{ConnectionTrait, ValueHolder, entity::prelude::*, types::ToSql};
use tokio_postgres::error::SqlState;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("returning_old_new_tests").await;
    let db = ctx.db.get().await?;

    db.batch_execute(
        "CREATE TABLE acct (id integer PRIMARY KEY, balance integer); \
         CREATE TABLE old (id integer PRIMARY KEY, v integer)",
    )
    .await?;

    an_update_returns_the_row_before_and_after(&db).await?;
    a_delete_has_no_new_row(&db).await?;
    insert_old_row_exists_only_where_updated(&db).await?;
    renamed_versions_read_the_same_rows(&db).await?;
    a_relation_named_old_takes_the_keyword(&db).await?;
    a_rename_that_clashes_is_refused(&db).await?;
    a_version_outside_returning_names_nothing(&db).await?;
    a_value_beside_the_versions_is_bound(&db).await?;

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

async fn reset(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(
        "TRUNCATE acct; TRUNCATE old; \
         INSERT INTO acct VALUES (1, 100), (2, 200); \
         INSERT INTO old VALUES (1, 10)",
    )
    .await
}

fn acct() -> Name {
    Name::runtime("acct")
}

fn id() -> Name {
    Name::runtime("id")
}

fn balance() -> Name {
    Name::runtime("balance")
}

/// Run a built statement with its values bound, and read every returned row
/// as nullable integers, column by column.
async fn returned(
    db: &DatabaseConnection,
    (sql, values): (String, Values),
) -> Result<Vec<Vec<Option<i32>>>, Error> {
    let holders: Vec<ValueHolder> = values.into_iter().map(ValueHolder).collect();
    let params: Vec<&(dyn ToSql + Sync)> =
        holders.iter().map(|v| v as &(dyn ToSql + Sync)).collect();
    let rows = db.query_all(&sql, &params).await?;
    Ok(rows
        .iter()
        .map(|row| (0..row.len()).map(|i| row.get(i)).collect())
        .collect())
}

/// The statement's refusal, which must be one.
async fn refusal(db: &DatabaseConnection, statement: (String, Values)) -> Error {
    let sql = statement.0.clone();
    match returned(db, statement).await {
        Ok(rows) => panic!("{sql} returned {rows:?}"),
        Err(error) => error,
    }
}

async fn balances(db: &DatabaseConnection) -> Result<Vec<(i32, i32)>, Error> {
    let rows = db
        .query_all("SELECT id, balance FROM acct ORDER BY id", &[])
        .await?;
    Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
}

/// `UPDATE acct SET balance = balance + <amount> WHERE id = 1`.
fn credit(amount: i32) -> UpdateStatement {
    Query::update()
        .table(acct())
        .value(balance(), Expr::col(balance()).add(amount))
        .and_where(Expr::col(id()).eq(1))
        .to_owned()
}

fn both_balances() -> ReturningClause {
    Query::returning().columns([(Old, balance()), (New, balance())])
}

/// `INSERT INTO acct (id, balance) VALUES (<id>, <balance>)`, overwriting
/// the balance on a conflict when `upsert` is set and skipping the row when
/// it is not.
fn deposit(id_: i32, amount: i32, upsert: bool) -> pgorm::pgorm_query::InsertStatement {
    let conflict = OnConflict::column(id());
    Query::insert()
        .into_table(acct())
        .columns([id(), balance()])
        .values_panic([id_.into(), amount.into()])
        .on_conflict(if upsert {
            conflict.update_column(balance()).into()
        } else {
            conflict.do_nothing()
        })
        .returning(both_balances())
        .to_owned()
}

/// `old` is the row the update found and `new` the row it wrote, and a
/// target column named bare reads the new one.
// [spec:pgorm:def:sql.ast.returning+4/test]    against a live server: UPDATE's `old` is the row
// before and `new` the row after; a bare column reads the new row
// [spec:pgorm:req:sql.render.returning+4/test]
async fn an_update_returns_the_row_before_and_after(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let rows = returned(db, credit(10).returning(both_balances()).build()).await?;
    assert_eq!(rows, [[Some(100), Some(110)]]);
    assert_eq!(balances(db).await?, [(1, 110), (2, 200)]);

    let bare = credit(5)
        .returning(Query::returning().exprs([Expr::col(balance()), Expr::col((New, balance()))]))
        .build();
    assert_eq!(returned(db, bare).await?, [[Some(115), Some(115)]]);
    Ok(())
}

/// A delete leaves no row behind, so every column of `new` is NULL while
/// `old` is the row it removed.
// [spec:pgorm:def:sql.ast.returning+4/test]    against a live server: DELETE's `new` reads NULL
// in every column
// [spec:pgorm:def:sql.types.column-ref+1/test]    `old.*` and `new.*` reach every column
async fn a_delete_has_no_new_row(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let delete = Query::delete()
        .from_table(acct())
        .and_where(Expr::col(id()).eq(2))
        .returning(Query::returning().columns([(Old, Asterisk), (New, Asterisk)]))
        .build();
    assert_eq!(
        returned(db, delete).await?,
        [[Some(2), Some(200), None, None]]
    );
    assert_eq!(balances(db).await?, [(1, 100)]);
    Ok(())
}

/// An inserted row had no earlier version, so `old` is NULL, unless `ON
/// CONFLICT DO UPDATE` updated an existing row instead, whose `old` is the
/// row as it stood. A row `DO NOTHING` skipped returns nothing at all.
// [spec:pgorm:def:sql.ast.returning+4/test]    against a live server: INSERT's `old` is NULL except
// for the row ON CONFLICT DO UPDATE updated; a skipped row returns nothing
async fn insert_old_row_exists_only_where_updated(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let fresh = returned(db, deposit(3, 300, false).build()).await?;
    assert_eq!(fresh, [[None, Some(300)]]);

    let updated = returned(db, deposit(3, 333, true).build()).await?;
    assert_eq!(updated, [[Some(300), Some(333)]]);

    let inserted = returned(db, deposit(4, 444, true).build()).await?;
    assert_eq!(inserted, [[None, Some(444)]]);

    let skipped = returned(db, deposit(4, 999, false).build()).await?;
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(
        balances(db).await?,
        [(1, 100), (2, 200), (3, 333), (4, 444)]
    );
    Ok(())
}

/// A rename reads the same rows under the new names, and the keyword it
/// replaced names nothing afterwards.
// [spec:pgorm:def:sql.ast.returning+4/test]    against a live server: `old_as` / `new_as` reach
// the same versions, and the renamed keyword is refused (42P01)
// [spec:pgorm:req:sql.render.returning+4/test]
async fn renamed_versions_read_the_same_rows(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let (before, after) = (alias("before"), alias("after"));
    let renamed = credit(10)
        .returning(
            Query::returning()
                .columns([(before, balance()), (after, balance())])
                .old_as(before)
                .new_as(after),
        )
        .build();
    assert_eq!(returned(db, renamed).await?, [[Some(100), Some(110)]]);

    let hidden = credit(10).returning(both_balances().old_as(before)).build();
    refused_with(&refusal(db, hidden).await, &SqlState::UNDEFINED_TABLE);
    assert_eq!(balances(db).await?, [(1, 110), (2, 200)]);
    Ok(())
}

/// A relation of the statement called `old` or `new` takes the name, so the
/// version reference reads that relation instead, and nothing says so. A
/// rename is how the list reaches the version then.
// [spec:pgorm:def:sql.ast.returning+4/test]    against a live server: a target called `old` and a
// FROM item called `new` capture the keywords silently; a rename reaches the old row
async fn a_relation_named_old_takes_the_keyword(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let v = || Name::runtime("v");
    let bump = || {
        Query::update()
            .table(Name::runtime("old"))
            .value(v(), Expr::col(v()).add(1))
            .to_owned()
    };
    let captured = bump()
        .returning(Query::returning().columns([(Old, v()), (New, v())]))
        .build();
    assert_eq!(returned(db, captured).await?, [[Some(11), Some(11)]]);

    let o = alias("o");
    let reached = bump()
        .returning(Query::returning().columns([(o, v())]).old_as(o))
        .build();
    assert_eq!(returned(db, reached).await?, [[Some(11)]]);

    let new = || Name::runtime("new");
    let source = Query::select()
        .expr_as(Expr::raw("1"), id())
        .expr_as(Expr::raw("7"), balance())
        .take();
    let joined = Query::update()
        .table(acct())
        .value(balance(), Expr::col((acct(), balance())).add(1))
        .from(FromItem::SubQuery(source, new()))
        .and_where(Expr::col((acct(), id())).equals((new(), id())))
        .returning(both_balances())
        .build();
    assert_eq!(returned(db, joined).await?, [[Some(100), Some(7)]]);
    assert_eq!(balances(db).await?, [(1, 101), (2, 200)]);
    Ok(())
}

/// A rename may not take a name the statement already uses, nor one name
/// for both versions; the server refuses each rather than choosing.
// [spec:pgorm:def:sql.ast.returning+4/test]    against a live server: a rename clashing with the
// target, or one name for both versions, is refused (42712)
async fn a_rename_that_clashes_is_refused(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let clash = credit(1)
        .returning(Query::returning().all().old_as(acct()))
        .build();
    refused_with(&refusal(db, clash).await, &SqlState::DUPLICATE_ALIAS);

    let both = credit(1)
        .returning(
            Query::returning()
                .all()
                .old_as(alias("r"))
                .new_as(alias("r")),
        )
        .build();
    refused_with(&refusal(db, both).await, &SqlState::DUPLICATE_ALIAS);
    assert_eq!(balances(db).await?, [(1, 100), (2, 200)]);
    Ok(())
}

/// Outside a RETURNING list, `old` names no relation.
// [spec:pgorm:def:sql.ast.returning+4/test]    against a live server: a version read in WHERE is
// refused (42P01)
async fn a_version_outside_returning_names_nothing(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let misplaced = Query::update()
        .table(acct())
        .value(balance(), 0)
        .and_where(Expr::col((Old, id())).eq(1))
        .build();
    refused_with(&refusal(db, misplaced).await, &SqlState::UNDEFINED_TABLE);
    assert_eq!(balances(db).await?, [(1, 100), (2, 200)]);
    Ok(())
}

/// A value in a RETURNING expression over both versions travels as a
/// parameter, so a hostile string comes back as data and the table survives.
// [spec:pgorm:def:sql.ast.returning+4/test]    against a live server: values beside the versions
// are bound
async fn a_value_beside_the_versions_is_bound(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let payload = "'; DROP TABLE acct; --";
    let (sql, values) = credit(10)
        .returning(
            Query::returning().exprs([
                Expr::col((New, balance()))
                    .sub(Expr::col((Old, balance())))
                    .eq(10),
                Expr::val(payload).into(),
            ]),
        )
        .build();
    assert!(!sql.contains("DROP"), "{sql}");
    let holders: Vec<ValueHolder> = values.into_iter().map(ValueHolder).collect();
    let params: Vec<&(dyn ToSql + Sync)> =
        holders.iter().map(|v| v as &(dyn ToSql + Sync)).collect();
    let row = db.query_one(&sql, &params).await?;
    assert!(row.get::<_, bool>(0));
    assert_eq!(row.get::<_, String>(1), payload);
    assert_eq!(balances(db).await?, [(1, 110), (2, 200)]);
    Ok(())
}
