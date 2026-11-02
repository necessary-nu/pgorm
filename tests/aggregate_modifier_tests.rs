#![allow(unused_imports, dead_code)]

//! `FILTER (WHERE ..)` and `WITHIN GROUP (ORDER BY ..)` against a live
//! PostgreSQL server.
//!
//! The render tests in pgorm-query feed the SQL to the real grammar, which
//! settles that it parses. What only a server settles is what the clauses
//! *mean*: that a filtered aggregate sees a different row set rather than the
//! same one, and that an ordered-set aggregate reads the ordering it is given.
//! Each case here is therefore checked against an independently computed
//! answer over a known distribution — a CASE-sum equivalence for FILTER, and
//! a hand-computed percentile for WITHIN GROUP — rather than against a number
//! the same clause produced.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{Expr, Func, Name, Order, Query, SimpleExpr};
use pgorm::{ConnectionTrait, entity::prelude::*};

/// A known distribution: `size_w` 1..=10, `ascii` true on the even ones.
const SEED: &str = "
    CREATE TABLE sample (id int primary key, size_w int, ascii bool);
    INSERT INTO sample (id, size_w, ascii)
    SELECT i, i, (i % 2 = 0) FROM generate_series(1, 10) AS i;
";

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("aggregate_modifier_tests").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SEED).await?;

    filter_agrees_with_the_case_sum_it_replaces(&db).await?;
    filter_narrows_the_row_set_the_aggregate_sees(&db).await?;
    within_group_reads_the_ordering_it_is_given(&db).await?;
    filter_composes_with_a_window(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn size_w() -> SimpleExpr {
    Expr::col(Name::runtime("size_w")).into()
}

fn even() -> SimpleExpr {
    Expr::col(Name::runtime("ascii")).eq(true)
}

/// The identity FILTER exists to replace: `SUM(x) FILTER (WHERE c)` and
/// `SUM(CASE WHEN c THEN x END)` agree, so the new spelling is checked against
/// the old one computed in the same query — one row, two columns, no chance of
/// the two seeing different data.
// [spec:pgorm:req:sql.render.func-mods/test]
// [spec:pgorm:def:sql.ast.func+12/test]
async fn filter_agrees_with_the_case_sum_it_replaces(db: &DatabaseConnection) -> Result<(), Error> {
    let sql = Query::select()
        .expr(Func::sum(size_w()).filter(even()))
        .expr(Expr::raw(
            "SUM(CASE WHEN \"ascii\" = TRUE THEN \"size_w\" END)",
        ))
        .from(Name::runtime("sample"))
        .to_string();

    let row = db.query_one(sql.as_str(), &[]).await?;
    let filtered: i64 = row.get(0);
    let case_sum: i64 = row.get(1);

    // 2 + 4 + 6 + 8 + 10
    assert_eq!(filtered, 30);
    assert_eq!(filtered, case_sum, "FILTER and the CASE sum are one answer");

    Ok(())
}

/// The sensitivity control the equivalence above cannot supply on its own: the
/// filtered count must differ from the unfiltered one. Without this, a FILTER
/// that rendered but was ignored by the server would still pass.
// [spec:pgorm:req:sql.render.func-mods/test]
async fn filter_narrows_the_row_set_the_aggregate_sees(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let sql = Query::select()
        .expr(Func::count(Expr::col(Name::runtime("id"))))
        .expr(Func::count(Expr::col(Name::runtime("id"))).filter(even()))
        .expr(
            Func::count(Expr::col(Name::runtime("id"))).filter(SimpleExpr::from(Expr::val(false))),
        )
        .from(Name::runtime("sample"))
        .to_string();

    let row = db.query_one(sql.as_str(), &[]).await?;
    let all: i64 = row.get(0);
    let evens: i64 = row.get(1);
    let none: i64 = row.get(2);

    assert_eq!(all, 10);
    assert_eq!(evens, 5, "the filter is applied, not ignored");
    assert_eq!(none, 0, "a filter matching nothing counts nothing");

    Ok(())
}

/// `PERCENTILE_CONT` interpolates and `PERCENTILE_DISC` does not, so over
/// 1..=10 the median is 5.5 and 5 respectively — two different answers from
/// the same ordering, which is what proves the WITHIN GROUP ordering reached
/// the server rather than the clause being dropped.
// [spec:pgorm:req:sql.render.func-mods/test]
// [spec:pgorm:def:sql.ast.func+12/test]
async fn within_group_reads_the_ordering_it_is_given(db: &DatabaseConnection) -> Result<(), Error> {
    let sql = Query::select()
        .expr(Func::percentile_cont(0.5).within_group(Name::runtime("size_w"), Order::Asc))
        .expr(Func::percentile_disc(0.5).within_group(Name::runtime("size_w"), Order::Asc))
        .expr(Func::percentile_disc(0.5).within_group(Name::runtime("size_w"), Order::Desc))
        .from(Name::runtime("sample"))
        .to_string();

    let row = db.query_one(sql.as_str(), &[]).await?;
    let cont: f64 = row.get(0);
    let disc_asc: i32 = row.get(1);
    let disc_desc: i32 = row.get(2);

    assert_eq!(cont, 5.5, "the continuous median interpolates");
    assert_eq!(disc_asc, 5, "the discrete median does not");
    assert_eq!(
        disc_desc, 6,
        "reversing the ordering moves the discrete median, so the direction is read"
    );

    Ok(())
}

/// `agg(x) FILTER (WHERE c) OVER w` is legal PostgreSQL and the two clauses
/// are written by different rules, so that they compose is worth asserting
/// against a server and not only against the grammar.
// [spec:pgorm:req:sql.render.func-mods/test]
// [spec:pgorm:req:sql.render.window+7/test]
async fn filter_composes_with_a_window(db: &DatabaseConnection) -> Result<(), Error> {
    let sql = Query::select()
        .expr_window(
            Func::count(Expr::col(Name::runtime("id"))).filter(even()),
            pgorm::pgorm_query::WindowStatement::new(),
        )
        .from(Name::runtime("sample"))
        .and_where(Expr::col(Name::runtime("id")).lte(4))
        .to_string();

    let rows = db.query_all(sql.as_str(), &[]).await?;
    assert_eq!(rows.len(), 4, "the window keeps every row");

    // Rows 1..=4, of which 2 and 4 are even: an unfiltered window count would
    // be 4 on every row.
    for row in &rows {
        let counted: i64 = row.get(0);
        assert_eq!(counted, 2, "the filter applies inside the window frame");
    }

    Ok(())
}
