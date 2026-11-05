//! The SQL/JSON entry points. Each returns its own builder rather than a
//! [`FunctionCall`], as [`Func::grouping`] does, because the clauses after the
//! arguments are grammar a call's argument list cannot hold.

use super::*;
use crate::{
    JsonArray, JsonArrayAgg, JsonArrayQuery, JsonExists, JsonInput, JsonObject, JsonObjectAgg,
    JsonParse, JsonQuery, JsonSerialize, JsonTable, JsonTableColumn, JsonValue, SelectStatement,
    SqlJson, json::JsonPathTarget,
};

impl Func {
    /// `JSON_EXISTS(context, path)`: whether the SQL/JSON `path` finds
    /// anything in `context`, a `jsonb`, `json` or `text` expression.
    ///
    /// The path is a value: it is bound like any other, as `text` cast to
    /// `jsonpath`, so it never becomes part of the statement's text.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(
    ///         Func::json_exists(Expr::col(Char::UserData), "$.tags[*] ? (@ == $tag)")
    ///             .passing("blue", Name::runtime("tag"))
    ///             .into(),
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.build().0,
    ///     [
    ///         r#"SELECT "id" FROM "character" WHERE JSON_EXISTS("user_data","#,
    ///         r#"CAST($1::text AS jsonpath) PASSING $2::text AS "tag")"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    pub fn json_exists<C, P>(context: C, path: P) -> JsonExists
    where
        C: Into<JsonInput>,
        P: Into<String>,
    {
        JsonExists {
            target: JsonPathTarget::new(context, path),
            on_error: None,
        }
    }

    /// `JSON_VALUE(context, path)`: the one SQL scalar `path` finds, as
    /// `text` unless [`returning`](JsonValue::returning) asks for a type.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(
    ///         Func::json_value(Expr::col(Char::UserData), "$.size")
    ///             .returning(JsonValueType::Integer)
    ///             .on_empty(JsonValueBehavior::Default(0.into()))
    ///             .on_error(JsonValueBehavior::Error),
    ///     )
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.build().0,
    ///     [
    ///         r#"SELECT JSON_VALUE("user_data", CAST($1::text AS jsonpath) RETURNING integer"#,
    ///         r#"DEFAULT 0 ON EMPTY ERROR ON ERROR) FROM "character""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    pub fn json_value<C, P>(context: C, path: P) -> JsonValue
    where
        C: Into<JsonInput>,
        P: Into<String>,
    {
        JsonValue {
            target: JsonPathTarget::new(context, path),
            returning: None,
            on_empty: None,
            on_error: None,
        }
    }

    /// `JSON_QUERY(context, path)`: the JSON `path` finds, as `jsonb` unless
    /// [`returning`](JsonQuery::returning) asks for a type.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(
    ///         Func::json_query(Expr::col(Char::UserData), "$.tags[*]")
    ///             .with_wrapper()
    ///             .on_empty(JsonQueryBehavior::EmptyArray),
    ///     )
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"SELECT JSON_QUERY("user_data", CAST('$.tags[*]' AS jsonpath)"#,
    ///         r#"WITH UNCONDITIONAL WRAPPER EMPTY ARRAY ON EMPTY) FROM "character""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    pub fn json_query<C, P>(context: C, path: P) -> JsonQuery
    where
        C: Into<JsonInput>,
        P: Into<String>,
    {
        JsonQuery {
            target: JsonPathTarget::new(context, path),
            returning: None,
            shaping: None,
            on_empty: None,
            on_error: None,
        }
    }

