//! The three SQL/JSON query functions: `JSON_EXISTS`, `JSON_VALUE` and
//! `JSON_QUERY`, and the behaviours each takes when its path finds nothing or
//! fails.

use super::{JsonInput, JsonPathTarget, JsonShaping, JsonValueType};
use crate::{ColumnType, IntoName, SimpleExpr, SqlJson, Value};

/// What `JSON_EXISTS` answers when evaluating its path fails: `ON ERROR`.
///
/// A path that merely finds nothing is not an error — the answer is then
/// `false` — so `JSON_EXISTS` has no `ON EMPTY`. Without a behaviour the
/// server answers `false`.
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JsonExistsBehavior {
    True,
    False,
    /// SQL `NULL`.
    Unknown,
    Error,
}

/// What `JSON_VALUE` returns when its path finds nothing (`ON EMPTY`) or
/// fails (`ON ERROR`) — including when what it finds is not one scalar, or
/// does not convert to the `RETURNING` type. Without a behaviour the server
/// returns `NULL` in both cases.
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum JsonValueBehavior {
    Null,
    Error,
    /// This value, which the server converts to the `RETURNING` type.
    ///
    /// It is written into the statement as a literal, escaped as every
    /// inlined value is, and never bound: PostgreSQL refuses a parameter here
    /// (`42804`), admitting only a constant, a function call or an operator
    /// expression.
    Default(Value),
}

/// What `JSON_QUERY` returns when its path finds nothing (`ON EMPTY`) or
/// fails (`ON ERROR`): [`JsonValueBehavior`]'s three, or an empty array or
/// object. Without a behaviour the server returns `NULL` in both cases.
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum JsonQueryBehavior {
    Null,
    Error,
    EmptyArray,
    EmptyObject,
    /// This value, written as a literal and never bound, as
    /// [`JsonValueBehavior::Default`]'s is.
    Default(Value),
}

/// `JSON_EXISTS(context, path ..)`: whether `path` finds anything in the
/// context item. Built by [`Func::json_exists`](crate::Func::json_exists).
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonExists {
    pub(crate) target: JsonPathTarget,
    pub(crate) on_error: Option<JsonExistsBehavior>,
}

impl JsonExists {
    /// Give the path a variable: `value` is what `$name` reads in it.
    ///
    /// The name is an identifier, quoted like every other, so the path names
    /// it exactly as written — `passing(1, Name::runtime("Min"))` is `$Min`.
    /// A value is bound as itself, its type written beside its placeholder,
    /// so a number compares as a number and a JSON document as JSON.
    pub fn passing<V, N>(mut self, value: V, name: N) -> Self
    where
        V: Into<JsonInput>,
        N: IntoName,
    {
        self.target.pass(value, name);
        self
    }

    /// What to answer when evaluating the path fails (`ON ERROR`). A later
    /// call replaces an earlier one.
    pub fn on_error(mut self, behavior: JsonExistsBehavior) -> Self {
        self.on_error = Some(behavior);
        self
    }
}

impl From<JsonExists> for SimpleExpr {
    fn from(json: JsonExists) -> Self {
        SqlJson::Exists(json).into()
    }
}

/// `JSON_VALUE(context, path ..)`: the one SQL scalar `path` finds. Built by
/// [`Func::json_value`](crate::Func::json_value).
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonValue {
    pub(crate) target: JsonPathTarget,
    pub(crate) returning: Option<ColumnType>,
    pub(crate) on_empty: Option<JsonValueBehavior>,
    pub(crate) on_error: Option<JsonValueBehavior>,
}

impl JsonValue {
    /// Give the path a variable, as [`JsonExists::passing`] does.
    pub fn passing<V, N>(mut self, value: V, name: N) -> Self
    where
        V: Into<JsonInput>,
        N: IntoName,
    {
        self.target.pass(value, name);
        self
    }

