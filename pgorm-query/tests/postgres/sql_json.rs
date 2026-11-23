//! SQL/JSON: the query functions, the constructors and `IS JSON`, rendered and
//! held to PostgreSQL 19's grammar, with the parse tree read back where the
//! text alone could hide a clause in the wrong slot.

use super::*;
use crate::oracle::{assert_eq, parsed_nodes};

/// The one node of `kind` in `sql`, as the parser read it.
fn only(sql: &str, kind: &str) -> serde_json::Value {
    let mut found = parsed_nodes(sql, kind);
    assert_eq!(found.len(), 1, "exactly one {kind} in {sql}");
    found.remove(0)
}

fn data() -> Expr {
    Expr::col(Char::UserData)
}

// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    the three query functions and their clauses
// [spec:pgorm:req:sql.render.sql-json+1/test]    the path bound as text cast to jsonpath, the
// clauses in the grammar's order
#[test]
fn query_functions_render_their_clauses_in_order() {
    let query = Query::select()
        .expr(
            Func::json_value(data(), "$.size")
                .passing(2, Name::runtime("Min"))
                .returning(JsonValueType::Integer)
                .on_empty(JsonValueBehavior::Default(0.into()))
                .on_error(JsonValueBehavior::Error),
        )
        .expr(
            Func::json_query(data(), "$.tags")
                .returning(ColumnType::Text)
                .omit_quotes()
                .on_empty(JsonQueryBehavior::EmptyObject)
                .on_error(JsonQueryBehavior::Null),
        )
        .expr(Func::json_exists(data(), "strict $.a").on_error(JsonExistsBehavior::Unknown))
        .from(Char::Table)
        .to_owned();

    assert_eq!(
        query.to_string(),
        [
            r#"SELECT JSON_VALUE("user_data", CAST('$.size' AS jsonpath) PASSING 2::int4 AS "Min""#,
            r#"RETURNING integer DEFAULT 0 ON EMPTY ERROR ON ERROR),"#,
            r#"JSON_QUERY("user_data", CAST('$.tags' AS jsonpath) RETURNING text OMIT QUOTES"#,
            r#"EMPTY OBJECT ON EMPTY NULL ON ERROR),"#,
            r#"JSON_EXISTS("user_data", CAST('strict $.a' AS jsonpath) UNKNOWN ON ERROR)"#,
            r#"FROM "character""#,
        ]
        .join(" ")
    );
    let (sql, values) = query.build();
    assert_eq!(
        sql,
        [
            r#"SELECT JSON_VALUE("user_data", CAST($1::text AS jsonpath) PASSING $2::int4 AS "Min""#,
            r#"RETURNING integer DEFAULT 0 ON EMPTY ERROR ON ERROR),"#,
            r#"JSON_QUERY("user_data", CAST($3::text AS jsonpath) RETURNING text OMIT QUOTES"#,
            r#"EMPTY OBJECT ON EMPTY NULL ON ERROR),"#,
            r#"JSON_EXISTS("user_data", CAST($4::text AS jsonpath) UNKNOWN ON ERROR)"#,
            r#"FROM "character""#,
        ]
        .join(" ")
    );
    assert_eq!(
        values,
        Values(vec![
            "$.size".into(),
            2i32.into(),
            "$.tags".into(),
            "strict $.a".into()
        ])
    );

    // The parser puts the DEFAULT under `on_empty` and the bare ERROR under
    // `on_error`: the two slots are not swapped.
    let funcs = parsed_nodes(&sql, "JsonFuncExpr");
    assert_eq!(funcs.len(), 3);
    let value = &funcs[0];
    assert!(value["on_empty"].get("expr").is_some(), "{value}");
    assert!(value["on_error"].get("expr").is_none(), "{value}");
    assert_eq!(
        value["passing"][0]["JsonArgument"]["name"], "Min",
        "the PASSING name keeps its case"
    );
}

// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    the wrapper and OMIT QUOTES share one slot, the
// last call winning, so OMIT QUOTES never stands beside a wrapper
#[test]
fn wrapper_and_quotes_fill_one_slot() {
    let render = |query: JsonQuery| Query::select().expr(query).to_string();
    assert_eq!(
        render(Func::json_query(data(), "$.a").omit_quotes().with_wrapper()),
        r#"SELECT JSON_QUERY("user_data", CAST('$.a' AS jsonpath) WITH UNCONDITIONAL WRAPPER)"#
    );
    assert_eq!(
        render(
            Func::json_query(data(), "$.a")
                .with_wrapper()
                .with_conditional_wrapper()
        ),
        r#"SELECT JSON_QUERY("user_data", CAST('$.a' AS jsonpath) WITH CONDITIONAL WRAPPER)"#
    );
    assert_eq!(
        render(Func::json_query(data(), "$.a").with_wrapper().omit_quotes()),
        r#"SELECT JSON_QUERY("user_data", CAST('$.a' AS jsonpath) OMIT QUOTES)"#
    );
    assert_eq!(
        render(Func::json_query(data(), "$.a")),
        r#"SELECT JSON_QUERY("user_data", CAST('$.a' AS jsonpath))"#
    );
}

