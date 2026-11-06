#![allow(unused_imports, dead_code)]

//! Simple-form `CASE`, against a live PostgreSQL server.
//!
//! The render tests settle that `CASE x WHEN v THEN …` parses with its
//! operand in place. What they cannot settle is the one way the simple form
//! differs from the searched form it resembles: each arm is the comparison
//! `x = v`, so a NULL operand is *unknown* against every arm — `WHEN NULL`
//! included — and falls through to the `ELSE`, where the searched form's
//! `WHEN x IS NULL` catches it. Each case below runs the simple form beside
//! the searched spelling a reader might think it abbreviates, so a renderer
//! that quietly produced the other form would show up as the answers
//! agreeing.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Expr, Keyword, Name, Order, OrderedStatement, Query, SelectStatement, SimpleExpr, Values,
};
use pgorm::{ConnectionTrait, SelectGetableTuple, SelectorRaw, entity::prelude::*};

/// Four notes: two that arms will name, one NULL, and one no arm names.
const SEED: &str = "
    CREATE TABLE sample (id int primary key, note text);
    INSERT INTO sample (id, note) VALUES (1, 'a'), (2, 'b'), (3, NULL), (4, 'c');
";

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("simple_case_tests").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SEED).await?;

    a_null_operand_matches_no_arm(&db).await?;
    an_unmatched_operand_without_else_is_null(&db).await?;
    the_first_matching_arm_wins(&db).await?;
    bound_arm_values_compare_like_literals(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn note() -> Expr {
    Expr::col(Name::runtime("note"))
}

/// `SELECT id, <expr> FROM sample ORDER BY id`.
fn per_row(expr: impl Into<SimpleExpr>) -> SelectStatement {
    Query::select()
        .column(Name::runtime("id"))
        .expr(expr)
        .from(Name::runtime("sample"))
        .order_by(Name::runtime("id"), Order::Asc)
        .take()
}

/// The answer column of [`per_row`], one entry per row in id order.
async fn answers(
    db: &DatabaseConnection,
    query: &SelectStatement,
) -> Result<Vec<Option<String>>, Error> {
    let rows = db.query_all(query.to_string().as_str(), &[]).await?;
    Ok(rows.iter().map(|row| row.get(1)).collect())
}

fn some(answers: [Option<&str>; 4]) -> Vec<Option<String>> {
    answers.into_iter().map(|a| a.map(str::to_owned)).collect()
}

/// A NULL note is equal to nothing, not even NULL: the simple form's
/// `WHEN NULL` arm never fires and the row takes the `ELSE`. The searched
/// form's `WHEN note IS NULL` — the spelling it is easy to mistake it for —
/// catches the same row.
// [spec:pgorm:def:sql.ast.case+3/test]    against a live server: the simple form compares by
// `=`, so a NULL operand matches no arm and takes the ELSE
// [spec:pgorm:req:sql.render.case/test]
// [spec:pgorm:req:sql.scope+23/test]
async fn a_null_operand_matches_no_arm(db: &DatabaseConnection) -> Result<(), Error> {
    let simple = per_row(
        Expr::case_of(note())
            .when("a", "first")
            .when("b", "second")
            .when(Keyword::Null, "null arm")
            .finally("fallthrough"),
    );
    let searched = per_row(
        Expr::case(note().is_null(), "null arm")
            .case(note().eq("a"), "first")
            .case(note().eq("b"), "second")
            .finally("fallthrough"),
    );

    assert!(
        simple.to_string().contains(r#"(CASE "note" WHEN 'a'"#),
        "the operand form was rendered: {simple}"
    );
    assert_eq!(
        answers(db, &simple).await?,
        some([
            Some("first"),
            Some("second"),
            Some("fallthrough"),
            Some("fallthrough"),
        ]),
        "row 3's NULL note is equal to no arm, the NULL arm included"
    );
    assert_eq!(
        answers(db, &searched).await?,
        some([
            Some("first"),
            Some("second"),
            Some("null arm"),
            Some("fallthrough"),
        ]),
        "the searched form's IS NULL arm catches the row the simple form cannot"
    );

    Ok(())
}

/// With no `finally`, an operand that equals no arm's value — the NULL one and
/// the unnamed one alike — yields NULL rather than an error or a default.
// [spec:pgorm:def:sql.ast.case+3/test]    against a live server: no ELSE yields NULL
async fn an_unmatched_operand_without_else_is_null(db: &DatabaseConnection) -> Result<(), Error> {
    let query = per_row(Expr::case_of(note()).when("a", "first"));

    assert_eq!(
        answers(db, &query).await?,
        some([Some("first"), None, None, None])
    );

    Ok(())
}

/// Arms are tried in the order they were added: a value named twice selects
/// the first arm's result, so the arms render in call order.
// [spec:pgorm:def:sql.ast.case+3/test]    against a live server: arms are tried in call order
async fn the_first_matching_arm_wins(db: &DatabaseConnection) -> Result<(), Error> {
    let query = per_row(
        Expr::case_of(note())
            .when("a", "earlier")
            .when("a", "later")
            .finally("other"),
    );

    assert_eq!(
        answers(db, &query).await?,
        some([Some("earlier"), Some("other"), Some("other"), Some("other"),])
    );

    Ok(())
}

/// The `build()` path: every arm value and result a `$N` placeholder. The
/// server types each arm value from its comparison with the operand, so
/// bound values answer exactly as the inlined literals do, NULL operand
/// included.
// [spec:pgorm:req:sql.render.case/test]    against a live server: the bound rendering answers as
// the inlined one does
async fn bound_arm_values_compare_like_literals(db: &DatabaseConnection) -> Result<(), Error> {
    let query = per_row(
        Expr::case_of(note())
            .when("a", "first")
            .when("c", "third")
            .finally("fallthrough"),
    );
    let (sql, values) = query.build();
    assert!(
        sql.contains(r#"(CASE "note" WHEN $1 THEN $2 WHEN $3 THEN $4 ELSE $5 END)"#),
        "{sql}"
    );

    let bound: Vec<(i32, Option<String>)> =
        SelectorRaw::<SelectGetableTuple<(i32, Option<String>)>>::into_tuple(&sql, values)
            .all(db)
            .await?;
    let bound: Vec<Option<String>> = bound.into_iter().map(|(_, answer)| answer).collect();

    assert_eq!(bound, answers(db, &query).await?);
    assert_eq!(
        bound,
        some([
            Some("first"),
            Some("fallthrough"),
            Some("fallthrough"),
            Some("third"),
        ])
    );

    Ok(())
}
