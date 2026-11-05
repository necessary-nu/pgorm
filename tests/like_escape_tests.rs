#![allow(unused_imports, dead_code)]

//! A pattern's `ESCAPE ''` against a live server.
//!
//! The render tests hold `ESCAPE ''` to libpg_query; only the server says
//! that escaping is then off, so each case reads a pattern holding a
//! backslash under the default escape, a chosen one, and none, where the
//! three disagree.

use pgorm_query::ExprTrait;
pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{Expr, LikeExpr, Name, Query, SimpleExpr};
use pgorm::{ConnectionTrait, Error, SelectGetableTuple, Selector};

fn text(value: &str) -> SimpleExpr {
    Expr::val(value).cast_as(Name::runtime("text"))
}

/// Each predicate's answer, read in one row.
async fn answers<C: ConnectionTrait>(
    db: &C,
    predicates: [SimpleExpr; 3],
) -> Result<(bool, bool, bool), Error> {
    let mut query = Query::select();
    for predicate in predicates {
        query.expr(predicate);
    }
    let rows = Selector::<SelectGetableTuple<(bool, bool, bool)>>::into_tuple(query.take())
        .all(db)
        .await?;
    Ok(rows[0])
}

// [spec:pgorm:req:sql.ast.expr.similar-to+2/test]    against a live server: `no_escape` turns
// escaping off for LIKE, ILIKE and SIMILAR TO, a backslash then matching itself
#[pgorm_macros::test]
async fn no_escape_reads_a_backslash_as_itself() -> Result<(), Error> {
    let ctx = TestContext::new("no_escape_reads_a_backslash_as_itself").await;
    let db = ctx.db.get().await?;
    let off = |pattern: &str| LikeExpr::new(pattern).no_escape();

    // `a\b` under the default escape is `ab`; with escaping off it is
    // itself, and `!` as the escape leaves the backslash alone too.
    assert_eq!(
        answers(
            &db,
            [
                text("a\\b").like("a\\b"),
                text("a\\b").like(off("a\\b")),
                text("a\\b").like(LikeExpr::new("a\\b").escape('!')),
            ],
        )
        .await?,
        (false, true, true)
    );
    // With escaping off, `\%` is a backslash and a wildcard, so `50%`,
    // which has no backslash, does not match it.
    assert_eq!(
        answers(
            &db,
            [
                text("50%").like("50\\%"),
                text("50%").not_like(off("50\\%")),
                Expr::expr(text("A\\B")).ilike(off("a\\b")),
            ],
        )
        .await?,
        (true, true, true)
    );
    assert_eq!(
        answers(
            &db,
            [
                text("a\\b").similar_to("a\\b"),
                text("a\\b").similar_to(off("a\\b")),
                text("a\\b").not_similar_to(off("a\\b")),
            ],
        )
        .await?,
        (false, true, false)
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
