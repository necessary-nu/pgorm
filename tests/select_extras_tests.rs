#![allow(unused_imports, dead_code)]

//! `FETCH FIRST .. WITH TIES`, `GROUP BY DISTINCT`, `SIMILAR TO`, `WITH
//! ORDINALITY` / `ROWS FROM` and `TABLESAMPLE`, against a live PostgreSQL
//! server.
//!
//! The render tests settle that each construct parses as the node it was
//! written as. What they cannot settle is what the server does with it, and
//! each wrong rendering here still parses: a tie limit read as a plain one
//! loses the tied rows, a dropped `DISTINCT` repeats groups, an ordinal column
//! numbered from the wrong end or a sample whose seed is ignored changes rows,
//! not syntax. So every case counts or reads rows, with the plain form beside
//! it as the control, and every bound value travels through `build`.

pub mod common;

pub use common::{TestContext, setup::*};
use pgorm::{
    ConnectionTrait, Error, PaginatorTrait, QueryTrait, RuntimeError, SelectGetableTuple, Selector,
    entity::prelude::*,
};
use pgorm_query::{
    ColumnType, Expr, Func, GroupingElement, LikeExpr, LockBehavior, LockType, Name, Order,
    OrderedStatement, Query, SampleMethod, SelectStatement, TableFunction, TableSample,
    error::Error as QueryError,
};
use pretty_assertions::assert_eq;
use std::num::NonZeroU64;
use tokio_postgres::error::SqlState;

mod player {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "player")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub name: String,
        pub score: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// Four players, two of them tied for second; two rows to group; a thousand
/// rows to sample.
const SEED: &str = "
    CREATE TABLE player (id int PRIMARY KEY, name text NOT NULL, score int NOT NULL);
    INSERT INTO player (id, name, score) VALUES
        (1, 'ada', 10),
        (2, 'bob', 9),
        (3, 'cy', 9),
        (4, 'di', 8);
    CREATE TABLE pair (a int, b int);
    INSERT INTO pair (a, b) VALUES (1, 2), (1, 3);
    CREATE TABLE sampled (n int);
    INSERT INTO sampled (n) SELECT generate_series(1, 1000);
