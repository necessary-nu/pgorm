#![allow(unused_imports, dead_code)]

//! PostgreSQL's SQL/JSON against a live PostgreSQL 18 server.
//!
//! The render tests in pgorm-query hold each form to libpg_query's grammar.
//! Only a server settles what a form returns: that a bound path selects what it
//! names and a `PASSING` name is matched exactly, that each behaviour answers
//! as specified and the server refuses what the types leave out, that a value
//! builds the same JSON bound as inlined, and — the decision the old operators
//! rest on — that a GIN or expression index serves the operators and not the
//! SQL/JSON functions.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    ColumnType, Expr, Func, JsonExistsBehavior, JsonKind, JsonQueryBehavior, JsonValueBehavior,
    JsonValueType, Name, Order, Query, SelectStatement, SimpleExpr, Values, WindowStatement,
};
use pgorm::{ConnectionTrait, entity::prelude::*};
use pretty_assertions::assert_eq;
use serde_json::json;
use tokio_postgres::error::SqlState;

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

fn doc() -> Expr {
    Expr::col(Name::runtime("doc"))
}

/// A one-row, one-column table `t` holding `value` as `doc`, of type `ty`.
async fn holding(db: &DatabaseConnection, ty: &str, value: &str) -> Result<(), Error> {
    db.batch_execute(&format!(
        "DROP TABLE IF EXISTS t; CREATE TABLE t (doc {ty}); INSERT INTO t VALUES ('{value}')"
    ))
    .await
}

/// `expr` projected from `t`, decoded as `T`, through the bound render.
async fn read<T>(db: &DatabaseConnection, expr: impl Into<SimpleExpr>) -> Result<T, Error>
where
    T: pgorm::TryGetableMany,
{
    Query::select()
        .expr(expr)
        .from(Name::runtime("t"))
        .build()
        .into_tuple::<T>()
        .one(db)
        .await
}

/// `expr` projected with no FROM, through both render paths, which must agree.
async fn both_paths<T>(db: &DatabaseConnection, expr: impl Into<SimpleExpr>) -> Result<T, Error>
where
    T: pgorm::TryGetableMany + PartialEq + std::fmt::Debug,
{
    let statement = Query::select().expr(expr).take();
    let bound = statement.build().into_tuple::<T>().one(db).await?;
    let inlined = (statement.to_string(), Values(vec![]))
        .into_tuple::<T>()
        .one(db)
        .await?;
    assert_eq!(bound, inlined, "{statement}");
    Ok(bound)
}

