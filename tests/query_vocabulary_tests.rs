#![allow(unused_imports, dead_code)]

//! Null-safe comparison, symmetric ranges, time-zone conversion, peer-group
//! window frames and the duplicate-keeping set operations, against a live
//! PostgreSQL server.
//!
//! The render tests in pgorm-query feed each of these to the real grammar,
//! which settles that it parses — and every construct here parses in a form
//! that means something *else*, which is exactly why parsing is not enough.
//! `IS DISTINCT FROM` reads as `<>` until a NULL arrives; `INTERSECT ALL`
//! reads as `INTERSECT` until a duplicate does; `BETWEEN SYMMETRIC` reads as
//! `BETWEEN` until the bounds are written backwards; a `GROUPS` frame reads as
//! `ROWS` until two rows tie. Each case below is therefore separated from its
//! neighbour by an answer only the server can give, and paired with the
//! ordinary spelling as a control, so a clause that rendered and was then
//! ignored still fails.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Expr, FrameType, Func, Keyword, Name, Order, OrderedStatement, Query, SelectStatement,
    SimpleExpr, UnionType, WindowStatement,
};
use pgorm::{ConnectionTrait, entity::prelude::*};

/// Five rows whose `grade` ties in two peer groups and then jumps, whose
/// `note` is NULL on two of them, and whose `at_local` is one fixed wall time.
const SEED: &str = "
    CREATE TABLE sample (id int primary key, grade int, note text, at_local timestamp);
    INSERT INTO sample (id, grade, note, at_local) VALUES
        (1, 1, 'a',  TIMESTAMP '2026-01-01 00:00:00'),
        (2, 1, NULL, TIMESTAMP '2026-01-01 00:00:00'),
        (3, 2, 'b',  TIMESTAMP '2026-01-01 00:00:00'),
        (4, 2, NULL, TIMESTAMP '2026-01-01 00:00:00'),
        (5, 5, 'a',  TIMESTAMP '2026-01-01 00:00:00');

    CREATE TABLE lhs (v int);
    INSERT INTO lhs (v) VALUES (1), (1), (1), (2), (2), (3);
    CREATE TABLE rhs (v int);
    INSERT INTO rhs (v) VALUES (1), (1), (2);
    CREATE TABLE third (v int);
    INSERT INTO third (v) VALUES (1), (2), (2), (4);