";

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("select_extras_tests").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SEED).await?;

    tie_limit_keeps_rows_tied_with_the_last(&db).await?;
    refused_tie_limits_never_reach_the_server(&db).await?;
    orm_row_limits_refuse_a_tie_limit(&db).await?;
    group_by_distinct_groups_each_set_once(&db).await?;
    similar_to_matches_bound_patterns(&db).await?;
    ordinality_numbers_a_function_rows(&db).await?;
    rows_from_pads_the_shorter_function(&db).await?;
    a_record_call_reads_through_its_definitions(&db).await?;
    the_server_refuses_what_definitions_cannot_see(&db).await?;
    tablesample_draws_by_a_bound_percentage(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn col(name: &str) -> Expr {
    Expr::col(Name::runtime(name))
}

/// A bound `text[]`. PostgreSQL has an `unnest` for arrays, multiranges and
/// `tsvector`, so a bound array beside it is cast to the type it is or the
/// call is ambiguous (`42725`).
fn texts(items: &[&str]) -> pgorm_query::SimpleExpr {
    Expr::val(
        items
            .iter()
            .map(|item| (*item).to_owned())
            .collect::<Vec<_>>(),
    )
    .cast_as_type(pgorm_query::TypeName::new(Name::runtime("text")).array())
}

/// A bound `int4`, cast for the same reason: `generate_series` is overloaded
/// on its argument types.
fn int(value: i32) -> pgorm_query::SimpleExpr {
    Expr::val(value).cast_as(Name::runtime("int4"))
}

/// Run `query` through `build`, its values bound, decoding each row as `T`.
async fn rows<T>(db: &DatabaseConnection, query: SelectStatement) -> Result<Vec<T>, Error>
where
    T: pgorm::TryGetableMany,
{
    Selector::<SelectGetableTuple<T>>::into_tuple::<T>(query)
        .all(db)
        .await
}

/// `SELECT name FROM player ORDER BY score DESC`, the order every tie case
/// reads. Ties are measured on every ORDER BY key, so the score is the only
/// one, and a case sorts the names it reads.
fn by_score() -> SelectStatement {
    Query::select()
        .expr(col("name"))
        .from(Name::runtime("player"))
        .order_by_expr(col("score").into(), Order::Desc)
        .take()
}

/// The names `query` reads, sorted.
async fn names(db: &DatabaseConnection, query: SelectStatement) -> Result<Vec<String>, Error> {
    let mut names = rows::<String>(db, query).await?;
    names.sort_unstable();
    Ok(names)
}

fn sql_state(err: &Error) -> Option<&SqlState> {
    match err {
        Error::Postgres(e) => e.code(),
        _ => None,
    }
}

// [spec:pgorm:req:sql.ast.select.tie-limit+2/test]    against a live server: the tied rows are kept,
// the count and offset bound, where a plain LIMIT cuts the tie
// [spec:pgorm:req:sql.render.tie-limit/test]
async fn tie_limit_keeps_rows_tied_with_the_last(db: &DatabaseConnection) -> Result<(), Error> {
    let plain = names(db, by_score().limit(2).take()).await?;
    assert_eq!(plain.len(), 2);
    assert_eq!(plain[0], "ada");

    let tied = names(db, by_score().limit_with_ties(2).take()).await?;
    assert_eq!(tied, ["ada", "bob", "cy"]);

    let offset = names(db, by_score().limit_with_ties(1).offset(1).take()).await?;
    assert_eq!(offset, ["bob", "cy"]);

    let none = names(db, by_score().limit_with_ties(0).take()).await?;
    assert!(none.is_empty());

    let locked = names(
        db,
        by_score()
            .limit_with_ties(2)
            .lock_with_behavior(LockType::Update, LockBehavior::Nowait)
            .take(),
    )
    .await?;
    assert_eq!(locked, ["ada", "bob", "cy"]);

    Ok(())
}

// [spec:pgorm:req:sql.ast.select.tie-limit+2/test]    the two refused shapes come back as builder
// errors before anything is sent, where the server would refuse their text with 42601
// [spec:pgorm:req:exec.tie-limit+1/test]
async fn refused_tie_limits_never_reach_the_server(db: &DatabaseConnection) -> Result<(), Error> {
    let unordered = Query::select()
        .expr(col("name"))
        .from(Name::runtime("player"))
        .limit_with_ties(2)
        .take();
    let sent = db
        .query_all(unordered.to_string().as_str(), &[])
        .await
        .expect_err("the server refuses WITH TIES without ORDER BY");
    assert_eq!(sql_state(&sent), Some(&SqlState::SYNTAX_ERROR));
    let refused = rows::<String>(db, unordered)
        .await
        .expect_err("a tie limit without an order is refused");
    assert!(
        matches!(refused, Error::QueryBuilder(QueryError::TiesWithoutOrder)),
        "{refused:?}"
    );

    let skipping = by_score()
        .limit_with_ties(2)
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .take();
    let sent = db
        .query_all(skipping.to_string().as_str(), &[])
        .await
        .expect_err("the server refuses WITH TIES beside SKIP LOCKED");
    assert_eq!(sql_state(&sent), Some(&SqlState::SYNTAX_ERROR));
    let refused = rows::<String>(db, skipping)
        .await
        .expect_err("a tie limit beside SKIP LOCKED is refused");
    assert!(
        matches!(refused, Error::QueryBuilder(QueryError::TiesWithSkipLocked)),
        "{refused:?}"
    );

    Ok(())
}

fn refusal_message(err: Error) -> String {
    match err {
        Error::Query(RuntimeError::Internal(message)) => message,
        other => panic!("expected Error::Query(RuntimeError::Internal(..)), got {other:?}"),
    }
}

// [spec:pgorm:req:exec.tie-limit+1/test]    the paginator and the cursor refuse a tie limit rather
// than replace it, `one` reads the first row of the order, and `all` keeps the ties
async fn orm_row_limits_refuse_a_tie_limit(db: &DatabaseConnection) -> Result<(), Error> {
    let mut tied = player::Entity::find().order_by_desc(player::Column::Score);
    QueryTrait::query(&mut tied).limit_with_ties(2);

    let all = tied.clone().all(db).await?;
    assert_eq!(all.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2, 3]);

    let first = tied.clone().one(db).await?;
    assert_eq!(first.name, "ada");

    let page_size = NonZeroU64::new(2).expect("non-zero");
    let paginator = tied.clone().paginate(db, page_size);
    let page = refusal_message(paginator.fetch_page(0).await.expect_err("a page"));
    assert!(page.contains("paginator"), "{page}");
    let count = refusal_message(paginator.num_items().await.expect_err("a count"));
    assert!(count.contains("WITH TIES"), "{count}");

    let cursor = refusal_message(
        tied.cursor_by(player::Column::Id)
            .first(2)
            .all(db)
            .await
            .expect_err("a cursor window"),
    );
    assert!(cursor.contains("cursor"), "{cursor}");

    Ok(())
}