    /// `JSON_OBJECT(..)`, empty until [`entry`](JsonObject::entry) adds a
    /// member.
    ///
    /// A value is bound with its type written beside its placeholder, so a
    /// number becomes a JSON number and a [`Value::Json`](crate::Value::Json) nested JSON, not a
    /// string — the server would otherwise read every parameter here as text.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(
    ///         Func::json_object()
    ///             .entry("id", Expr::col(Char::Id))
    ///             .entry("size", 12)
    ///             .absent_on_null()
    ///             .returning(ColumnType::JsonBinary),
    ///     )
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.build().0,
    ///     [
    ///         r#"SELECT JSON_OBJECT($1::text : "id", $2::text : $3::int4"#,
    ///         r#"ABSENT ON NULL RETURNING jsonb) FROM "character""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    pub fn json_object() -> JsonObject {
        JsonObject {
            entries: Vec::new(),
            absent_on_null: false,
            unique_keys: false,
            returning: None,
        }
    }

    /// `JSON_ARRAY(..)`, empty until [`element`](JsonArray::element) adds a
    /// value.
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::json_array().element(1).element("a").null_on_null())
    ///         .to_string(),
    ///     r#"SELECT JSON_ARRAY(1::int4, 'a'::text NULL ON NULL)"#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    pub fn json_array() -> JsonArray {
        JsonArray {
            elements: Vec::new(),
            null_on_null: false,
            returning: None,
        }
    }

    /// `JSON_ARRAY(SELECT ..)`: an array of `query`'s one column.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::json_array_query(
    ///             Query::select().column(Char::Id).from(Char::Table).take()
    ///         ))
    ///         .to_string(),
    ///     r#"SELECT JSON_ARRAY(SELECT "id" FROM "character")"#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    pub fn json_array_query(query: SelectStatement) -> JsonArrayQuery {
        JsonArrayQuery {
            query: Box::new(query),
            returning: None,
        }
    }

    /// `JSON_OBJECTAGG(key : value)`: an object with a member per row.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::json_objectagg(Expr::col(Char::Character), Expr::col(Char::FontSize)))
    ///         .from(Char::Table)
    ///         .to_string(),
    ///     r#"SELECT JSON_OBJECTAGG("character" : "font_size") FROM "character""#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    pub fn json_objectagg<K, V>(key: K, value: V) -> JsonObjectAgg
    where
        K: Into<SimpleExpr>,
        V: Into<JsonInput>,
    {
        JsonObjectAgg {
            key: key.into(),
            value: value.into(),
            absent_on_null: false,
            unique_keys: false,
            returning: None,
            filter: None,
        }
    }

    /// `JSON_ARRAYAGG(value)`: an array with an element per row.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::json_arrayagg(Expr::col(Char::Id)).order_by(Expr::col(Char::FontSize), Order::Desc))
    ///         .from(Char::Table)
    ///         .to_string(),
    ///     r#"SELECT JSON_ARRAYAGG("id" ORDER BY "font_size" DESC) FROM "character""#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    pub fn json_arrayagg<V>(value: V) -> JsonArrayAgg
    where
        V: Into<JsonInput>,
    {
        JsonArrayAgg {
            value: value.into(),
            order_by: Vec::new(),
            null_on_null: false,
            returning: None,
            filter: None,
        }
    }

    /// `JSON(input)`: text parsed as `json`.
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::json(r#"{"a":1}"#).with_unique_keys())
    ///         .to_string(),
    ///     r#"SELECT JSON('{"a":1}'::text WITH UNIQUE KEYS)"#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    pub fn json<V>(input: V) -> JsonParse
    where
        V: Into<JsonInput>,
    {
        JsonParse {
            input: input.into(),
            unique_keys: false,
        }
    }

    /// `JSON_SCALAR(expr)`: an SQL scalar as the JSON scalar of its type — a
    /// number as a number, a `text` as a string, a `date` as its ISO string.
    ///
    /// A bound value carries its type, so `5` is the number `5`, not the
    /// string `"5"` the server would make of an untyped parameter.
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// let (sql, values) = Query::select().expr(Func::json_scalar(5)).build();
    /// assert_eq!(sql, r#"SELECT JSON_SCALAR($1::int4)"#);
    /// assert_eq!(values, Values(vec![5.into()]));
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    pub fn json_scalar<T>(expr: T) -> SimpleExpr
    where
        T: Into<SimpleExpr>,
    {
        SqlJson::Scalar(expr.into()).into()
    }

    /// `JSON_SERIALIZE(input)`: a JSON value as `text`.
    ///
    /// The input is read through `JSON(..)`, which serializes a `jsonb` value
    /// as its document where PostgreSQL 18.6's `JSON_SERIALIZE` alone does
    /// not ([`JsonSerialize`]).
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::json_serialize(Expr::col(Char::UserData)).returning(ColumnType::Bytea))
    ///         .from(Char::Table)
    ///         .to_string(),
    ///     r#"SELECT JSON_SERIALIZE(JSON("user_data") RETURNING bytea) FROM "character""#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    pub fn json_serialize<V>(input: V) -> JsonSerialize
    where
        V: Into<JsonInput>,
    {
        JsonSerialize {
            input: input.into(),
            returning: None,
        }
    }

    /// `JSON_TABLE(context, path COLUMNS (column, ..))`: a FROM item with a
    /// row per item `path` finds. It takes its first column, because a
    /// `JSON_TABLE` without one is refused (`42601`).
    ///
    /// The path is a literal here, escaped as an inlined string is: the
    /// server takes no parameter for it. A value the path needs is passed
    /// with [`passing`](JsonTable::passing), which binds it.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Asterisk)
    ///     .from(Char::Table)
    ///     .from(
    ///         Func::json_table(
    ///             Expr::col(Char::UserData),
    ///             "$.tags[*]",
    ///             JsonTableColumn::ordinality(Name::runtime("n")),
    ///         )
    ///         .column(JsonTableColumn::value(Name::runtime("tag"), ColumnType::Text).path("$"))
    ///         .alias(Name::runtime("t")),
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.build().0,
    ///     [
    ///         r#"SELECT * FROM "character", JSON_TABLE("user_data", '$.tags[*]'"#,
    ///         r#"COLUMNS ("n" FOR ORDINALITY, "tag" text PATH '$')) AS "t""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.json-table]
    pub fn json_table<C, P, K>(context: C, path: P, column: K) -> JsonTable
    where
        C: Into<JsonInput>,
        P: Into<String>,
        K: Into<JsonTableColumn>,
    {
        JsonTable {
            target: JsonPathTarget::new(context, path),
            path_name: None,
            columns: vec![column.into()],
            on_error: None,
        }
    }
}