";

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("query_vocabulary_tests").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SEED).await?;

    distinct_from_answers_where_equality_says_unknown(&db).await?;
    symmetric_between_sorts_the_bounds_it_is_given(&db).await?;
    at_time_zone_shifts_the_clock(&db).await?;
    a_groups_frame_reaches_whole_peer_groups(&db).await?;
    the_all_set_operations_keep_the_duplicates(&db).await?;
    chained_set_operations_combine_in_call_order(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn note() -> Expr {
    Expr::col(Name::runtime("note"))
}

fn grade() -> Expr {
    Expr::col(Name::runtime("grade"))
}

fn counting(sample: &str) -> SelectStatement {
    Query::select().from(Name::runtime(sample)).take()
}

/// Three ways to ask "is `note` something other than 'a'", over a table where
/// two rows answer NULL. `<>` loses those rows, `IS DISTINCT FROM` keeps them,
/// and `IS NOT DISTINCT FROM` finds exactly them — so the operator is not a
/// verbose spelling of the comparison it sits beside.
// [spec:pgorm:req:sql.ast.expr.operators+6/test]
// [spec:pgorm:req:sql.scope+23/test]
async fn distinct_from_answers_where_equality_says_unknown(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let counted = |predicate: SimpleExpr| {
        counting("sample")
            .expr(Func::count(Expr::col(Name::runtime("id"))))
            .and_where(predicate)
            .to_string()
    };

    let unequal: i64 = db
        .query_one(counted(note().ne("a")).as_str(), &[])
        .await?
        .get(0);
    let distinct: i64 = db
        .query_one(counted(note().is_distinct_from("a")).as_str(), &[])
        .await?
        .get(0);
    let not_distinct: i64 = db
        .query_one(counted(note().is_not_distinct_from("a")).as_str(), &[])
        .await?
        .get(0);
    let is_null_form: i64 = db
        .query_one(
            counted(note().is_not_distinct_from(Keyword::Null)).as_str(),
            &[],
        )
        .await?
        .get(0);

    // 'b' alone: the two NULL rows answer unknown and are dropped.
    assert_eq!(unequal, 1, "<> drops the rows that answer NULL");
    // 'b' and the two NULLs: nothing answers unknown.
    assert_eq!(distinct, 3, "IS DISTINCT FROM keeps them");
    // The two 'a' rows, and the NULLs are *not* among them.
    assert_eq!(not_distinct, 2);
    // Which leaves IS NOT DISTINCT FROM NULL as a spelling of IS NULL.
    assert_eq!(is_null_form, 2);
    assert_eq!(
        unequal + not_distinct + is_null_form,
        5,
        "the three predicates partition the table, so none of them was ignored"
    );

    Ok(())
}

/// `BETWEEN` with its bounds the wrong way round matches nothing, which is the
/// silent-empty-result bug `SYMMETRIC` exists to remove. Both spellings are
/// run over the same reversed pair, so a dropped keyword shows up as the two
/// answers agreeing.
// [spec:pgorm:req:sql.ast.expr.operators+6/test]
// [spec:pgorm:req:sql.scope+23/test]
async fn symmetric_between_sorts_the_bounds_it_is_given(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let counted = |predicate: SimpleExpr| {
        counting("sample")
            .expr(Func::count(Expr::col(Name::runtime("id"))))
            .and_where(predicate)
            .to_string()
    };

    let forwards: i64 = db
        .query_one(counted(grade().between(1, 2)).as_str(), &[])
        .await?
        .get(0);
    let backwards: i64 = db
        .query_one(counted(grade().between(2, 1)).as_str(), &[])
        .await?
        .get(0);
    let symmetric: i64 = db
        .query_one(counted(grade().between_symmetric(2, 1)).as_str(), &[])
        .await?
        .get(0);
    let not_symmetric: i64 = db
        .query_one(counted(grade().not_between_symmetric(2, 1)).as_str(), &[])
        .await?
        .get(0);

    assert_eq!(forwards, 4, "grades 1, 1, 2, 2");
    assert_eq!(
        backwards, 0,
        "plain BETWEEN reads the pair as an empty range"
    );
    assert_eq!(
        symmetric, forwards,
        "SYMMETRIC sorts the pair, so the order stops mattering"
    );
    assert_eq!(not_symmetric, 1, "and its complement is the grade-5 row");

    Ok(())
}

/// Applied twice, `AT TIME ZONE` round-trips a wall time through a zone: the
/// first reads the value as UTC, the second reports the same instant in Tokyo,
/// nine hours later. The UTC-to-UTC pairing is the control — same statement
/// shape, same operator, identity answer — so a clause the server ignored
/// would leave both answers at midnight.
// [spec:pgorm:req:sql.ast.expr.operators+6/test]
// [spec:pgorm:req:sql.scope+23/test]
async fn at_time_zone_shifts_the_clock(db: &DatabaseConnection) -> Result<(), Error> {
    let read_in = |zone: &str| {
        let local = Expr::col(Name::runtime("at_local")).at_time_zone("UTC");
        Query::select()
            .expr(Expr::expr(Expr::expr(local).at_time_zone(zone)).cast_as(Name::runtime("text")))
            .from(Name::runtime("sample"))
            .and_where(Expr::col(Name::runtime("id")).eq(1))
            .to_string()
    };

    let utc: String = db.query_one(read_in("UTC").as_str(), &[]).await?.get(0);
    let tokyo: String = db
        .query_one(read_in("Asia/Tokyo").as_str(), &[])
        .await?
        .get(0);

    assert_eq!(utc, "2026-01-01 00:00:00");
    assert_eq!(tokyo, "2026-01-01 09:00:00", "the zone moved the clock");

    Ok(())
}

/// The three frame modes over grades 1, 1, 2, 2, 5, read at the last row.
/// `ROWS` counts rows, `RANGE` counts values within a distance, and `GROUPS`
/// counts peer groups — three different answers from one offset, which is what
/// makes `GROUPS` a mode rather than a spelling of one of the other two.
// [spec:pgorm:def:sql.ast.window-statement+8/test]
// [spec:pgorm:req:sql.scope+23/test]
async fn a_groups_frame_reaches_whole_peer_groups(db: &DatabaseConnection) -> Result<(), Error> {
    let framed = |r#type: FrameType| {
        WindowStatement::new()
            .order_by(Name::runtime("grade"), Order::Asc)
            .frame(r#type.preceding(1).and_current_row())
            .take()
    };

    let sql = Query::select()
        .expr_window(
            Func::count(Expr::col(Name::runtime("id"))),
            framed(FrameType::Rows),
        )
        .expr_window(
            Func::count(Expr::col(Name::runtime("id"))),
            framed(FrameType::Range),
        )
        .expr_window(
            Func::count(Expr::col(Name::runtime("id"))),
            framed(FrameType::Groups),
        )
        .from(Name::runtime("sample"))
        .order_by(Name::runtime("grade"), Order::Desc)
        .limit(1)
        .to_string();

    let row = db.query_one(sql.as_str(), &[]).await?;
    let rows: i64 = row.get(0);
    let range: i64 = row.get(1);
    let groups: i64 = row.get(2);

    assert_eq!(rows, 2, "one row back, plus this one");
    assert_eq!(range, 1, "no grade lies within 1 of 5, so only this one");
    assert_eq!(
        groups, 3,
        "one peer group back — both grade-2 rows — plus this one"
    );

    Ok(())
}

/// Six set operations over two bags with duplicates. The `ALL` forms keep
/// multiplicity and the plain forms collapse it, so each pair differs; without
/// `INTERSECT ALL` and `EXCEPT ALL` the middle four answers would be
/// unreachable from the builder at all.
// [spec:pgorm:sem:query.build.union/test]
// [spec:pgorm:req:sql.scope+23/test]
async fn the_all_set_operations_keep_the_duplicates(db: &DatabaseConnection) -> Result<(), Error> {
    let bag = |table: &str| {
        Query::select()
            .column(Name::runtime("v"))
            .from(Name::runtime(table))
            .take()
    };
    let combined = |union: UnionType| bag("lhs").union(union, bag("rhs")).to_string();

    let rows = async |union: UnionType| -> Result<usize, Error> {
        Ok(db.query_all(combined(union).as_str(), &[]).await?.len())
    };

    // lhs is 1,1,1,2,2,3 and rhs is 1,1,2.
    assert_eq!(
        rows(UnionType::Distinct).await?,
        3,
        "UNION collapses to three distinct values"
    );
    assert_eq!(
        rows(UnionType::All).await?,
        9,
        "UNION ALL: every row of both"
    );
    assert_eq!(
        rows(UnionType::Intersect).await?,
        2,
        "INTERSECT collapses to two distinct values"
    );
    assert_eq!(
        rows(UnionType::IntersectAll).await?,
        3,
        "INTERSECT ALL: two 1s — the smaller multiplicity — and one 2"
    );
    assert_eq!(
        rows(UnionType::Except).await?,
        1,
        "EXCEPT leaves only the value rhs never had"
    );
    assert_eq!(
        rows(UnionType::ExceptAll).await?,
        3,
        "EXCEPT ALL: one 1 and one 2 survive the subtraction, and the 3"
    );

    Ok(())
}

/// `op` over two bags of values, as PostgreSQL computes it: the plain forms
/// yield each qualifying value once, the `ALL` forms by multiplicity.
fn combine(op: UnionType, left: &[i32], right: &[i32]) -> Vec<i32> {
    use std::collections::BTreeMap;
    let count = |bag: &[i32]| {
        bag.iter().fold(BTreeMap::new(), |mut counts, v| {
            *counts.entry(*v).or_insert(0usize) += 1;
            counts
        })
    };
    let (l, r) = (count(left), count(right));
    let times = |v: &i32| -> usize {
        let (a, b) = (
            l.get(v).copied().unwrap_or(0),
            r.get(v).copied().unwrap_or(0),
        );
        match op {
            UnionType::Distinct => usize::from(a + b > 0),
            UnionType::All => a + b,
            UnionType::Intersect => usize::from(a > 0 && b > 0),
            UnionType::IntersectAll => a.min(b),
            UnionType::Except => usize::from(a > 0 && b == 0),
            UnionType::ExceptAll => a.saturating_sub(b),
            other => panic!("no bag model is known for {other:?}"),
        }
    };
    let mut values: Vec<i32> = l.keys().chain(r.keys()).copied().collect();
    values.sort_unstable();
    values.dedup();
    values
        .iter()
        .flat_map(|v| std::iter::repeat_n(*v, times(v)))
        .collect()
}

/// Every pair of set operations chained over three bags reads in call order,
/// `(lhs op1 rhs) op2 third`, which the server is asked and a model of the six
/// operations computes. Where the flat chain would mean something else — an
/// INTERSECT after a UNION or EXCEPT — the bags are chosen so the two readings
/// differ, so a chain that rendered flat fails here.
// [spec:pgorm:sem:sql.ast.select.union+2/test]
async fn chained_set_operations_combine_in_call_order(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let operations = [
        UnionType::Distinct,
        UnionType::All,
        UnionType::Intersect,
        UnionType::IntersectAll,
        UnionType::Except,
        UnionType::ExceptAll,
    ];
    let bag = |table: &str| {
        Query::select()
            .column(Name::runtime("v"))
            .from(Name::runtime(table))
            .take()
    };
    let (lhs, rhs, third) = ([1, 1, 1, 2, 2, 3], [1, 1, 2], [1, 2, 2, 4]);
    let intersects = |op| matches!(op, UnionType::Intersect | UnionType::IntersectAll);

    for first in operations {
        for second in operations {
            let sql = bag("lhs")
                .union(first, bag("rhs"))
                .union(second, bag("third"))
                .order_by(Name::runtime("v"), Order::Asc)
                .to_string();
            let rows: Vec<i32> = db
                .query_all(sql.as_str(), &[])
                .await?
                .iter()
                .map(|row| row.get(0))
                .collect();
            let in_call_order = combine(second, &combine(first, &lhs, &rhs), &third);
            assert_eq!(rows, in_call_order, "{first:?} then {second:?}: {sql}");

            if intersects(second) && !intersects(first) {
                let flat = combine(first, &lhs, &combine(second, &rhs, &third));
                assert_ne!(
                    flat, in_call_order,
                    "{first:?} then {second:?} discriminates"
                );
            }
        }
    }

    Ok(())
}