    /// Return the scalar as this type rather than as `text`. A scalar that
    /// does not convert is an error, so [`on_error`](Self::on_error) decides
    /// what it becomes. The type is never `json` or `jsonb`, which
    /// PostgreSQL 18 returned wrongly ([`JsonValueType`]).
    pub fn returning(mut self, ty: JsonValueType) -> Self {
        self.returning = Some(ty.into());
        self
    }

    /// What to return when the path finds nothing (`ON EMPTY`).
    pub fn on_empty(mut self, behavior: JsonValueBehavior) -> Self {
        self.on_empty = Some(behavior);
        self
    }

    /// What to return when the path fails, finds more than one scalar or
    /// something that is not a scalar, or the scalar does not convert
    /// (`ON ERROR`).
    pub fn on_error(mut self, behavior: JsonValueBehavior) -> Self {
        self.on_error = Some(behavior);
        self
    }
}

impl From<JsonValue> for SimpleExpr {
    fn from(json: JsonValue) -> Self {
        SqlJson::Value(json).into()
    }
}

/// `JSON_QUERY(context, path ..)`: the JSON `path` finds, as `jsonb` unless
/// another type is asked for. Built by
/// [`Func::json_query`](crate::Func::json_query).
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonQuery {
    pub(crate) target: JsonPathTarget,
    pub(crate) returning: Option<ColumnType>,
    pub(crate) shaping: Option<JsonShaping>,
    pub(crate) on_empty: Option<JsonQueryBehavior>,
    pub(crate) on_error: Option<JsonQueryBehavior>,
}

impl JsonQuery {
    /// Give the path a variable, as [`JsonExists::passing`] does.
    pub fn passing<V, N>(mut self, value: V, name: N) -> Self
    where
        V: Into<JsonInput>,
        N: IntoName,
    {
        self.target.pass(value, name);
        self
    }

    /// Return the result as this type — `json`, `text`, `bytea` or another —
    /// rather than as `jsonb`.
    pub fn returning(mut self, column_type: ColumnType) -> Self {
        self.returning = Some(column_type);
        self
    }

    /// Wrap the result in an array, whatever it is: `WITH UNCONDITIONAL
    /// WRAPPER`. Without a wrapper a path finding several items is an error.
    ///
    /// This, [`with_conditional_wrapper`](Self::with_conditional_wrapper) and
    /// [`omit_quotes`](Self::omit_quotes) fill one slot, and the last call
    /// wins: PostgreSQL refuses `OMIT QUOTES` beside a wrapper.
    pub fn with_wrapper(mut self) -> Self {
        self.shaping = Some(JsonShaping::Wrapped);
        self
    }

    /// Wrap the result in an array only when the path finds several items:
    /// `WITH CONDITIONAL WRAPPER`. One item, scalar or not, is returned as it
    /// is.
    pub fn with_conditional_wrapper(mut self) -> Self {
        self.shaping = Some(JsonShaping::ConditionallyWrapped);
        self
    }

    /// Return a scalar string without its quotes: `OMIT QUOTES`. What remains
    /// must still be a value of the `RETURNING` type — `x` is `text`, but no
    /// `jsonb` — or the query fails.
    pub fn omit_quotes(mut self) -> Self {
        self.shaping = Some(JsonShaping::Unquoted);
        self
    }

    /// What to return when the path finds nothing (`ON EMPTY`).
    pub fn on_empty(mut self, behavior: JsonQueryBehavior) -> Self {
        self.on_empty = Some(behavior);
        self
    }

    /// What to return when the path fails, finds several items without a
    /// wrapper, or the result does not convert (`ON ERROR`).
    pub fn on_error(mut self, behavior: JsonQueryBehavior) -> Self {
        self.on_error = Some(behavior);
        self
    }
}

impl From<JsonQuery> for SimpleExpr {
    fn from(json: JsonQuery) -> Self {
        SqlJson::Query(json).into()
    }
}
