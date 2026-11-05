//! SQL/JSON: PostgreSQL's standard JSON query functions, constructors and the
//! `IS JSON` predicate.
//!
//! Each is an expression of its own rather than a [`FunctionCall`]: their
//! clauses — `PASSING`, `RETURNING`, the wrapper and quotes behaviour,
//! `ON EMPTY` / `ON ERROR`, `ON NULL`, `WITH UNIQUE KEYS`, `FORMAT JSON` — are
//! grammar between the parentheses that no argument list can spell, the way a
//! `CASE` or a window frame is. The JSON *operators* (`->`, `->>`, `#>`, `#>>`,
//! `?`, `?|`, `?&`) are a separate vocabulary on [`Expr`] and stay: they are
//! what a GIN or expression index serves, which these functions are not.
//!
//! [`FunctionCall`]: crate::FunctionCall
//! [`Expr`]: crate::Expr

use crate::{IntoName, Name, SimpleExpr};

#[path = "json_construct.rs"]
mod construct;
#[path = "json_query.rs"]
mod query;
#[path = "json_table.rs"]
mod table;
#[path = "json_value_type.rs"]
mod value_type;

pub use construct::{
    JsonArray, JsonArrayAgg, JsonArrayQuery, JsonObject, JsonObjectAgg, JsonParse, JsonSerialize,
};
pub use query::{
    JsonExists, JsonExistsBehavior, JsonQuery, JsonQueryBehavior, JsonValue, JsonValueBehavior,
};
pub(crate) use table::ColumnKind;
pub use table::{
    JsonExistsColumn, JsonNestedColumns, JsonQueryColumn, JsonTable, JsonTableBehavior,
    JsonTableColumn, JsonValueColumn,
};
pub use value_type::JsonValueType;

/// One SQL/JSON expression: the payload of [`SimpleExpr::SqlJson`].
///
/// Every form is built through [`Func`](crate::Func) — or, for `IS JSON`,
/// [`ExprTrait::is_json`](crate::ExprTrait::is_json) — and converts into a
/// [`SimpleExpr`]. The builders' fields are the crate's, so each variant holds
/// only what its builder could set.
// [spec:pgorm:def:sql.ast.expr.sql-json+2]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SqlJson {
    Exists(JsonExists),
    Value(JsonValue),
    Query(JsonQuery),
    Object(JsonObject),
    /// `JSON_ARRAY` over a list of values.
    Array(JsonArray),
    /// `JSON_ARRAY` over the rows of a one-column query.
    ArrayQuery(JsonArrayQuery),
    ObjectAgg(JsonObjectAgg),
    ArrayAgg(JsonArrayAgg),
    /// `JSON(..)`: text parsed as a `json` value.
    Parse(JsonParse),
    /// `JSON_SCALAR(..)`: an SQL scalar as a JSON one.
    Scalar(SimpleExpr),
    /// `JSON_SERIALIZE(..)`: a JSON value as text, or as the bytes of that text.
    Serialize(JsonSerialize),
    /// `operand IS [NOT] JSON ..`.
    Is {
        operand: SimpleExpr,
        test: JsonTest,
        negated: bool,
    },
}

impl From<SqlJson> for SimpleExpr {
    fn from(json: SqlJson) -> Self {
        SimpleExpr::SqlJson(Box::new(json))
    }
}

/// An expression in a position SQL/JSON reads as JSON, with or without
/// `FORMAT JSON`.
///
/// Every expression converts into one unformatted.
/// [`ExprTrait::format_json`](crate::ExprTrait::format_json) marks one as JSON text: a
/// `text` value then embeds as the JSON it spells rather than as a JSON
/// string, and a `bytea` value is read as UTF-8 JSON — the only encoding
/// PostgreSQL reads, so `ENCODING UTF8` is never written.
// [spec:pgorm:def:sql.ast.expr.sql-json+2]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonInput {
    pub(crate) expr: SimpleExpr,
    pub(crate) format_json: bool,
}

impl<T> From<T> for JsonInput
where
    T: Into<SimpleExpr>,
{
    fn from(expr: T) -> Self {
        Self {
            expr: expr.into(),
            format_json: false,
        }
    }
}

/// Which JSON an `IS JSON` test accepts: any value, or only a scalar, an
/// array or an object.
// [spec:pgorm:def:sql.ast.expr.sql-json+2]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonKind {
    Value,
    Scalar,
    Array,
    Object,
}

impl JsonKind {
    /// The test for this kind that also fails an object repeating a key, at
    /// any depth: `IS JSON .. WITH UNIQUE KEYS`.
    pub fn with_unique_keys(self) -> JsonTest {
        JsonTest {
            kind: self,
            unique_keys: true,
        }
    }
}

/// What an `IS JSON` predicate tests: a [`JsonKind`], and whether an object
/// repeating a key fails it. A bare kind converts into the test that lets one
/// through, PostgreSQL's default.
// [spec:pgorm:def:sql.ast.expr.sql-json+2]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonTest {
    pub(crate) kind: JsonKind,
    pub(crate) unique_keys: bool,
}

impl From<JsonKind> for JsonTest {
    fn from(kind: JsonKind) -> Self {
        Self {
            kind,
            unique_keys: false,
        }
    }
}

/// The context, path and `PASSING` list a query function reads, shared by
/// `JSON_EXISTS`, `JSON_VALUE` and `JSON_QUERY`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JsonPathTarget {
    pub(crate) context: JsonInput,
    pub(crate) path: String,
    pub(crate) passing: Vec<(JsonInput, Name)>,
}

impl JsonPathTarget {
    pub(crate) fn new<C, P>(context: C, path: P) -> Self
    where
        C: Into<JsonInput>,
        P: Into<String>,
    {
        Self {
            context: context.into(),
            path: path.into(),
            passing: Vec::new(),
        }
    }

    pub(crate) fn pass<V, N>(&mut self, value: V, name: N)
    where
        V: Into<JsonInput>,
        N: IntoName,
    {
        self.passing.push((value.into(), name.into_name()));
    }
}

/// What a `JSON_QUERY` does to its result beyond the defaults, `WITHOUT
/// WRAPPER` and `KEEP QUOTES`, which are never spelled: wrap it in an array,
/// always or only when it is several items, or strip a scalar string's quotes.
/// One slot, because PostgreSQL refuses `OMIT QUOTES` beside either wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsonShaping {
    Wrapped,
    ConditionallyWrapped,
    Unquoted,
}