/// A bound path selects what it names — including a key holding a quote,
/// which an interpolated path would end — and a `PASSING` variable is the
/// name as written, case and all.
// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    the query functions over a live document
// [spec:pgorm:req:sql.render.sql-json+1/test]    the path is bound, never part of the text
#[pgorm_macros::test]
async fn bound_paths_and_variables_select_what_they_name() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_bound_paths").await;
    let db = ctx.db.get().await?;
    holding(
        &db,
        "jsonb",
        r#"{"size": 12, "tags": ["red", "blue"], "it''s": "quoted"}"#,
    )
    .await?;

    let size: i32 = read(
        &db,
        Func::json_value(doc(), "$.size").returning(JsonValueType::Integer),
    )
    .await?;
    assert_eq!(size, 12);

    let quoted: String = read(&db, Func::json_value(doc(), r#"$."it's""#)).await?;
    assert_eq!(quoted, "quoted");

    let blue: bool = read(
        &db,
        Func::json_exists(doc(), "$.tags[*] ? (@ == $Tag)").passing("blue", Name::runtime("Tag")),
    )
    .await?;
    let green: bool = read(
        &db,
        Func::json_exists(doc(), "$.tags[*] ? (@ == $Tag)").passing("green", Name::runtime("Tag")),
    )
    .await?;
    assert_eq!((blue, green), (true, false));

    let over: Json = read(
        &db,
        Func::json_query(doc(), "$.size ? (@ > $min)")
            .passing(10, Name::runtime("min"))
            .with_wrapper(),
    )
    .await?;
    assert_eq!(over, json!([12]));

    // A variable the path reads and nothing passes fails when it is read.
    let unpassed = read::<bool>(&db, Func::json_exists(doc(), "$ ? (@.size > $min)"))
        .await
        .expect_err("$min is passed by no PASSING");
    assert!(unpassed.to_string().contains("min"), "{unpassed}");

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Each behaviour answers as specified, and each function's default is what
/// an absent clause gives.
// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    the behaviours of the three query functions
#[pgorm_macros::test]
async fn behaviours_answer_as_specified() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_behaviours").await;
    let db = ctx.db.get().await?;
    holding(
        &db,
        "jsonb",
        r#"{"word": "x", "obj": {"a": 1}, "list": [1, 2]}"#,
    )
    .await?;

    // JSON_EXISTS: a strict path missing its key is an error, which the
    // behaviour answers; finding nothing in lax mode is simply false.
    let strict = || Func::json_exists(doc(), "strict $.missing");
    assert_eq!(read::<bool>(&db, strict()).await?, false);
    assert_eq!(
        read::<Option<bool>>(&db, strict().on_error(JsonExistsBehavior::Unknown)).await?,
        None
    );
    assert_eq!(
        read::<bool>(&db, strict().on_error(JsonExistsBehavior::True)).await?,
        true
    );
    let error = read::<bool>(&db, strict().on_error(JsonExistsBehavior::Error))
        .await
        .expect_err("ERROR ON ERROR raises");
    refused_with(&error, &SqlState::from_code("2203A"));

    // JSON_VALUE: ON EMPTY answers a missing key, ON ERROR a value that does
    // not convert or is not a scalar.
    let missing = || Func::json_value(doc(), "$.missing").returning(JsonValueType::Integer);
    assert_eq!(read::<Option<i32>>(&db, missing()).await?, None);
    assert_eq!(
        read::<i32>(
            &db,
            missing().on_empty(JsonValueBehavior::Default(7.into()))
        )
        .await?,
        7
    );
    let error = read::<i32>(&db, missing().on_empty(JsonValueBehavior::Error))
        .await
        .expect_err("ERROR ON EMPTY raises");
    refused_with(&error, &SqlState::from_code("22035"));

    let word = || Func::json_value(doc(), "$.word").returning(JsonValueType::Integer);
    assert_eq!(read::<Option<i32>>(&db, word()).await?, None);
    assert_eq!(
        read::<i32>(
            &db,
            word()
                .on_empty(JsonValueBehavior::Default(7.into()))
                .on_error(JsonValueBehavior::Default((-1).into()))
        )
        .await?,
        -1,
        "a found value that does not convert is ON ERROR's, not ON EMPTY's"
    );
    let error = read::<i32>(&db, word().on_error(JsonValueBehavior::Error))
        .await
        .expect_err("ERROR ON ERROR raises");
    refused_with(&error, &SqlState::INVALID_TEXT_REPRESENTATION);
    let error = read::<String>(
        &db,
        Func::json_value(doc(), "$.obj").on_error(JsonValueBehavior::Error),
    )
    .await
    .expect_err("an object is no scalar");
    refused_with(&error, &SqlState::from_code("2203F"));

    // JSON_QUERY: the empty-container behaviours, the two wrappers, and OMIT
    // QUOTES returning text.
    let none = || Func::json_query(doc(), "$.missing");
    assert_eq!(
        read::<Json>(&db, none().on_empty(JsonQueryBehavior::EmptyArray)).await?,
        json!([])
    );
    assert_eq!(
        read::<Json>(&db, none().on_empty(JsonQueryBehavior::EmptyObject)).await?,
        json!({})
    );
    let items = || Func::json_query(doc(), "$.list[*]");
    assert_eq!(read::<Option<Json>>(&db, items()).await?, None);
    let error = read::<Json>(&db, items().on_error(JsonQueryBehavior::Error))
        .await
        .expect_err("several items without a wrapper");
    refused_with(&error, &SqlState::from_code("22034"));
    assert_eq!(
        read::<Json>(&db, items().with_conditional_wrapper()).await?,
        json!([1, 2])
    );
    assert_eq!(
        read::<Json>(
            &db,
            Func::json_query(doc(), "$.list[0]").with_conditional_wrapper()
        )
        .await?,
        json!(1),
        "one item is not wrapped conditionally"
    );
    assert_eq!(
        read::<Json>(&db, Func::json_query(doc(), "$.list[0]").with_wrapper()).await?,
        json!([1])
    );
    assert_eq!(
        read::<String>(
            &db,
            Func::json_query(doc(), "$.word")
                .returning(ColumnType::Text)
                .omit_quotes()
        )
        .await?,
        "x"
    );
    assert_eq!(
        read::<String>(
            &db,
            Func::json_query(doc(), "$.word").returning(ColumnType::Text)
        )
        .await?,
        r#""x""#
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A `DEFAULT` is a literal even under the bound render — the server refuses
/// a parameter there — and a hostile one stays a string.
// [spec:pgorm:req:sql.render.sql-json+1/test]    the DEFAULT literal is escaped, and the server
// accepts it where it refuses a parameter (42804)
#[pgorm_macros::test]
async fn a_default_is_a_literal_the_server_accepts() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_default_literal").await;
    let db = ctx.db.get().await?;
    holding(&db, "jsonb", "{}").await?;

    let hostile = "x'); DROP TABLE t; --\\";
    let answer: String = read(
        &db,
        Func::json_value(doc(), "$.missing").on_empty(JsonValueBehavior::Default(hostile.into())),
    )
    .await?;
    assert_eq!(answer, hostile);
    let still: i64 = db.query_one("SELECT count(*) FROM t", &[]).await?.get(0);
    assert_eq!(still, 1);

    let refused = db
        .query_one(
            "SELECT JSON_VALUE(doc, '$.missing' DEFAULT $1 ON EMPTY) FROM t",
            &[&"x"],
        )
        .await
        .expect_err("a parameter is no DEFAULT");
    refused_with(&refused, &SqlState::DATATYPE_MISMATCH);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A value builds the same JSON bound as inlined: a number is a number, a
/// string a string, and a JSON document nests.
// [spec:pgorm:req:sql.render.sql-json+1/test]    a value carries its own type in every SQL/JSON
// position, so both render paths build the same JSON
#[pgorm_macros::test]
async fn values_build_the_same_json_bound_and_inlined() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_typed_values").await;
    let db = ctx.db.get().await?;

    let object: Json = both_paths(
        &db,
        Func::json_object()
            .entry("n", 5)
            .entry("s", "5")
            .entry("doc", json!({"a": [1, 2]}))
            .entry("yes", true)
            .entry("none", Option::<i32>::None)
            .returning(ColumnType::JsonBinary),
    )
    .await?;
    assert_eq!(
        object,
        json!({"n": 5, "s": "5", "doc": {"a": [1, 2]}, "yes": true, "none": null})
    );

    let array: Json = both_paths(
        &db,
        Func::json_array()
            .element(1.5)
            .element("a")
            .element(json!({"b": null}))
            .element(Option::<String>::None)
            .returning(ColumnType::JsonBinary),
    )
    .await?;
    assert_eq!(array, json!([1.5, "a", {"b": null}]));

    let number: Json = both_paths(&db, Func::json_scalar(5)).await?;
    let string: Json = both_paths(&db, Func::json_scalar("5")).await?;
    assert_eq!((number, string), (json!(5), json!("5")));

    let exists: bool = both_paths(
        &db,
        Func::json_exists(json!({"a": 3}), "$.a ? (@ > $min)").passing(2, Name::runtime("min")),
    )
    .await?;
    assert!(exists);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The constructors' clauses do what their names say, and the server refuses
/// what the builder cannot see.
// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    ON NULL, UNIQUE KEYS, RETURNING and the
// query form, live
#[pgorm_macros::test]
async fn constructor_clauses_hold_live() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_constructors").await;
    let db = ctx.db.get().await?;

    let nothing = || Option::<i32>::None;
    let kept: Json = both_paths(
        &db,
        Func::json_object()
            .entry("a", nothing())
            .returning(ColumnType::JsonBinary),
    )
    .await?;
    let absent: Json = both_paths(
        &db,
        Func::json_object()
            .entry("a", nothing())
            .absent_on_null()
            .returning(ColumnType::JsonBinary),
    )
    .await?;
    assert_eq!((kept, absent), (json!({"a": null}), json!({})));

    let dropped: Json = both_paths(&db, Func::json_array().element(nothing())).await?;
    let nulled: Json =
        both_paths(&db, Func::json_array().element(nothing()).null_on_null()).await?;
    assert_eq!((dropped, nulled), (json!([]), json!([null])));

    let error = both_paths::<Json>(
        &db,
        Func::json_object()
            .entry("a", 1)
            .entry("a", 2)
            .with_unique_keys(),
    )
    .await
    .expect_err("a repeated key");
    refused_with(&error, &SqlState::from_code("22030"));
    let repeated: String = both_paths(
        &db,
        Func::json_object()
            .entry("a", 1)
            .entry("a", 2)
            .returning(ColumnType::Text),
    )
    .await?;
    assert_eq!(repeated, r#"{"a" : 1, "a" : 2}"#);
    let error = both_paths::<Json>(&db, Func::json_object().entry(Option::<String>::None, 1))
        .await
        .expect_err("a NULL key");
    refused_with(&error, &SqlState::NULL_VALUE_NOT_ALLOWED);

    let series_array = |upper: i32| {
        Func::json_array_query(
            Query::select()
                .column(Name::runtime("x"))
                .from_function(
                    Func::named(Name::runtime("generate_series")).args([
                        Expr::val(1).cast_as(Name::runtime("int4")),
                        Expr::val(upper).cast_as(Name::runtime("int4")),
                    ]),
                    Name::runtime("x"),
                )
                .order_by(Name::runtime("x"), Order::Desc)
                .take(),
        )
        .returning(ColumnType::JsonBinary)
    };
    let from_query: Json = both_paths(&db, series_array(3)).await?;
    assert_eq!(from_query, json!([3, 2, 1]));
    // [spec:pgorm:req:sql.target+1/test]    a query of no rows: NULL on 18, the
    // empty array on 19
    let from_no_rows: Option<Json> = both_paths(&db, series_array(0)).await?;
    #[cfg(not(feature = "pg-19"))]
    assert_eq!(
        from_no_rows, None,
        "PostgreSQL 18 answers a query of no rows with NULL, as JSON_ARRAYAGG does"
    );
    #[cfg(feature = "pg-19")]
    assert_eq!(
        from_no_rows,
        Some(json!([])),
        "PostgreSQL 19 answers a query of no rows with an empty array"
    );

    let parsed: Json = both_paths(&db, Func::json(r#"{"a": [1]}"#)).await?;
    assert_eq!(parsed, json!({"a": [1]}));
    let error = both_paths::<Json>(&db, Func::json(r#"{"a":1,"a":2}"#).with_unique_keys())
        .await
        .expect_err("a repeated key");
    refused_with(&error, &SqlState::from_code("22030"));
    let error = both_paths::<Json>(&db, Func::json("not json"))
        .await
        .expect_err("text that is not JSON");
    refused_with(&error, &SqlState::INVALID_TEXT_REPRESENTATION);
    let from_bytes: Json = both_paths(
        &db,
        Func::json(Expr::val(br#"{"b":2}"#.to_vec()).format_json()),
    )
    .await?;
    assert_eq!(from_bytes, json!({"b": 2}));

    let text: String = both_paths(&db, Func::json_serialize(Func::json(r#"{"a" : 1}"#))).await?;
    assert_eq!(text, r#"{"a" : 1}"#);
    let bytes: Vec<u8> = both_paths(
        &db,
        Func::json_serialize(Func::json("[1]")).returning(ColumnType::Bytea),
    )
    .await?;
    assert_eq!(bytes, b"[1]");

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `JSON_SERIALIZE` over a `jsonb` value writes the document, in both render
/// paths, as it does over `json`, text and `bytea`, `FORMAT JSON` or not.
// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    JSON_SERIALIZE over every operand it takes
// [spec:pgorm:req:sql.render.sql-json+1/test]    the operand read through JSON(..)
#[pgorm_macros::test]
async fn json_serialize_writes_a_jsonb_document() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_serialize_jsonb").await;
    let db = ctx.db.get().await?;

    let jsonb: String = both_paths(&db, Func::json_serialize(json!({"b": [1, 2], "a": 1}))).await?;
    assert_eq!(jsonb, r#"{"a": 1, "b": [1, 2]}"#);
    let jsonb_bytes: Vec<u8> = both_paths(
        &db,
        Func::json_serialize(json!({"a": 1})).returning(ColumnType::Bytea),
    )
    .await?;
    assert_eq!(jsonb_bytes, br#"{"a": 1}"#);
    let marked: String = both_paths(
        &db,
        Func::json_serialize(Expr::val(json!([1])).format_json()),
    )
    .await?;
    assert_eq!(marked, "[1]");

    let json: String = both_paths(
        &db,
        Func::json_serialize(Expr::val(json!({"a": 1})).cast_as(Name::runtime("json"))),
    )
    .await?;
    assert_eq!(json, r#"{"a":1}"#);
    let text: String = both_paths(&db, Func::json_serialize(r#"{"a" : 1}"#)).await?;
    assert_eq!(text, r#"{"a" : 1}"#);
    let text_marked: String = both_paths(
        &db,
        Func::json_serialize(Expr::val(r#"{"a" : 1}"#).format_json()),
    )
    .await?;
    assert_eq!(text_marked, r#"{"a" : 1}"#);
    let bytes = || Expr::val(br#"{"a" : 1}"#.to_vec());
    let from_bytes: String = both_paths(&db, Func::json_serialize(bytes())).await?;
    let from_marked_bytes: String =
        both_paths(&db, Func::json_serialize(bytes().format_json())).await?;
    assert_eq!(
        (from_bytes.as_str(), from_marked_bytes.as_str()),
        (r#"{"a" : 1}"#, r#"{"a" : 1}"#)
    );

    let error = both_paths::<String>(&db, Func::json_serialize("not json"))
        .await
        .expect_err("text that is not JSON");
    refused_with(&error, &SqlState::INVALID_TEXT_REPRESENTATION);
    let error = both_paths::<String>(&db, Func::json_serialize(5))
        .await
        .expect_err("an integer is not JSON text");
    refused_with(&error, &SqlState::CANNOT_COERCE);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// PostgreSQL 18.6's own `JSON_SERIALIZE` over a `jsonb` operand returns bytes
/// of the binary header — `\x01`, a one-member object's member count — rather
/// than the document. This is why the builder reads the operand through
/// `JSON(..)`; when a release fixes it this fails, and the `JSON(..)` can go.
// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    the 18.6 JSON_SERIALIZE defect the builder
// works around
#[pgorm_macros::test]
async fn bare_json_serialize_over_jsonb_is_broken() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_serialize_canary").await;
    let db = ctx.db.get().await?;

    let (broken, two_members): (String, String) = (
        r#"SELECT JSON_SERIALIZE('{"a": 1}'::jsonb), JSON_SERIALIZE('{"a": 1, "b": 2}'::jsonb)"#,
        Values(vec![]),
    )
        .into_tuple()
        .one(&db)
        .await?;
    assert_eq!((broken.as_str(), two_members.as_str()), ("\u{1}", "\u{2}"));

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// PostgreSQL 18.6's `JSON_VALUE .. RETURNING jsonb` (and `json`) returns
/// `NULL` for every row after one whose answer was `NULL` (bug #19695), which
/// is why [`JsonValueType`] has no JSON variant; when a release fixes it this
/// fails, and the two variants can come back. `RETURNING integer`, which the
/// type keeps, answers each row.
// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    JSON_VALUE's RETURNING leaves out the
// types 18.6 returns wrongly
#[pgorm_macros::test]
async fn json_value_returning_jsonb_sticks_at_null() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_value_canary").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        r#"CREATE TABLE t (n integer, doc jsonb);
           INSERT INTO t VALUES (1, '{"k": 1}'), (2, '{"k": null}'), (3, '{"k": 3}'),
                                (4, '{}'), (5, '{"k": 5}'), (6, NULL), (7, '{"k": 7}')"#,
    )
    .await?;

    let rows = |sql: &str| {
        let sql = sql.to_owned();
        let db = &db;
        async move {
            (sql, Values(vec![]))
                .into_tuple::<Option<String>>()
                .all(db)
                .await
        }
    };
    let every = |text: [Option<&str>; 7]| text.map(|t| t.map(str::to_owned)).to_vec();
    for ty in ["jsonb", "json"] {
        assert_eq!(
            rows(&format!(
                "SELECT JSON_VALUE(doc, '$.k' RETURNING {ty})::text FROM t ORDER BY n"
            ))
            .await?,
            every([Some("1"), None, None, None, None, None, None]),
            "RETURNING {ty}"
        );
    }

    assert_eq!(
        JsonValueType::try_from(ColumnType::JsonBinary),
        Err(ColumnType::JsonBinary)
    );
    assert_eq!(
        JsonValueType::try_from(ColumnType::Json),
        Err(ColumnType::Json)
    );
    let ints: Vec<Option<i32>> = Query::select()
        .expr(Func::json_value(doc(), "$.k").returning(JsonValueType::Integer))
        .from(Name::runtime("t"))
        .order_by(Name::runtime("n"), Order::Asc)
        .build()
        .into_tuple()
        .all(&db)
        .await?;
    assert_eq!(ints, [Some(1), None, Some(3), None, Some(5), None, Some(7)]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The aggregates order, filter and treat `NULL` as specified, answer `NULL`
/// over no rows, and run as window functions.
// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    JSON_ARRAYAGG and JSON_OBJECTAGG, live
// [spec:pgorm:def:sql.ast.window-statement+8/test]    a JSON aggregate under OVER
#[pgorm_macros::test]
async fn aggregates_order_filter_and_drop_nulls() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_aggregates").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        "CREATE TABLE t (k text, v integer); \
         INSERT INTO t VALUES ('a', 1), ('b', NULL), ('c', 3), ('a', 4)",
    )
    .await?;
    let v = || Expr::col(Name::runtime("v"));
    let k = || Expr::col(Name::runtime("k"));

    let ordered: Json = read(
        &db,
        Func::json_arrayagg(v())
            .order_by(v(), Order::Desc)
            .returning(ColumnType::JsonBinary),
    )
    .await?;
    assert_eq!(ordered, json!([4, 3, 1]), "a NULL is dropped by default");
    let nulled: Json = read(
        &db,
        Func::json_arrayagg(v())
            .order_by(k(), Order::Asc)
            .order_by(v(), Order::Asc)
            .null_on_null()
            .returning(ColumnType::JsonBinary),
    )
    .await?;
    assert_eq!(nulled, json!([1, 4, null, 3]));
    let filtered: Json = read(
        &db,
        Func::json_arrayagg(v())
            .order_by(v(), Order::Asc)
            .returning(ColumnType::JsonBinary)
            .filter(v().gt(1)),
    )
    .await?;
    assert_eq!(filtered, json!([3, 4]));
    let empty: Option<Json> = read(&db, Func::json_arrayagg(v()).filter(v().gt(100))).await?;
    assert_eq!(empty, None);

    let running: Vec<Json> = Query::select()
        .expr_window(
            Func::json_arrayagg(v()).returning(ColumnType::JsonBinary),
            WindowStatement::new()
                .order_by(Name::runtime("v"), Order::Asc)
                .take(),
        )
        .from(Name::runtime("t"))
        .and_where(v().is_not_null())
        .order_by(Name::runtime("v"), Order::Asc)
        .build()
        .into_tuple::<Json>()
        .all(&db)
        .await?;
    assert_eq!(running, [json!([1]), json!([1, 3]), json!([1, 3, 4])]);

    let last_wins: Json = read(
        &db,
        Func::json_objectagg(k(), v())
            .absent_on_null()
            .returning(ColumnType::JsonBinary),
    )
    .await?;
    assert_eq!(last_wins.as_object().map(|o| o.len()), Some(2));
    let error = read::<Json>(&db, Func::json_objectagg(k(), v()).with_unique_keys())
        .await
        .expect_err("'a' twice");
    refused_with(&error, &SqlState::from_code("22030"));

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `IS JSON` tests each kind, and unique keys at any depth; `IS NOT JSON`
/// negates it; a `NULL` operand is `NULL` either way.
// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    IS JSON over each kind, live
#[pgorm_macros::test]
async fn is_json_tests_each_kind() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_is_json").await;
    let db = ctx.db.get().await?;

    let test = |text: &'static str, kind: JsonKind| Expr::val(text).is_json(kind);
    let answers: (bool, bool, bool, bool, bool) = Query::select()
        .expr(test("{}", JsonKind::Object))
        .expr(test("[]", JsonKind::Object))
        .expr(test("1", JsonKind::Scalar))
        .expr(test("x", JsonKind::Value))
        .expr(test("[]", JsonKind::Array))
        .build()
        .into_tuple()
        .one(&db)
        .await?;
    assert_eq!(answers, (true, false, true, false, true));

    let unique: (bool, bool, bool, Option<bool>) = Query::select()
        .expr(Expr::val(r#"[{"a":1,"a":2}]"#).is_json(JsonKind::Array))
        .expr(Expr::val(r#"[{"a":1,"a":2}]"#).is_json(JsonKind::Array.with_unique_keys()))
        .expr(Expr::val("x").is_not_json(JsonKind::Value))
        .expr(Expr::val(Option::<String>::None).is_json(JsonKind::Value))
        .build()
        .into_tuple()
        .one(&db)
        .await?;
    assert_eq!(unique, (true, false, true, None));

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Run `EXPLAIN` over `statement`'s bound render with sequential scans
/// disabled, so the plan uses an index whenever one can serve the predicate.
async fn forced_plan(
    db: &DatabaseConnection,
    statement: &SelectStatement,
) -> Result<String, Error> {
    let (sql, values) = statement.build();
    db.batch_execute("SET enable_seqscan = off").await?;
    let plan = (format!("EXPLAIN (COSTS OFF) {sql}"), values)
        .into_tuple::<String>()
        .all(db)
        .await;
    db.batch_execute("RESET enable_seqscan").await?;
    Ok(plan?.join("\n"))
}

/// The decision that keeps the JSON operators: an index serves them, and not
/// the SQL/JSON function that reads the same thing. `?`, `?|` and `?&` are
/// served by a GIN `jsonb_ops` index and `JSON_EXISTS` by none; `->>` and its
/// cast by an expression index on exactly that expression, which `JSON_VALUE`
/// does not match. `JSON_VALUE` is served by an index on itself, with the
/// path bound under a custom plan.
// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    the index evidence behind keeping the
// operators
// [spec:pgorm:req:sql.ast.expr.json+3/test]    the operators an index serves and SQL/JSON
// does not replace
#[pgorm_macros::test]
async fn an_index_serves_operators_not_sql_json() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_index_use").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        "CREATE TABLE t (id integer PRIMARY KEY, doc jsonb NOT NULL); \
         INSERT INTO t SELECT i, jsonb_build_object('k', i % 100, 'tag', 't' || (i % 7)) \
         FROM generate_series(1, 2000) i; \
         CREATE INDEX t_gin ON t USING gin (doc); \
         CREATE INDEX t_k ON t ((doc ->> 'k')); \
         CREATE INDEX t_k_int ON t (((doc ->> 'k')::int)); \
         ANALYZE t",
    )
    .await?;
    let t = || Name::runtime("t");
    let filtered = |predicate: SimpleExpr| {
        Query::select()
            .column(Name::runtime("id"))
            .from(t())
            .and_where(predicate)
            .take()
    };
    let uses = |plan: &str, index: &str| plan.contains(&format!("Index Scan on {index}"));

    for predicate in [
        doc().has_json_key("tag"),
        doc().has_any_json_keys(["tag", "zz"]),
        doc().has_all_json_keys(["tag", "k"]),
    ] {
        let plan = forced_plan(&db, &filtered(predicate)).await?;
        assert!(uses(&plan, "t_gin"), "{plan}");
    }
    let plan = forced_plan(&db, &filtered(Func::json_exists(doc(), "$.tag").into())).await?;
    assert!(
        !plan.contains("Index"),
        "JSON_EXISTS is served by no index:\n{plan}"
    );

    let plan = forced_plan(
        &db,
        &filtered(Expr::expr(doc().cast_json_field("k")).eq("42")),
    )
    .await?;
    assert!(uses(&plan, "t_k"), "{plan}");
    let plan = forced_plan(
        &db,
        &filtered(Expr::expr(Func::json_value(doc(), "$.k")).eq("42")),
    )
    .await?;
    assert!(!plan.contains("Index"), "{plan}");
    let as_int = || Func::json_value(doc(), "$.k").returning(JsonValueType::Integer);
    let plan = forced_plan(&db, &filtered(Expr::expr(as_int()).eq(42))).await?;
    assert!(!plan.contains("Index"), "{plan}");

    db.batch_execute("CREATE INDEX t_jv ON t ((JSON_VALUE(doc, '$.k' RETURNING integer)))")
        .await?;
    let plan = forced_plan(&db, &filtered(Expr::expr(as_int()).eq(42))).await?;
    assert!(
        uses(&plan, "t_jv"),
        "an index on JSON_VALUE itself serves it:\n{plan}"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `document` as a `jsonb` value the operators apply to.
fn jsonb(document: Json) -> SimpleExpr {
    Expr::val(document).cast_as(Name::runtime("jsonb"))
}

/// Where the two families answer differently: `?` matches a top-level string
/// and a string array element where `JSON_EXISTS` reads a key; lax
/// `JSON_EXISTS` unwraps an array `?` does not; `->>` hands back an object's
/// text and a boolean as `true`, where `JSON_VALUE` returns `NULL` for the one
/// and `t` for the other; and a failed cast raises where `JSON_VALUE` follows
/// `ON ERROR`.
// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    the semantic differences the rule records
// [spec:pgorm:req:sql.ast.expr.json+3/test]
#[pgorm_macros::test]
async fn the_operators_and_sql_json_answer_differently() -> Result<(), Error> {
    let ctx = TestContext::new("sql_json_operator_semantics").await;
    let db = ctx.db.get().await?;

    let key = |document: Json| {
        Query::select()
            .expr(Expr::expr(jsonb(document.clone())).has_json_key("a"))
            .expr(Func::json_exists(document, "$.a"))
            .take()
    };
    for (document, answers) in [
        (json!(["a"]), (true, false)),
        (json!("a"), (true, false)),
        (json!([{"a": 1}]), (false, true)),
        (json!({"a": null}), (true, true)),
    ] {
        let read: (bool, bool) = key(document.clone()).build().into_tuple().one(&db).await?;
        assert_eq!(read, answers, "{document}");
    }

    let field = |document: Json, name: &str| {
        Query::select()
            .expr(Expr::expr(jsonb(document.clone())).cast_json_field(name))
            .expr(Func::json_value(document, format!("$.{name}")))
            .take()
    };
    let document = json!({"obj": {"x": 1}, "flag": true, "word": "x"});
    let read: (Option<String>, Option<String>) = field(document.clone(), "obj")
        .build()
        .into_tuple()
        .one(&db)
        .await?;
    assert_eq!(read, (Some(r#"{"x": 1}"#.to_owned()), None));
    let read: (Option<String>, Option<String>) = field(document.clone(), "flag")
        .build()
        .into_tuple()
        .one(&db)
        .await?;
    assert_eq!(read, (Some("true".to_owned()), Some("t".to_owned())));
    let read: (Option<String>, Option<String>) = field(document.clone(), "missing")
        .build()
        .into_tuple()
        .one(&db)
        .await?;
    assert_eq!(read, (None, None));

    let cast = Query::select()
        .expr(
            Expr::expr(Expr::expr(jsonb(document.clone())).cast_json_field("word"))
                .cast_as(Name::runtime("int4")),
        )
        .build();
    let error = cast
        .into_tuple::<i32>()
        .one(&db)
        .await
        .expect_err("'x' is no integer");
    refused_with(&error, &SqlState::INVALID_TEXT_REPRESENTATION);
    let followed: Option<i32> = Query::select()
        .expr(Func::json_value(document, "$.word").returning(JsonValueType::Integer))
        .build()
        .into_tuple()
        .one(&db)
        .await?;
    assert_eq!(followed, None);

    drop(db);
    ctx.delete().await;
    Ok(())
}
