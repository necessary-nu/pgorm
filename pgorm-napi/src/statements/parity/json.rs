//! The SQL/JSON family's cases, built with pgorm-query directly.

use pgorm::pgorm_query::{
    ColumnRef, ColumnType, Expr, ExprTrait, FromItem, Func, JoinType, JsonExistsBehavior, JsonKind,
    JsonQueryBehavior, JsonTableBehavior, JsonTableColumn, JsonValueBehavior, JsonValueType,
    NamedTable, Order, Query, SelectStatement, SimpleExpr, StringLen, TableName, Values,
};

use super::n;

macro_rules! col {
    ($name:expr) => {
        SimpleExpr::from(Expr::col(n($name)))
    };
}

macro_rules! table {
    ($name:expr) => {
        FromItem::Table(NamedTable::from(TableName::Table(n($name))))
    };
}

macro_rules! docs {
    () => {
        FromItem::Table(NamedTable::from(TableName::Table(n("docs"))).alias(n("d")))
    };
}

macro_rules! scalar {
    ($column:expr) => {
        JsonValueType::try_from($column).expect("a scalar type")
    };
}

macro_rules! one {
    ($expr:expr) => {
        Query::select().expr($expr).build()
    };
}

fn json_table() -> (String, Values) {
    let columns = [
        JsonTableColumn::value(n("n"), ColumnType::Integer).into(),
        JsonTableColumn::value(n("label"), ColumnType::Text)
            .path("$.name")
            .on_empty(JsonValueBehavior::Default("none".into()))
            .on_error(JsonValueBehavior::Null)
            .into(),
        JsonTableColumn::query(n("tags"), ColumnType::JsonBinary)
            .with_conditional_wrapper()
            .on_empty(JsonQueryBehavior::EmptyArray)
            .into(),
        JsonTableColumn::exists(n("flagged"), ColumnType::Boolean)
            .path("$.flag")
            .on_error(JsonExistsBehavior::False)
            .into(),
        JsonTableColumn::nested(
            "$.parts[*]",
            JsonTableColumn::from(JsonTableColumn::value(n("part"), ColumnType::Text).path("$")),
        )
        .path_name(n("parts"))
        .into(),
    ];
    let items = columns
        .into_iter()
        .fold(
            Func::json_table(
                SimpleExpr::from(Expr::col((n("d"), n("doc")))),
                "$.items[*] ? (@.n >= $Min)",
                JsonTableColumn::ordinality(n("i")),
            ),
            |table, column: JsonTableColumn| table.column(column),
        )
        .passing(1i64, n("Min"))
        .path_name(n("root"))
        .on_error(JsonTableBehavior::Empty)
        .alias(n("jt"));
    Query::select()
        .expr(Expr::col((n("d"), n("id"))))
        .expr(Expr::col((n("jt"), n("n"))))
        .expr(Expr::col((n("jt"), n("part"))))
        .from(docs!())
        .from(items)
        .build()
}

fn json_table_left_join() -> (String, Values) {
    let mood = ColumnType::Enum {
        name: n("mood"),
        schema: None,
        variants: Vec::new(),
    };
    let labels = Func::json_table(
        SimpleExpr::from(Expr::col((n("d"), n("doc")))),
        "$.items[*]",
        JsonTableColumn::value(n("label"), mood),
    )
    .alias(n("l"));
    Query::select()
        .expr(Expr::col((n("d"), n("id"))))
        .expr(Expr::col(ColumnRef::TableAsterisk(n("l"))))
        .from(docs!())
        .join(JoinType::LeftJoin, labels, SimpleExpr::from(true))
        .build()
}

