//! The SQL/JSON constructors: `JSON_OBJECT`, `JSON_ARRAY` in both its forms,
//! the aggregates `JSON_OBJECTAGG` and `JSON_ARRAYAGG`, and `JSON(..)` and
//! `JSON_SERIALIZE(..)`.
//!
//! Each `ON NULL` and `UNIQUE KEYS` choice has one spelling, the one that
//! differs from PostgreSQL's default: an object keeps a `NULL` member unless
//! told [`absent_on_null`](JsonObject::absent_on_null), an array drops one
//! unless told [`null_on_null`](JsonArray::null_on_null), and duplicate keys
//! are allowed unless [`with_unique_keys`](JsonObject::with_unique_keys) says
//! otherwise.

use super::JsonInput;
use crate::{
    ColumnType, Condition, IntoCondition, Order, OrderExpr, SelectStatement, SimpleExpr, SqlJson,
};

/// `JSON_OBJECT(..)`: an object of key/value members. Built by
/// [`Func::json_object`](crate::Func::json_object).
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonObject {
    pub(crate) entries: Vec<(SimpleExpr, JsonInput)>,
    pub(crate) absent_on_null: bool,
    pub(crate) unique_keys: bool,
    pub(crate) returning: Option<ColumnType>,
}

impl JsonObject {
    /// Add the member `key`: `value`. The key is any `text` expression and
    /// may not be `NULL` (`22004`).
    pub fn entry<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<SimpleExpr>,
        V: Into<JsonInput>,
    {
        self.entries.push((key.into(), value.into()));
        self
    }

    /// Leave out a member whose value is `NULL`: `ABSENT ON NULL`.
    pub fn absent_on_null(mut self) -> Self {
        self.absent_on_null = true;
        self
    }

    /// Fail on a key given twice (`22030`): `WITH UNIQUE KEYS`.
    pub fn with_unique_keys(mut self) -> Self {
        self.unique_keys = true;
        self
    }

    /// Return the object as this type — `jsonb`, `text`, `bytea` — rather
    /// than as `json`.
    pub fn returning(mut self, column_type: ColumnType) -> Self {
        self.returning = Some(column_type);
        self
    }
}

impl From<JsonObject> for SimpleExpr {
    fn from(json: JsonObject) -> Self {
        SqlJson::Object(json).into()
    }
}

/// `JSON_ARRAY(..)`: an array of the given values. Built by
/// [`Func::json_array`](crate::Func::json_array).
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonArray {
    pub(crate) elements: Vec<JsonInput>,
    pub(crate) null_on_null: bool,
    pub(crate) returning: Option<ColumnType>,
}

impl JsonArray {
    /// Append an element.
    pub fn element<V>(mut self, value: V) -> Self
    where
        V: Into<JsonInput>,
    {
        self.elements.push(value.into());
        self
    }

    /// Keep a `NULL` element as JSON `null`: `NULL ON NULL`.
    pub fn null_on_null(mut self) -> Self {
        self.null_on_null = true;
        self
    }

    /// Return the array as this type rather than as `json`.
    pub fn returning(mut self, column_type: ColumnType) -> Self {
        self.returning = Some(column_type);
        self
    }
}

impl From<JsonArray> for SimpleExpr {
    fn from(json: JsonArray) -> Self {
        SqlJson::Array(json).into()
    }
}

/// `JSON_ARRAY(SELECT ..)`: an array of a one-column query's values, in the
/// query's order. Built by
/// [`Func::json_array_query`](crate::Func::json_array_query).
///
/// The query form takes no `ON NULL` (`42601`), so it has none to set: a
/// `NULL` row is always left out. A query of more than one column is refused
/// (`42601`). Over a query of no rows it is the empty array, where
/// [`JSON_ARRAYAGG`](crate::Func::json_arrayagg) is `NULL`, as this was on
/// PostgreSQL 18.
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonArrayQuery {
    pub(crate) query: Box<SelectStatement>,
    pub(crate) returning: Option<ColumnType>,
}

impl JsonArrayQuery {
    /// Return the array as this type rather than as `json`.
    pub fn returning(mut self, column_type: ColumnType) -> Self {
        self.returning = Some(column_type);
        self
    }
}

impl From<JsonArrayQuery> for SimpleExpr {
    fn from(json: JsonArrayQuery) -> Self {
        SqlJson::ArrayQuery(json).into()
    }
}

/// `JSON_OBJECTAGG(key : value ..)`: an object of one member per row. Built by
/// [`Func::json_objectagg`](crate::Func::json_objectagg).
///
/// An object's members have no order to give it, and the grammar takes no
/// `ORDER BY` here (`42601`).
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonObjectAgg {
    pub(crate) key: SimpleExpr,
    pub(crate) value: JsonInput,
    pub(crate) absent_on_null: bool,
    pub(crate) unique_keys: bool,
    pub(crate) returning: Option<ColumnType>,
    pub(crate) filter: Option<Condition>,
}