// A later behaviour replaces an earlier one in its slot, and ON EMPTY renders
// before ON ERROR whichever was set first.
// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]
#[test]
fn behaviours_replace_and_keep_the_grammars_order() {
    assert_eq!(
        Query::select()
            .expr(
                Func::json_value(data(), "$.a")
                    .on_error(JsonValueBehavior::Null)
                    .on_empty(JsonValueBehavior::Null)
                    .on_error(JsonValueBehavior::Default("none".into()))
            )
            .to_string(),
        r#"SELECT JSON_VALUE("user_data", CAST('$.a' AS jsonpath) NULL ON EMPTY DEFAULT 'none' ON ERROR)"#
    );
}

// The DEFAULT value is a literal under both render paths — the server refuses a
// parameter there — escaped as every inlined value is.
// [spec:pgorm:req:sql.render.sql-json+1/test]    DEFAULT inline and escaped under build()
#[test]
fn a_default_is_an_escaped_literal_under_build() {
    let hostile = "x'); DROP TABLE t; --\\";
    let (sql, values) = Query::select()
        .expr(Func::json_value(data(), "$.a").on_empty(JsonValueBehavior::Default(hostile.into())))
        .from(Char::Table)
        .build();
    assert_eq!(
        sql,
        r#"SELECT JSON_VALUE("user_data", CAST($1::text AS jsonpath) DEFAULT E'x\'); DROP TABLE t; --\\' ON EMPTY) FROM "character""#
    );
    assert_eq!(values, Values(vec!["$.a".into()]));
    let func = only(&sql, "JsonFuncExpr");
    assert_eq!(
        func["on_empty"]["expr"]["AConst"]["val"]["Sval"]["sval"], hostile,
        "the payload is one string constant"
    );
}

// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    the constructors, with each ON NULL and UNIQUE
// spelling the one that differs from the default
// [spec:pgorm:req:sql.render.sql-json+1/test]    `key : value` members, values carrying their
// type in both render paths
#[test]
fn constructors_render_typed_operands_in_both_paths() {
    let object = Func::json_object()
        .entry("id", Expr::col(Char::Id))
        .entry("n", 5)
        .entry("doc", serde_json::json!({"a": 1}))
        .absent_on_null()
        .with_unique_keys()
        .returning(ColumnType::JsonBinary);
    let array = Func::json_array()
        .element(true)
        .element(Expr::col(Char::Character).format_json())
        .null_on_null();

    let query = Query::select()
        .expr(object)
        .expr(array)
        .from(Char::Table)
        .to_owned();
    assert_eq!(
        query.to_string(),
        [
            r#"SELECT JSON_OBJECT('id'::text : "id", 'n'::text : 5::int4,"#,
            r#"'doc'::text : '{"a":1}'::jsonb ABSENT ON NULL WITH UNIQUE KEYS RETURNING jsonb),"#,
            r#"JSON_ARRAY(TRUE::bool, "character" FORMAT JSON NULL ON NULL) FROM "character""#,
        ]
        .join(" ")
    );
    assert_eq!(
        query.build().0,
        [
            r#"SELECT JSON_OBJECT($1::text : "id", $2::text : $3::int4,"#,
            r#"$4::text : $5::jsonb ABSENT ON NULL WITH UNIQUE KEYS RETURNING jsonb),"#,
            r#"JSON_ARRAY($6::bool, "character" FORMAT JSON NULL ON NULL) FROM "character""#,
        ]
        .join(" ")
    );

    assert_eq!(
        Query::select()
            .expr(Func::json_object())
            .expr(Func::json_object().returning(ColumnType::JsonBinary))
            .expr(Func::json_array())
            .to_string(),
        r#"SELECT JSON_OBJECT(), JSON_OBJECT(RETURNING jsonb), JSON_ARRAY()"#
    );
}

// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    the query form of JSON_ARRAY, whose query
// numbers its parameters in the enclosing statement's sequence
#[test]
fn an_array_query_renders_its_select_bare() {
    let (sql, values) = Query::select()
        .expr(
            Func::json_array_query(
                Query::select()
                    .column(Char::Id)
                    .from(Char::Table)
                    .and_where(Expr::col(Char::SizeW).gt(3))
                    .order_by(Char::Id, Order::Desc)
                    .take(),
            )
            .returning(ColumnType::JsonBinary),
        )
        .and_where(Expr::val(1).eq(1))
        .build();
    assert_eq!(
        sql,
        [
            r#"SELECT JSON_ARRAY(SELECT "id" FROM "character" WHERE "size_w" > $1"#,
            r#"ORDER BY "id" DESC RETURNING jsonb) WHERE $2 = $3"#,
        ]
        .join(" ")
    );
    assert_eq!(values, Values(vec![3i32.into(), 1i32.into(), 1i32.into()]));
}

// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    the two aggregates, their ORDER BY and FILTER
// [spec:pgorm:req:sql.render.sql-json+1/test]    FORMAT JSON before ORDER BY, FILTER after the
// parentheses
#[test]
fn aggregates_render_order_and_filter() {
    let query = Query::select()
        .expr(
            Func::json_arrayagg(Expr::col(Char::Character).format_json())
                .order_by(Expr::col(Char::FontSize), Order::Desc)
                .order_by(Expr::col(Char::Id), Order::Asc)
                .null_on_null()
                .returning(ColumnType::JsonBinary)
                .filter(Expr::col(Char::SizeW).gt(1)),
        )
        .expr(
            Func::json_objectagg(Expr::col(Char::Character), Expr::col(Char::FontSize))
                .absent_on_null()
                .with_unique_keys()
                .filter(Expr::col(Char::SizeW).gt(2)),
        )
        .from(Char::Table)
        .to_owned();
    assert_eq!(
        query.build().0,
        [
            r#"SELECT JSON_ARRAYAGG("character" FORMAT JSON ORDER BY "font_size" DESC, "id" ASC"#,
            r#"NULL ON NULL RETURNING jsonb) FILTER (WHERE "size_w" > $1),"#,
            r#"JSON_OBJECTAGG("character" : "font_size" ABSENT ON NULL WITH UNIQUE KEYS)"#,
            r#"FILTER (WHERE "size_w" > $2) FROM "character""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    JSON(), JSON_SCALAR() and JSON_SERIALIZE()
#[test]
fn parse_scalar_and_serialize_render_their_operand() {
    let (sql, values) = Query::select()
        .expr(Func::json(Expr::col(Char::Character).format_json()).with_unique_keys())
        .expr(Func::json_scalar(5))
        .expr(Func::json_scalar("5"))
        .expr(Func::json_serialize(Expr::col(Char::UserData)).returning(ColumnType::Bytea))
        .from(Char::Table)
        .build();
    assert_eq!(
        sql,
        [
            r#"SELECT JSON("character" FORMAT JSON WITH UNIQUE KEYS), JSON_SCALAR($1::int4),"#,
            r#"JSON_SCALAR($2::text), JSON_SERIALIZE(JSON("user_data") RETURNING bytea) FROM "character""#,
        ]
        .join(" ")
    );
    assert_eq!(values, Values(vec![5i32.into(), "5".into()]));
}

// [spec:pgorm:def:sql.ast.expr.sql-json+2/test]    IS JSON and IS NOT JSON over each kind
// [spec:pgorm:req:sql.render.sql-json+1/test]    self-parenthesised, and around an operator
// operand
#[test]
fn is_json_wraps_itself_and_an_operator_operand() {
    let sql = Query::select()
        .column(Char::Id)
        .from(Char::Table)
        .and_where(Expr::col(Char::Character).is_json(JsonKind::Value))
        .and_where(Expr::col(Char::Character).is_not_json(JsonKind::Scalar))
        .and_where(Expr::col(Char::Character).is_json(JsonKind::Array))
        .and_where(
            Expr::expr(Expr::col(Char::Character).concat("x"))
                .is_json(JsonKind::Object.with_unique_keys()),
        )
        .to_string();
    assert_eq!(
        sql,
        [
            r#"SELECT "id" FROM "character" WHERE ("character" IS JSON)"#,
            r#"AND ("character" IS NOT JSON SCALAR) AND ("character" IS JSON ARRAY)"#,
            r#"AND (("character" || 'x') IS JSON OBJECT WITH UNIQUE KEYS)"#,
        ]
        .join(" ")
    );
    assert_eq!(parsed_nodes(&sql, "JsonIsPredicate").len(), 4);
}

// OVER follows a JSON aggregate as it follows a call, after its FILTER.
// [spec:pgorm:def:sql.ast.window-statement+8/test]    the JSON aggregates are window functions
#[test]
fn a_json_aggregate_takes_a_window() {
    assert_eq!(
        Query::select()
            .expr_window_as(
                Func::json_arrayagg(Expr::col(Char::Id)).filter(Expr::col(Char::SizeW).gt(1)),
                WindowStatement::partition_by(Char::FontSize),
                Name::runtime("ids"),
            )
            .expr_window_name(
                Func::json_objectagg(Expr::col(Char::Character), Expr::col(Char::Id)),
                Name::runtime("w"),
            )
            .window(Name::runtime("w"), WindowStatement::partition_by(Char::FontSize))
            .from(Char::Table)
            .to_string(),
        [
            r#"SELECT JSON_ARRAYAGG("id") FILTER (WHERE "size_w" > 1) OVER ( PARTITION BY "font_size" ) AS "ids","#,
            r#"JSON_OBJECTAGG("character" : "id") OVER "w" FROM "character""#,
            r#"WINDOW "w" AS ( PARTITION BY "font_size" )"#,
        ]
        .join(" ")
    );
}