/// `SELECT GROUPING(a, b), count(a) FROM pair GROUP BY ROLLUP (a, b), ROLLUP
/// (a)`, deduplicated or not.
fn rolled_up(distinct: bool) -> SelectStatement {
    let mut query = Query::select()
        .expr(Func::grouping(col("a")).arg(col("b")))
        .expr(Func::count(col("a")))
        .from(Name::runtime("pair"))
        .group_by_element(GroupingElement::rollup([col("a"), col("b")]))
        .group_by_element(GroupingElement::rollup([col("a")]))
        .take();
    if distinct {
        query.group_by_distinct();
    }
    query
}

// [spec:pgorm:req:sql.ast.select.group-distinct/test]    against a live server: the list
// `ROLLUP (a, b), ROLLUP (a)` reaches `(a, b)` twice, `(a)` three times and `()` once, and
// `DISTINCT` groups by each once
async fn group_by_distinct_groups_each_set_once(db: &DatabaseConnection) -> Result<(), Error> {
    let masks = |rows: Vec<(i32, i64)>| {
        let mut masks = rows.into_iter().map(|(mask, _)| mask).collect::<Vec<_>>();
        masks.sort_unstable();
        masks
    };

    let repeated = rows::<(i32, i64)>(db, rolled_up(false)).await?;
    assert_eq!(masks(repeated), [0, 0, 0, 0, 1, 1, 1, 3]);

    let distinct = rows::<(i32, i64)>(db, rolled_up(true)).await?;
    assert_eq!(masks(distinct), [0, 0, 1, 3]);

    Ok(())
}

// [spec:pgorm:req:sql.ast.expr.similar-to+2/test]    against a live server: the bound pattern is
// matched whole, alternation and escape included
async fn similar_to_matches_bound_patterns(db: &DatabaseConnection) -> Result<(), Error> {
    let named = |predicate: pgorm_query::SimpleExpr| {
        Query::select()
            .expr(col("name"))
            .from(Name::runtime("player"))
            .and_where(predicate)
            .order_by_expr(col("id").into(), Order::Asc)
            .take()
    };

    let matched = rows::<String>(db, named(col("name").similar_to("(ada|cy)"))).await?;
    assert_eq!(matched, ["ada", "cy"]);

    // SIMILAR TO matches the whole string, where LIKE's `%` would be needed.
    let partial = rows::<String>(db, named(col("name").similar_to("a"))).await?;
    assert!(partial.is_empty());

    let negated = rows::<String>(db, named(col("name").not_similar_to("_{2}"))).await?;
    assert_eq!(negated, ["ada", "bob"]);

    // A bound left operand has no type of its own beside a bound pattern, so
    // it is cast to the text it is.
    let text = |value: &str| Expr::val(value).cast_as(Name::runtime("text"));
    let escaped = Query::select()
        .expr(text("50%").similar_to(LikeExpr::new("50!%").escape('!')))
        .expr(text("500").similar_to(LikeExpr::new("50!%").escape('!')))
        .take();
    assert_eq!(rows::<(bool, bool)>(db, escaped).await?, [(true, false)]);

    Ok(())
}

// [spec:pgorm:req:sql.ast.select.table-function+2/test]    against a live server: `WITH ORDINALITY`
// numbers an array's elements in order, under the column names the alias gives
// [spec:pgorm:req:sql.render.table-function+2/test]
async fn ordinality_numbers_a_function_rows(db: &DatabaseConnection) -> Result<(), Error> {
    let numbered = Query::select()
        .expr(col("v"))
        .expr(col("n"))
        .from(
            TableFunction::new(Func::unnest(texts(&["x", "y", "z"])))
                .with_ordinality()
                .columns([Name::runtime("v"), Name::runtime("n")])
                .alias(Name::runtime("t")),
        )
        .take();
    assert_eq!(
        rows::<(String, i64)>(db, numbered).await?,
        [
            ("x".to_owned(), 1),
            ("y".to_owned(), 2),
            ("z".to_owned(), 3)
        ]
    );

    let counted_down = Query::select()
        .expr(col("v"))
        .from(
            TableFunction::new(Func::generate_series_step(int(10), int(1), int(-4)))
                .columns([Name::runtime("v")])
                .alias(Name::runtime("s")),
        )
        .take();
    assert_eq!(rows::<i32>(db, counted_down).await?, [10, 6, 2]);

    let too_many = Query::select()
        .expr(Expr::val(1))
        .from(
            TableFunction::new(Func::generate_series(int(1), int(2)))
                .with_ordinality()
                .columns(["a", "b", "c"].map(Name::runtime))
                .alias(Name::runtime("t")),
        )
        .take();
    let refused = rows::<i32>(db, too_many)
        .await
        .expect_err("three names for two columns");
    assert_eq!(
        sql_state(&refused),
        Some(&SqlState::INVALID_COLUMN_REFERENCE)
    );

    Ok(())
}