impl JsonObjectAgg {
    /// Leave out a row whose value is `NULL`: `ABSENT ON NULL`.
    pub fn absent_on_null(mut self) -> Self {
        self.absent_on_null = true;
        self
    }

    /// Fail on a key two rows give (`22030`): `WITH UNIQUE KEYS`. Without it
    /// a `json` result keeps both members and a `jsonb` one the last.
    pub fn with_unique_keys(mut self) -> Self {
        self.unique_keys = true;
        self
    }

    /// Return the object as this type rather than as `json`.
    pub fn returning(mut self, column_type: ColumnType) -> Self {
        self.returning = Some(column_type);
        self
    }

    /// Aggregate only the rows satisfying `condition`: `FILTER (WHERE ..)`,
    /// as [`FunctionCall::filter`](crate::FunctionCall::filter) writes it. A
    /// later call replaces an earlier one.
    pub fn filter<C>(mut self, condition: C) -> Self
    where
        C: IntoCondition,
    {
        self.filter = Some(condition.into_condition());
        self
    }
}

impl From<JsonObjectAgg> for SimpleExpr {
    fn from(json: JsonObjectAgg) -> Self {
        SqlJson::ObjectAgg(json).into()
    }
}

/// `JSON_ARRAYAGG(value ..)`: an array of one element per row, `NULL` over no
/// rows. Built by [`Func::json_arrayagg`](crate::Func::json_arrayagg).
///
/// The grammar takes no `DISTINCT` here (`42601`).
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonArrayAgg {
    pub(crate) value: JsonInput,
    pub(crate) order_by: Vec<OrderExpr>,
    pub(crate) null_on_null: bool,
    pub(crate) returning: Option<ColumnType>,
    pub(crate) filter: Option<Condition>,
}

impl JsonArrayAgg {
    /// Order the elements by `expr`: `ORDER BY` inside the parentheses.
    /// Repeated calls add keys, the first the most significant.
    pub fn order_by<T>(mut self, expr: T, order: Order) -> Self
    where
        T: Into<SimpleExpr>,
    {
        self.order_by.push(OrderExpr {
            expr: expr.into(),
            order,
            nulls: None,
        });
        self
    }

    /// Keep a row whose value is `NULL` as JSON `null`: `NULL ON NULL`.
    pub fn null_on_null(mut self) -> Self {
        self.null_on_null = true;
        self
    }

    /// Return the array as this type rather than as `json`.
    pub fn returning(mut self, column_type: ColumnType) -> Self {
        self.returning = Some(column_type);
        self
    }

    /// Aggregate only the rows satisfying `condition`: `FILTER (WHERE ..)`.
    /// A later call replaces an earlier one.
    pub fn filter<C>(mut self, condition: C) -> Self
    where
        C: IntoCondition,
    {
        self.filter = Some(condition.into_condition());
        self
    }
}

impl From<JsonArrayAgg> for SimpleExpr {
    fn from(json: JsonArrayAgg) -> Self {
        SqlJson::ArrayAgg(json).into()
    }
}

/// `JSON(..)`: text — or, under `FORMAT JSON`, UTF-8 bytes — parsed as a
/// `json` value, failing on text that is not JSON (`22P02`). Built by
/// [`Func::json`](crate::Func::json).
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonParse {
    pub(crate) input: JsonInput,
    pub(crate) unique_keys: bool,
}

impl JsonParse {
    /// Fail on an object repeating a key, at any depth (`22030`): `WITH
    /// UNIQUE KEYS`.
    pub fn with_unique_keys(mut self) -> Self {
        self.unique_keys = true;
        self
    }
}

impl From<JsonParse> for SimpleExpr {
    fn from(json: JsonParse) -> Self {
        SqlJson::Parse(json).into()
    }
}

/// `JSON_SERIALIZE(..)`: a JSON value as `text`, or as another string type or
/// `bytea`. Built by [`Func::json_serialize`](crate::Func::json_serialize).
///
/// The operand is written inside `JSON(..)`: PostgreSQL 19.0 serializes a
/// `json`, string or `bytea` operand faithfully but a `jsonb` one wrongly, as
/// bytes of its binary header (`\x01` for a one-member object), and `JSON(..)`
/// makes every operand the `json` value `JSON_SERIALIZE` makes of the others,
/// `jsonb` included. An operand of another type is refused by `JSON(..)` as
/// it would be by `JSON_SERIALIZE` (`42846` rather than `42804`).
// [spec:pgorm:def:sql.ast.expr.sql-json+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonSerialize {
    pub(crate) input: JsonInput,
    pub(crate) returning: Option<ColumnType>,
}

impl JsonSerialize {
    /// Return the text as this type — `varchar(n)`, `bytea` — rather than as
    /// `text`. Any other type is refused (`42804`).
    pub fn returning(mut self, column_type: ColumnType) -> Self {
        self.returning = Some(column_type);
        self
    }
}

impl From<JsonSerialize> for SimpleExpr {
    fn from(json: JsonSerialize) -> Self {
        SqlJson::Serialize(json).into()
    }
}