fn aggregates() -> (String, Values) {
    Query::select()
        .expr(
            Func::json_objectagg(col!("k"), col!("v"))
                .absent_on_null()
                .with_unique_keys()
                .returning(ColumnType::JsonBinary)
                .filter(Expr::expr(col!("v")).is_not_null()),
        )
        .expr(
            Func::json_arrayagg(col!("v"))
                .order_by(col!("k"), Order::Desc)
                .null_on_null()
                .returning(ColumnType::JsonBinary)
                .filter(Expr::expr(col!("k")).gt(0i64)),
        )
        .from(table!("t"))
        .build()
}

fn parse_scalar_serialize() -> (String, Values) {
    Query::select()
        .expr(Func::json(SimpleExpr::from("{\"a\": 1}")).with_unique_keys())
        .expr(Func::json_scalar(5i64))
        .expr(Func::json_scalar(5i16))
        .expr(Func::json_serialize(col!("doc")).returning(ColumnType::Text))
        .build()
}

fn is_json() -> (String, Values) {
    Query::select()
        .expr(Expr::expr(col!("doc")).is_json(JsonKind::Value))
        .expr(Expr::expr(col!("doc")).is_json(JsonKind::Object.with_unique_keys()))
        .expr(Expr::expr(col!("body")).is_not_json(JsonKind::Array))
        .expr(Expr::expr(SimpleExpr::from("[1]")).is_json(JsonKind::Scalar))
        .build()
}

fn array_query() -> SelectStatement {
    Query::select()
        .expr(col!("id"))
        .from(table!("t"))
        .to_owned()
}

pub(super) fn cases() -> Vec<(&'static str, (String, Values))> {
    vec![
        (
            "json-value",
            one!(
                Func::json_value(col!("doc"), "$.size")
                    .returning(scalar!(ColumnType::Integer))
                    .on_empty(JsonValueBehavior::Default(0i64.into()))
                    .on_error(JsonValueBehavior::Error)
            ),
        ),
        (
            "json-value-sized",
            one!(
                Func::json_value(col!("doc"), "$.price")
                    .returning(scalar!(ColumnType::Decimal(Some((10, 2)))))
                    .on_empty(JsonValueBehavior::Null)
            ),
        ),
        (
            "json-exists-passing",
            one!(
                Func::json_exists(col!("doc"), "$.tags[*] ? (@ == $Tag)")
                    .passing("blue", n("Tag"))
                    .on_error(JsonExistsBehavior::False)
            ),
        ),
        (
            "json-query",
            one!(
                Func::json_query(col!("doc"), "$.tags[*]")
                    .returning(ColumnType::JsonBinary)
                    .with_wrapper()
                    .on_empty(JsonQueryBehavior::EmptyArray)
                    .on_error(JsonQueryBehavior::Null)
            ),
        ),
        (
            "json-query-omit",
            one!(
                Func::json_query(Expr::expr(col!("body")).format_json(), "$.name")
                    .returning(ColumnType::String(StringLen::N(20)))
                    .omit_quotes()
            ),
        ),
        (
            "json-object",
            one!(
                Func::json_object()
                    .entry("id", col!("id"))
                    .entry("body", Expr::expr(col!("body")).format_json())
                    .absent_on_null()
                    .returning(ColumnType::JsonBinary)
            ),
        ),
        (
            "json-object-pairs",
            one!(
                Func::json_object()
                    .entry(col!("k"), 1i64)
                    .entry(col!("k"), 2i64)
                    .with_unique_keys()
            ),
        ),
        (
            "json-array",
            one!(
                Func::json_array()
                    .element(1i64)
                    .element("two")
                    .element(Expr::expr(col!("three")).format_json())
                    .null_on_null()
                    .returning(ColumnType::Json)
            ),
        ),
        (
            "json-array-query",
            one!(Func::json_array_query(array_query()).returning(ColumnType::JsonBinary)),
        ),
        ("json-aggregates", aggregates()),
        ("json-parse-scalar-serialize", parse_scalar_serialize()),
        ("is-json", is_json()),
        ("json-table", json_table()),
        ("json-table-left-join", json_table_left_join()),
    ]
}
