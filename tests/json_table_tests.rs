#![allow(unused_imports, dead_code)]

//! `JSON_TABLE` against a live PostgreSQL 19 server: the rows and columns a
//! document becomes, what each behaviour does to a row, that its paths are
//! literals the server accepts and a hostile one stays data, and what the
//! server refuses that the builder cannot see.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Asterisk, ColumnType, Condition, Expr, Func, JoinType, JsonExistsBehavior, JsonQueryBehavior,
    JsonTableBehavior, JsonTableColumn, JsonValueBehavior, Name, Order, Query, SelectStatement,
    SimpleExpr, Values,
};
use pgorm::{ConnectionTrait, entity::prelude::*};
use pgorm_driver::error::SqlState;
use pretty_assertions::assert_eq;
use serde_json::json;

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn doc() -> Expr {
    Expr::col((n("d"), n("doc")))
}

/// A table `docs (id, doc jsonb)` holding two documents.
async fn docs(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(
        r#"CREATE TABLE docs (id integer PRIMARY KEY, doc jsonb NOT NULL);
           INSERT INTO docs VALUES
             (1, '{"items": [{"n": 1, "label": "one", "tags": ["a"], "flag": true, "parts": ["x", "y"]},
                             {"n": 2, "tags": [], "parts": []}]}'),
             (2, '{"items": [{"n": "three", "label": "three"}]}')"#,
    )
    .await
}

/// Every row `statement` yields, through both render paths, which must agree.
async fn rows<T>(db: &DatabaseConnection, statement: &SelectStatement) -> Result<Vec<T>, Error>
where
    T: pgorm::TryGetableMany + PartialEq + std::fmt::Debug,
{
    let bound = statement.build().into_tuple::<T>().all(db).await?;
    let inlined = (statement.to_string(), Values(vec![]))
        .into_tuple::<T>()
        .all(db)
        .await?;
    assert_eq!(bound, inlined, "{statement}");
    Ok(bound)
}