// [spec:pgorm:req:sql.ast.select.table-function+2/test]    against a live server: `ROWS FROM` reads
// its calls side by side and pads the one that runs out with NULLs
// [spec:pgorm:req:sql.render.table-function+2/test]
async fn rows_from_pads_the_shorter_function(db: &DatabaseConnection) -> Result<(), Error> {
    let zipped = Query::select()
        .expr(col("x"))
        .expr(col("y"))
        .expr(col("n"))
        .from(
            TableFunction::rows_from(Func::generate_series(int(1), int(2)))
                .and(Func::unnest(texts(&["a", "b", "c"])))
                .with_ordinality()
                .columns(["x", "y", "n"].map(Name::runtime))
                .alias(Name::runtime("t")),
        )
        .take();
    assert_eq!(
        rows::<(Option<i32>, String, i64)>(db, zipped).await?,
        [
            (Some(1), "a".to_owned(), 1),
            (Some(2), "b".to_owned(), 2),
            (None, "c".to_owned(), 3)
        ]
    );

    Ok(())
}

/// A bound JSON document, which the record functions' one signature types
/// `json` or `jsonb`.
fn document(value: serde_json::Value) -> pgorm_query::SimpleExpr {
    Expr::val(value).into()
}

// [spec:pgorm:req:sql.ast.select.column-definitions/test]    against a live server:
// `json_to_record` and `jsonb_to_recordset` read a bound document through the columns their lists
// define, a missing key reading NULL, a lone list moved into `ROWS FROM` beside an ordinal, and
// each call of a `ROWS FROM` through its own list
// [spec:pgorm:req:sql.render.column-definitions/test]
async fn a_record_call_reads_through_its_definitions(db: &DatabaseConnection) -> Result<(), Error> {
    let one = Query::select()
        .expr(col("a"))
        .expr(col("b"))
        .expr(col("tags"))
        .from(
            TableFunction::new(Func::json_to_record(document(
                serde_json::json!({"a": 7, "b": "seven", "tags": ["x", "y"]}),
            )))
            .column_definition(Name::runtime("a"), ColumnType::Integer)
            .column_definition(Name::runtime("b"), ColumnType::Text)
            .column_definition(
                Name::runtime("tags"),
                ColumnType::Array(std::sync::Arc::new(ColumnType::Text)),
            )
            .alias(Name::runtime("t")),
        )
        .take();
    assert_eq!(
        rows::<(i32, String, Vec<String>)>(db, one).await?,
        [(7, "seven".to_owned(), vec!["x".to_owned(), "y".to_owned()])]
    );

    let numbered = Query::select()
        .expr(col("a"))
        .expr(col("missing"))
        .expr(col("ordinality"))
        .from(
            TableFunction::new(Func::jsonb_to_recordset(document(
                serde_json::json!([{"a": 2}, {"a": 1}, {"a": 3}]),
            )))
            .column_definition(Name::runtime("a"), ColumnType::Integer)
            .column_definition(Name::runtime("missing"), ColumnType::Text)
            .with_ordinality()
            .alias(Name::runtime("t")),
        )
        .take();
    assert!(
        numbered
            .to_string()
            .contains("ROWS FROM (JSONB_TO_RECORDSET("),
        "{numbered}"
    );
    assert_eq!(
        rows::<(i32, Option<String>, i64)>(db, numbered).await?,
        [(2, None, 1), (1, None, 2), (3, None, 3)]
    );

    let zipped = Query::select()
        .expr(col("a"))
        .expr(col("b"))
        .expr(col("s"))
        .from(
            TableFunction::new(Func::json_to_recordset(document(
                serde_json::json!([{"a": 1}, {"a": 2}]),
            )))
            .column_definition(Name::runtime("a"), ColumnType::Integer)
            .and(Func::jsonb_to_recordset(document(
                serde_json::json!([{"b": "x"}, {"b": "y"}, {"b": "z"}]),
            )))
            .column_definition(Name::runtime("b"), ColumnType::Text)
            .and(Func::generate_series(int(10), int(11)))
            .columns(["a", "b", "s"].map(Name::runtime))
            .alias(Name::runtime("t")),
        )
        .take();
    assert_eq!(
        rows::<(Option<i32>, String, Option<i32>)>(db, zipped).await?,
        [
            (Some(1), "x".to_owned(), Some(10)),
            (Some(2), "y".to_owned(), Some(11)),
            (None, "z".to_owned(), None)
        ]
    );

    Ok(())
}

