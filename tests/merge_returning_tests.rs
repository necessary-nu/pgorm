#![allow(unused_imports, dead_code)]

//! PostgreSQL 17's MERGE grammar against a live PostgreSQL 18 server: arms
//! for target rows no source row matched, a RETURNING list with the action
//! each row took, and a MERGE as a common table expression's body.
//!
//! The render tests in pgorm-query hold the spelling to libpg_query's
//! `MergeStmt`. Only a server settles which rows a by-source arm takes and
//! what it may read, what each returned row holds per action, that a row
//! left alone is not returned, where `merge_action()` resolves, and that a
//! MERGE body runs whether or not the enclosing query reads it. Each case
//! compares what a statement returns with the rows the tables hold before
//! and after it, so a clause that rendered and was then ignored still fails.

pub mod common;
use ReturningRow::{New, Old};
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Asterisk, CommonTableExpression, Expr, Func, MatchedAction, MergeInsert, MergeStatement,
    MergeUpdate, Name, NotMatchedAction, PendingMerge, Query, ReturningRow, SimpleExpr, Values,
    WithClause,
};
use pgorm::{ConnectionTrait, ValueHolder, entity::prelude::*, types::ToSql};
use tokio_postgres::error::SqlState;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("merge_returning_tests").await;
    let db = ctx.db.get().await?;

    db.batch_execute(
        "CREATE TABLE stock (sku integer PRIMARY KEY, qty integer NOT NULL, note text); \
         CREATE TABLE delivery (sku integer NOT NULL, qty integer NOT NULL)",
    )
    .await?;

    by_source_arms_take_unmatched_target_rows(&db).await?;
    a_by_source_arm_cannot_read_the_source(&db).await?;
    returning_reports_each_row_and_its_action(&db).await?;
    a_row_left_alone_is_not_returned(&db).await?;
    merge_action_resolves_only_in_a_merge_list(&db).await?;
    a_merge_body_yields_its_returned_rows(&db).await?;
    by_source_and_returning_values_are_bound(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

/// Stock 1, 2 and 3, and deliveries for 1 (matched) and 4 (new).
async fn reset(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(
        "TRUNCATE stock; TRUNCATE delivery; \
         INSERT INTO stock VALUES (1, 5, NULL), (2, 5, NULL), (3, 50, NULL); \
         INSERT INTO delivery VALUES (1, 3), (4, 7)",
    )
    .await
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

async fn stock(db: &DatabaseConnection) -> Result<Vec<(i32, i32, Option<String>)>, Error> {
    let rows = db
        .query_all("SELECT sku, qty, note FROM stock ORDER BY sku", &[])
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

/// Run a built statement with its values bound and return its rows.
async fn rows(
    db: &DatabaseConnection,
    (sql, values): (String, Values),
) -> Result<Vec<tokio_postgres::Row>, Error> {
    let holders: Vec<ValueHolder> = values.into_iter().map(ValueHolder).collect();
    let params: Vec<&(dyn ToSql + Sync)> =
        holders.iter().map(|v| v as &(dyn ToSql + Sync)).collect();
    db.query_all(&sql, &params).await
}

fn stock_t() -> Name {
    Name::runtime("stock")
}

fn delivery_t() -> Name {
    Name::runtime("delivery")
}

fn sku() -> Name {
    Name::runtime("sku")
}

fn qty() -> Name {
    Name::runtime("qty")
}

fn stock_col(column: Name) -> Expr {
    Expr::col((stock_t(), column))
}

fn delivery_col(column: Name) -> Expr {
    Expr::col((delivery_t(), column))
}

fn by_sku() -> PendingMerge {
    Query::merge(
        stock_t(),
        delivery_t(),
        stock_col(sku()).equals((delivery_t(), sku())),
    )
}

fn restock() -> MergeUpdate {
    MergeUpdate::value(qty(), stock_col(qty()).add(delivery_col(qty())))
}

fn stock_new() -> MergeInsert {
    MergeInsert::value(sku(), delivery_col(sku())).and_value(qty(), delivery_col(qty()))
}

/// Stock 2 and 3 have no delivery. The first by-source arm whose condition
/// holds takes each: 3, over 10, is halved by the conditional arm, and 2 is
/// deleted by the unconditional one, which renders after it whatever order
/// the calls came in. The matched and not-matched rows take their own arms.
// [spec:pgorm:req:sql.ast.merge+2/test]    against a live server: by-source arms take the target
// rows no source row matched, the conditional arm first
// [spec:pgorm:req:sql.render.merge+1/test]
async fn by_source_arms_take_unmatched_target_rows(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let merge = by_sku()
        .when_not_matched_by_source(MatchedAction::Delete)
        .when_not_matched_by_source_and(
            stock_col(qty()).gt(10),
            MergeUpdate::value(qty(), stock_col(qty()).div(2)),
        )
        .when_matched(restock())
        .when_not_matched(stock_new())
        .build();
    let (sql, values) = merge;
    let holders: Vec<ValueHolder> = values.into_iter().map(ValueHolder).collect();
    let params: Vec<&(dyn ToSql + Sync)> =
        holders.iter().map(|v| v as &(dyn ToSql + Sync)).collect();
    assert_eq!(db.execute(&sql, &params).await?, 4, "{sql}");
    assert_eq!(
        stock(db).await?,
        [(1, 8, None), (3, 25, None), (4, 7, None)]
    );
    Ok(())
}

/// A target row no source row matched has no source row to read, in its
/// arm's condition or in its update.
// [spec:pgorm:req:sql.ast.merge+2/test]    against a live server: a by-source arm reading the
// source is refused (42P01)
async fn a_by_source_arm_cannot_read_the_source(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    for merge in [
        by_sku().when_not_matched_by_source_and(delivery_col(qty()).gt(0), MatchedAction::Delete),
        by_sku().when_not_matched_by_source(MergeUpdate::value(qty(), delivery_col(qty()))),
    ] {
        let refused = rows(db, merge.build()).await.expect_err("refused");
        refused_with(&refused, &SqlState::UNDEFINED_TABLE);
    }
    assert_eq!(
        stock(db).await?,
        [(1, 5, None), (2, 5, None), (3, 50, None)]
    );
    Ok(())
}

/// Every written row comes back with the action it took and both its
/// versions: an update's old and new rows, an insert's new row with no old,
/// a delete's old row with no new. A target column named bare reads the
/// deleted row as it was.
// [spec:pgorm:req:sql.ast.merge+2/test]    against a live server: `merge_action()` names each
// row's action, `old` / `new` hold what the action produced, and a bare name both relations
// have is ambiguous (42702)
// [spec:pgorm:def:sql.ast.returning+3/test]    on a MERGE
// [spec:pgorm:req:sql.render.returning+4/test]
async fn returning_reports_each_row_and_its_action(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let merge = by_sku()
        .when_matched(restock())
        .when_not_matched(stock_new())
        .when_not_matched_by_source(MatchedAction::Delete)
        .returning_action()
        .returning(Query::returning().exprs([
            Expr::col((Old, qty())),
            Expr::col((New, qty())),
            stock_col(sku()),
        ]))
        .build();
    let mut returned: Vec<(String, Option<i32>, Option<i32>, i32)> = rows(db, merge)
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect();
    returned.sort_by_key(|row| row.3);
    assert_eq!(
        returned,
        [
            ("UPDATE".to_owned(), Some(5), Some(8), 1),
            ("DELETE".to_owned(), Some(5), None, 2),
            ("DELETE".to_owned(), Some(50), None, 3),
            ("INSERT".to_owned(), None, Some(7), 4),
        ]
    );
    assert_eq!(stock(db).await?, [(1, 8, None), (4, 7, None)]);

    reset(db).await?;
    let bare = by_sku()
        .when_matched(MatchedAction::Delete)
        .returning(Query::returning().column(sku()))
        .build();
    let refused = rows(db, bare).await.expect_err("ambiguous");
    refused_with(&refused, &SqlState::AMBIGUOUS_COLUMN);
    Ok(())
}

/// `DO NOTHING` leaves a row alone, and a row left alone is not returned.
// [spec:pgorm:req:sql.ast.merge+2/test]    against a live server: a DO NOTHING row is not returned
async fn a_row_left_alone_is_not_returned(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let merge = by_sku()
        .when_matched(MatchedAction::DoNothing)
        .when_not_matched(NotMatchedAction::DoNothing)
        .when_not_matched_by_source_and(stock_col(qty()).gt(10), MatchedAction::DoNothing)
        .when_not_matched_by_source(MatchedAction::Delete)
        .returning_action()
        .build();
    let actions: Vec<String> = rows(db, merge)
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(actions, ["DELETE"]);
    assert_eq!(stock(db).await?, [(1, 5, None), (3, 50, None)]);
    Ok(())
}

/// `merge_action()` resolves in a MERGE's RETURNING list alone. A named
/// function call cannot reach it, because the name is quoted and the quoted
/// name is no function, and the bare keyword outside a MERGE's list is
/// refused, which is why the builder has no expression for it.
// [spec:pgorm:req:sql.ast.merge+2/test]    against a live server: `merge_action()` outside a
// MERGE's RETURNING is refused (42601), and `Func::named` writes no such function (42883)
async fn merge_action_resolves_only_in_a_merge_list(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let named = Query::select()
        .expr(Func::named(Name::runtime("merge_action")))
        .build();
    let refused = rows(db, named).await.expect_err("no such function");
    refused_with(&refused, &SqlState::UNDEFINED_FUNCTION);

    let elsewhere = Query::update()
        .table(stock_t())
        .value(qty(), 0)
        .returning(Query::returning().expr(Expr::raw("merge_action()")))
        .build();
    let refused = rows(db, elsewhere).await.expect_err("outside a MERGE");
    refused_with(&refused, &SqlState::SYNTAX_ERROR);
    assert_eq!(
        stock(db).await?,
        [(1, 5, None), (2, 5, None), (3, 50, None)]
    );
    Ok(())
}

/// A MERGE body yields the rows its RETURNING list returns. Without a list
/// it still runs, and only reading it is refused.
// [spec:pgorm:req:sql.ast+4/test]    against a live server: a MERGE CTE body runs and yields its
// returned rows; one without RETURNING runs, and reading it is refused (0A000)
// [spec:pgorm:def:sql.ast.with+5/test]
async fn a_merge_body_yields_its_returned_rows(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let m = || Name::runtime("m");
    let body = by_sku()
        .when_matched(restock())
        .when_not_matched(stock_new())
        .returning_action()
        .returning(Query::returning().column((stock_t(), sku())))
        .to_owned();
    let read = Query::select()
        .column(Asterisk)
        .from(m())
        .order_by(sku(), pgorm::pgorm_query::Order::Asc)
        .with(WithClause::new(CommonTableExpression::new(m(), body)))
        .build();
    let yielded: Vec<(String, i32)> = rows(db, read)
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        yielded,
        [("UPDATE".to_owned(), 1), ("INSERT".to_owned(), 4)]
    );
    assert_eq!(
        stock(db).await?,
        [(1, 8, None), (2, 5, None), (3, 50, None), (4, 7, None)]
    );

    reset(db).await?;
    let silent = || {
        CommonTableExpression::new(
            m(),
            by_sku()
                .when_not_matched_by_source(MatchedAction::Delete)
                .to_owned(),
        )
    };
    let unread = Query::select()
        .expr(Expr::raw("1"))
        .with(WithClause::new(silent()))
        .build();
    assert_eq!(rows(db, unread).await?.len(), 1);
    assert_eq!(stock(db).await?, [(1, 5, None)]);

    reset(db).await?;
    let unreadable = Query::select()
        .column(Asterisk)
        .from(m())
        .with(WithClause::new(silent()))
        .build();
    let refused = rows(db, unreadable)
        .await
        .expect_err("no RETURNING to read");
    refused_with(&refused, &SqlState::FEATURE_NOT_SUPPORTED);
    assert_eq!(
        stock(db).await?,
        [(1, 5, None), (2, 5, None), (3, 50, None)]
    );
    Ok(())
}

/// A value in a by-source arm and in the RETURNING list travels as a
/// parameter: a hostile string is written and returned as data.
// [spec:pgorm:req:sql.render.merge+1/test]    against a live server: values in a by-source arm
// and in RETURNING are bound
async fn by_source_and_returning_values_are_bound(db: &DatabaseConnection) -> Result<(), Error> {
    reset(db).await?;
    let payload = "'; DROP TABLE stock; --";
    let merge = by_sku()
        .when_not_matched_by_source_and(
            stock_col(qty()).lt(10),
            MergeUpdate::value(Name::runtime("note"), payload),
        )
        .returning(
            Query::returning().exprs([Expr::col((New, Name::runtime("note"))), Expr::val(payload)]),
        )
        .build();
    assert!(!merge.0.contains("DROP"), "{}", merge.0);
    let returned: Vec<(String, String)> = rows(db, merge)
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(returned, [(payload.to_owned(), payload.to_owned())]);
    assert_eq!(
        stock(db).await?,
        [
            (1, 5, None),
            (2, 5, Some(payload.to_owned())),
            (3, 50, None)
        ]
    );
    Ok(())
}