/// Each column kind reads what it names out of each item, the table reading
/// its context from the FROM item before it, and a nested path's rows joining
/// their parent's as an outer join would.
// [spec:pgorm:def:sql.ast.json-table+3/test]    the column kinds, lateral, and NESTED as an outer
// join, live
// [spec:pgorm:req:sql.render.json-table/test]
#[pgorm_macros::test]
async fn a_document_becomes_rows_and_columns() -> Result<(), Error> {
    let ctx = TestContext::new("json_table_rows").await;
    let db = ctx.db.get().await?;
    docs(&db).await?;

    let table = Func::json_table(doc(), "$.items[*]", JsonTableColumn::ordinality(n("i")))
        .column(JsonTableColumn::value(n("n"), ColumnType::Integer))
        .column(JsonTableColumn::value(n("label"), ColumnType::Text))
        .column(
            JsonTableColumn::query(n("tags"), ColumnType::JsonBinary)
                .on_empty(JsonQueryBehavior::EmptyArray),
        )
        .column(JsonTableColumn::exists(n("flagged"), ColumnType::Boolean).path("$.flag"))
        .column(JsonTableColumn::nested(
            "$.parts[*]",
            JsonTableColumn::value(n("part"), ColumnType::Text).path("$"),
        ));
    let statement = Query::select()
        .column((n("d"), n("id")))
        .columns([
            n("i"),
            n("n"),
            n("label"),
            n("tags"),
            n("flagged"),
            n("part"),
        ])
        .from_as(n("docs"), n("d"))
        .from(table.alias(n("jt")))
        .order_by((n("d"), n("id")), Order::Asc)
        .order_by(n("i"), Order::Asc)
        .order_by(n("part"), Order::Asc)
        .take();
    type Row = (
        i32,
        i32,
        Option<i32>,
        Option<String>,
        Json,
        bool,
        Option<String>,
    );
    assert_eq!(
        rows::<Row>(&db, &statement).await?,
        vec![
            (
                1,
                1,
                Some(1),
                Some("one".into()),
                json!(["a"]),
                true,
                Some("x".into())
            ),
            (
                1,
                1,
                Some(1),
                Some("one".into()),
                json!(["a"]),
                true,
                Some("y".into())
            ),
            (1, 2, Some(2), None, json!([]), false, None),
            (2, 1, None, Some("three".into()), json!([]), false, None),
        ],
        "a value that does not convert is NULL by default; an item with no parts keeps its row"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The behaviours: a column's ON EMPTY and ON ERROR, an EXISTS column's ON
/// ERROR, and the table's own ON ERROR for a failing root path.
// [spec:pgorm:def:sql.ast.json-table+3/test]    the column and table behaviours, live
#[pgorm_macros::test]
async fn behaviours_decide_what_a_row_holds() -> Result<(), Error> {
    let ctx = TestContext::new("json_table_behaviours").await;
    let db = ctx.db.get().await?;
    docs(&db).await?;
    let over = |column: JsonTableColumn| {
        Query::select()
            .column(n("v"))
            .from_as(n("docs"), n("d"))
            .from(Func::json_table(doc(), "$.items[*]", column).alias(n("jt")))
            .and_where(Expr::col((n("d"), n("id"))).eq(2))
            .take()
    };

    let defaulted: Vec<i32> = rows(
        &db,
        &over(
            JsonTableColumn::value(n("v"), ColumnType::Integer)
                .path("$.n")
                .on_error(JsonValueBehavior::Default((-1).into()))
                .into(),
        ),
    )
    .await?;
    assert_eq!(defaulted, [-1]);
    let error = rows::<i32>(
        &db,
        &over(
            JsonTableColumn::value(n("v"), ColumnType::Integer)
                .path("$.n")
                .on_error(JsonValueBehavior::Error)
                .into(),
        ),
    )
    .await
    .expect_err("'three' is no integer");
    refused_with(&error, &SqlState::INVALID_TEXT_REPRESENTATION);
    let empty: Vec<String> = rows(
        &db,
        &over(
            JsonTableColumn::value(n("v"), ColumnType::Text)
                .path("$.missing")
                .on_empty(JsonValueBehavior::Default("none".into()))
                .into(),
        ),
    )
    .await?;
    assert_eq!(empty, ["none"]);
    let unknown: Vec<Option<bool>> = rows(
        &db,
        &over(
            JsonTableColumn::exists(n("v"), ColumnType::Boolean)
                .path("strict $.missing")
                .on_error(JsonExistsBehavior::Unknown)
                .into(),
        ),
    )
    .await?;
    assert_eq!(unknown, [None]);

    let strict = |behavior: Option<JsonTableBehavior>| {
        let mut table = Func::json_table(
            doc(),
            "strict $.missing[*]",
            JsonTableColumn::ordinality(n("v")),
        );
        if let Some(behavior) = behavior {
            table = table.on_error(behavior);
        }
        Query::select()
            .column(n("v"))
            .from_as(n("docs"), n("d"))
            .from(table.alias(n("jt")))
            .take()
    };
    assert_eq!(rows::<i32>(&db, &strict(None)).await?, Vec::<i32>::new());
    assert_eq!(
        rows::<i32>(&db, &strict(Some(JsonTableBehavior::Empty))).await?,
        Vec::<i32>::new()
    );
    let error = rows::<i32>(&db, &strict(Some(JsonTableBehavior::Error)))
        .await
        .expect_err("ERROR ON ERROR raises");
    refused_with(&error, &SqlState::from_code("2203A"));

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A path is a literal the server accepts — it refuses a parameter there —
/// and a path holding a quote and a backslash stays one literal, selecting the
/// key it names.
// [spec:pgorm:req:sql.render.json-table/test]    paths are escaped literals, which the server
// requires (0A000 for a bound root path, 42601 for a bound column path)
#[pgorm_macros::test]
async fn paths_are_escaped_literals_the_server_requires() -> Result<(), Error> {
    let ctx = TestContext::new("json_table_path_literals").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        r#"CREATE TABLE docs (id integer, doc jsonb);
           INSERT INTO docs VALUES (1, '{"it''s \\ here": {"v": "found"}}')"#,
    )
    .await?;

    let key = r#"$."it's \\ here""#;
    let statement = Query::select()
        .column(n("v"))
        .from_as(n("docs"), n("d"))
        .from(
            Func::json_table(
                doc(),
                key,
                JsonTableColumn::value(n("v"), ColumnType::Text).path("$.v"),
            )
            .alias(n("jt")),
        )
        .take();
    assert_eq!(rows::<String>(&db, &statement).await?, ["found"]);

    let error = db
        .query_all(
            r#"SELECT * FROM docs d, JSON_TABLE(d.doc, $1 COLUMNS (v text)) jt"#,
            &[&"$"],
        )
        .await
        .expect_err("a bound root path");
    refused_with(&error, &SqlState::FEATURE_NOT_SUPPORTED);
    let error = db
        .query_all(
            r#"SELECT * FROM docs d, JSON_TABLE(d.doc, '$' COLUMNS (v text PATH $1)) jt"#,
            &[&"$"],
        )
        .await
        .expect_err("a bound column path");
    refused_with(&error, &SqlState::SYNTAX_ERROR);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `LEFT JOIN JSON_TABLE(..) ON TRUE` keeps a row whose document yields
/// nothing, and a duplicate column name is the server's refusal.
// [spec:pgorm:def:sql.ast.json-table+3/test]    a JSON_TABLE joined, and a duplicate name (42712)
#[pgorm_macros::test]
async fn json_table_joins_and_names_are_unique() -> Result<(), Error> {
    let ctx = TestContext::new("json_table_join").await;
    let db = ctx.db.get().await?;
    docs(&db).await?;

    let statement = Query::select()
        .column((n("d"), n("id")))
        .column(n("label"))
        .from_as(n("docs"), n("d"))
        .join(
            JoinType::LeftJoin,
            Func::json_table(
                doc(),
                "$.items[*] ? (@.flag == true)",
                JsonTableColumn::value(n("label"), ColumnType::Text),
            )
            .alias(n("jt")),
            SimpleExpr::Constant(true.into()),
        )
        .order_by((n("d"), n("id")), Order::Asc)
        .take();
    assert_eq!(
        rows::<(i32, Option<String>)>(&db, &statement).await?,
        [(1, Some("one".to_owned())), (2, None)]
    );

    let duplicate = Query::select()
        .column(Asterisk)
        .from_as(n("docs"), n("d"))
        .from(
            Func::json_table(doc(), "$", JsonTableColumn::ordinality(n("x")))
                .column(JsonTableColumn::value(n("x"), ColumnType::Text))
                .alias(n("jt")),
        )
        .take();
    let error = rows::<(i32,)>(&db, &duplicate).await.expect_err("x twice");
    refused_with(&error, &SqlState::DUPLICATE_ALIAS);

    let formatted_integer = Query::select()
        .column(Asterisk)
        .from_as(n("docs"), n("d"))
        .from(
            Func::json_table(
                doc(),
                "$.items[*]",
                JsonTableColumn::query(n("n"), ColumnType::Integer),
            )
            .alias(n("jt")),
        )
        .take();
    let error = rows::<(i32,)>(&db, &formatted_integer)
        .await
        .expect_err("FORMAT JSON on an integer column");
    refused_with(&error, &SqlState::FEATURE_NOT_SUPPORTED);

    drop(db);
    ctx.delete().await;
    Ok(())
}