// [spec:pgorm:req:sql.ast.select.column-definitions/test]    against a live server: which function
// returns `record` is the server's to know, so a missing list, a list on another function and two
// columns of one name are its refusals
async fn the_server_refuses_what_definitions_cannot_see(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let read = |function: TableFunction| {
        Query::select()
            .expr(Expr::val(1))
            .from(function.alias(Name::runtime("t")))
            .take()
    };
    let record = || Func::json_to_record(document(serde_json::json!({"a": 1})));
    let refusals = [
        (read(TableFunction::new(record())), SqlState::SYNTAX_ERROR),
        (
            read(
                TableFunction::new(Func::generate_series(int(1), int(2)))
                    .column_definition(Name::runtime("a"), ColumnType::Integer),
            ),
            SqlState::SYNTAX_ERROR,
        ),
        (
            read(
                TableFunction::new(record())
                    .column_definition(Name::runtime("a"), ColumnType::Integer)
                    .column_definition(Name::runtime("a"), ColumnType::Text),
            ),
            SqlState::DUPLICATE_COLUMN,
        ),
    ];
    for (query, state) in refusals {
        let refused = rows::<i32>(db, query.clone())
            .await
            .expect_err("the server refuses it");
        assert_eq!(sql_state(&refused), Some(&state), "{query}");
    }

    Ok(())
}

/// The `n` of every `sampled` row a `TABLESAMPLE` draws, in order.
async fn sample(db: &DatabaseConnection, sample: TableSample) -> Result<Vec<i32>, Error> {
    rows::<i32>(
        db,
        Query::select()
            .expr(col("n"))
            .from(sample)
            .order_by_expr(col("n").into(), Order::Asc)
            .take(),
    )
    .await
}

// [spec:pgorm:req:sql.ast.select.table-sample/test]    against a live server: the bound
// percentage draws everything at 100 and nothing at 0, a seed draws the same sample twice, and
// out of range is the server's refusal
// [spec:pgorm:req:sql.render.table-sample/test]
async fn tablesample_draws_by_a_bound_percentage(db: &DatabaseConnection) -> Result<(), Error> {
    let sampled = || Name::runtime("sampled");

    let all = sample(
        db,
        TableSample::new(sampled(), SampleMethod::Bernoulli, 100),
    )
    .await?;
    assert_eq!(all.len(), 1000);
    let pages = sample(db, TableSample::new(sampled(), SampleMethod::System, 100)).await?;
    assert_eq!(pages.len(), 1000);
    let nothing = sample(db, TableSample::new(sampled(), SampleMethod::Bernoulli, 0)).await?;
    assert!(nothing.is_empty());

    let seeded =
        || TableSample::new(sampled(), SampleMethod::Bernoulli, 37.5).repeatable(20_261_018);
    let first = sample(db, seeded()).await?;
    let again = sample(db, seeded()).await?;
    assert_eq!(first, again);
    assert!(!first.is_empty() && first.len() < 1000, "{}", first.len());

    let aliased = TableSample::new(
        pgorm_query::IntoNamedTable::into_named_table(sampled()).alias(Name::runtime("s")),
        SampleMethod::System,
        100,
    );
    let qualified = Query::select()
        .expr(Expr::col((Name::runtime("s"), Name::runtime("n"))))
        .from(aliased)
        .and_where(Expr::col((Name::runtime("s"), Name::runtime("n"))).lte(3))
        .order_by_expr(col("n").into(), Order::Asc)
        .take();
    assert_eq!(rows::<i32>(db, qualified).await?, [1, 2, 3]);

    let refused = sample(
        db,
        TableSample::new(sampled(), SampleMethod::Bernoulli, 150),
    )
    .await
    .expect_err("a percentage over 100");
    assert_eq!(
        sql_state(&refused),
        Some(&SqlState::INVALID_TABLESAMPLE_ARGUMENT)
    );

    Ok(())
}
