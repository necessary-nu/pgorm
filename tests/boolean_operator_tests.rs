#![allow(unused_imports, dead_code)]

//! The regular-expression complements, the jsonpath existence test and the
//! range position tests against a live server.
//!
//! The render oracle proves the lexemes and how the grammar groups them; it
//! cannot prove what PostgreSQL answers. Each operator is therefore asked of
//! rows where it holds and rows where it does not, built by pgorm-query with
//! its operand bound, and its answer held to the one the same test written as
//! SQL text gets — under the logical operators too, where these operators
//! render bare.
//!
//! Run the test locally:
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:54329 cargo test --test boolean_operator_tests

pub mod common;

pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    BinOper, Condition, Expr, Multirange, Name, Query, Range, RangeType, SimpleExpr, Value, Values,
};
use pgorm::{ConnectionTrait, entity::prelude::*};
use pretty_assertions::assert_eq;
use std::ops::Bound::{Excluded, Included};

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("boolean_operator_tests").await;
    let db = ctx.db.get().await?;

    seed(&db).await?;
    each_operator_answers_as_postgres_does(&db).await?;
    bare_operators_group_as_built(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn col(name: &str) -> Expr {
    Expr::col(n(name))
}

async fn seed(db: &DatabaseConnection) -> Result<(), Error> {
    db.execute(
        "CREATE TABLE operand (id int PRIMARY KEY, label text NOT NULL, doc jsonb NOT NULL, \
         span int4range NOT NULL, spans int4multirange NOT NULL)",
        &[],
    )
    .await?;
    db.execute(
        "INSERT INTO operand VALUES \
         (1, 'pgorm', '{\"a\": 1}', '[1,5)', '{[1,3),[7,9)}'), \
         (2, 'PGORM', '{\"b\": [1, 2]}', '[5,8)', '{[5,8)}'), \
         (3, 'sql', '{\"a\": \"x\"}', '[10,12)', '{[20,30)}')",
        &[],
    )
    .await?;
    Ok(())
}

fn span(lower: i32, upper: i32) -> Value {
    Value::Range(
        RangeType::Int4,
        Some(Box::new(Range::new(
            Included(lower.into()),
            Excluded(upper.into()),
        ))),
    )
}

fn spans(ranges: &[(i32, i32)]) -> Value {
    Value::Multirange(
        RangeType::Int4,
        Some(Box::new(Multirange::from(
            ranges
                .iter()
                .map(|&(lower, upper)| Range::new(Included(lower.into()), Excluded(upper.into())))
                .collect::<Vec<_>>(),
        ))),
    )
}

/// The ids of the rows `condition` selects, built with its operands bound.
async fn built(db: &DatabaseConnection, condition: Condition) -> Result<Vec<i32>, Error> {
    let (sql, values) = Query::select()
        .column(n("id"))
        .from(n("operand"))
        .cond_where(condition)
        .order_by(n("id"), pgorm::pgorm_query::Order::Asc)
        .build();
    (sql, values).into_tuple().all(db).await
}

/// The ids of the rows a WHERE clause written as SQL text selects.
async fn written(db: &DatabaseConnection, predicate: &str) -> Result<Vec<i32>, Error> {
    let sql = format!("SELECT id FROM operand WHERE {predicate} ORDER BY id");
    (sql, Values(Vec::new())).into_tuple().all(db).await
}

// [spec:pgorm:def:sql.types.opers+6/test]    against a live server, each of these operators selects
// the rows PostgreSQL's own operator written as SQL text selects, holding on some rows and
// failing on others
// [spec:pgorm:def:sql.render.operators+8/test]
// [spec:pgorm:req:sql.ast.expr.json+3/test]
async fn each_operator_answers_as_postgres_does(db: &DatabaseConnection) -> Result<(), Error> {
    let cases: [(SimpleExpr, &str, Vec<i32>); 9] = [
        (
            col("label").binary(BinOper::NotRegex, "^pg"),
            "label !~ '^pg'",
            vec![2, 3],
        ),
        (
            col("label").binary(BinOper::NotRegexCaseInsensitive, "^pg"),
            "label !~* '^pg'",
            vec![3],
        ),
        (
            col("doc").json_path_exists("$.a"),
            "doc @? '$.a'",
            vec![1, 3],
        ),
        (
            col("span").binary(BinOper::NotExtendRightOf, span(1, 6)),
            "span &< '[1,6)'::int4range",
            vec![1],
        ),
        (
            col("span").binary(BinOper::NotExtendLeftOf, span(5, 6)),
            "span &> '[5,6)'::int4range",
            vec![2, 3],
        ),
        (
            col("span").binary(BinOper::AdjacentTo, span(8, 9)),
            "span -|- '[8,9)'::int4range",
            vec![2],
        ),
        (
            col("spans").binary(BinOper::NotExtendRightOf, spans(&[(1, 10)])),
            "spans &< '{[1,10)}'::int4multirange",
            vec![1, 2],
        ),
        (
            col("span").binary(BinOper::LShift, span(9, 20)),
            "span << '[9,20)'::int4range",
            vec![1, 2],
        ),
        (
            col("span").binary(BinOper::RShift, span(1, 5)),
            "span >> '[1,5)'::int4range",
            vec![2, 3],
        ),
    ];
    for (test, predicate, expected) in cases {
        let ids = built(db, Condition::all().add(test.clone())).await?;
        assert_eq!(ids, written(db, predicate).await?, "{predicate}");
        assert_eq!(ids, expected, "{predicate}");
    }
    Ok(())
}

// [spec:pgorm:def:sql.render.precedence+14/test]    against a live server, these operators
// rendered bare under AND, OR and NOT select the rows the parenthesised condition selects
async fn bare_operators_group_as_built(db: &DatabaseConnection) -> Result<(), Error> {
    let condition = Condition::all()
        .add(col("label").binary(BinOper::NotRegex, "^s"))
        .add(
            Condition::any()
                .add(col("span").binary(BinOper::AdjacentTo, span(8, 9)))
                .add(col("doc").json_path_exists("$.a")),
        )
        .add(
            col("span")
                .binary(BinOper::NotExtendLeftOf, span(9, 10))
                .not(),
        );
    let query = Query::select()
        .column(n("id"))
        .from(n("operand"))
        .cond_where(condition.clone())
        .to_owned();
    assert_eq!(
        query.build().0,
        [
            r#"SELECT "id" FROM "operand" WHERE "label" !~ $1"#,
            r#"AND ("span" -|- $2 OR "doc" @? CAST($3::text AS jsonpath))"#,
            r#"AND (NOT "span" &> $4)"#,
        ]
        .join(" ")
    );
    let ids = built(db, condition).await?;
    assert_eq!(
        ids,
        written(
            db,
            "(label !~ '^s') AND ((span -|- '[8,9)'::int4range) OR (doc @? '$.a')) \
             AND NOT (span &> '[9,10)'::int4range)"
        )
        .await?
    );
    assert_eq!(ids, vec![1, 2]);
    Ok(())
}
