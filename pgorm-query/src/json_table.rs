//! `JSON_TABLE`: a FROM item that reads rows out of a JSON document.
//!
//! Its paths are literals, unlike the query functions': PostgreSQL refuses a
//! parameter for the root path (`0A000`) and its grammar takes only a string
//! constant for a column's or a nested path (`42601`). Each is written
//! through the value pipeline's literal escaping, never interpolated.

use super::{JsonInput, JsonPathTarget, JsonShaping};
use crate::{
    ColumnType, FromItem, IntoName, JsonExistsBehavior, JsonQueryBehavior, JsonValueBehavior, Name,
};

/// What `JSON_TABLE` produces when its root path fails: `ON ERROR`, after
/// the column list. Without a behaviour it produces no rows, as `Empty`
/// says; PostgreSQL takes no other (`42601`).
// [spec:pgorm:def:sql.ast.json-table+3]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JsonTableBehavior {
    /// Raise the error.
    Error,
    /// No rows: `EMPTY`.
    Empty,
}

/// `JSON_TABLE(context, path COLUMNS (..))`: one row per item `path` finds,
/// one column per [`JsonTableColumn`]. Built by
/// [`Func::json_table`](crate::Func::json_table), which takes the first
/// column so the list is never empty (`COLUMNS ()` is `42601`), and placed in
/// a FROM list by [`alias`](Self::alias).
// [spec:pgorm:def:sql.ast.json-table+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonTable {
    pub(crate) target: JsonPathTarget,
    pub(crate) path_name: Option<Name>,
    pub(crate) columns: Vec<JsonTableColumn>,
    pub(crate) on_error: Option<JsonTableBehavior>,
}

impl JsonTable {
    /// Append a column.
    pub fn column<C>(mut self, column: C) -> Self
    where
        C: Into<JsonTableColumn>,
    {
        self.columns.push(column.into());
        self
    }

    /// Give the path a variable, as
    /// [`JsonExists::passing`](crate::JsonExists::passing) does. The value is
    /// bound; only the paths are literals.
    pub fn passing<V, N>(mut self, value: V, name: N) -> Self
    where
        V: Into<JsonInput>,
        N: IntoName,
    {
        self.target.pass(value, name);
        self
    }

    /// Name the root path: `AS name`. Path names share one namespace with
    /// the column names, and a name given twice is refused (`42712`).
    pub fn path_name<N>(mut self, name: N) -> Self
    where
        N: IntoName,
    {
        self.path_name = Some(name.into_name());
        self
    }

    /// What to produce when the root path fails (`ON ERROR`).
    pub fn on_error(mut self, behavior: JsonTableBehavior) -> Self {
        self.on_error = Some(behavior);
        self
    }

    /// Stand in a FROM list or a join as `JSON_TABLE(..) AS "alias"`.
    ///
    /// PostgreSQL would name an unaliased table `json_table`; the alias is
    /// required here, as on every FROM item that is not a table, so a column
    /// of it is always qualified by a name the caller chose.
    pub fn alias<A>(self, alias: A) -> FromItem
    where
        A: IntoName,
    {
        FromItem::JsonTable(Box::new(self), alias.into_name())
    }
}

/// One column of a [`JsonTable`]: an ordinal, a value, a JSON fragment, an
/// existence test, or a nested path with columns of its own.
// [spec:pgorm:def:sql.ast.json-table+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonTableColumn(pub(crate) ColumnKind);

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ColumnKind {
    Ordinality(Name),
    Value(JsonValueColumn),
    Query(JsonQueryColumn),
    Exists(JsonExistsColumn),
    Nested(JsonNestedColumns),
}

impl JsonTableColumn {
    /// `name FOR ORDINALITY`: the row's number, counting from 1.
    pub fn ordinality<N>(name: N) -> Self
    where
        N: IntoName,
    {
        Self(ColumnKind::Ordinality(name.into_name()))
    }

    /// `name type`: the SQL scalar the column's path finds, read as
    /// `JSON_VALUE` reads it.
    pub fn value<N>(name: N, column_type: ColumnType) -> JsonValueColumn
    where
        N: IntoName,
    {
        JsonValueColumn {
            name: name.into_name(),
            column_type,
            path: None,
            on_empty: None,
            on_error: None,
        }
    }

    /// `name type FORMAT JSON`: the JSON the column's path finds, read as
    /// `JSON_QUERY` reads it. The type must be a string type, `json`, `jsonb`
    /// or `bytea` (`0A000` otherwise).
    pub fn query<N>(name: N, column_type: ColumnType) -> JsonQueryColumn
    where
        N: IntoName,
    {
        JsonQueryColumn {
            name: name.into_name(),
            column_type,
            path: None,
            shaping: None,
            on_empty: None,
            on_error: None,
        }
    }

    /// `name type EXISTS`: whether the column's path finds anything, as a
    /// `boolean`, an integer `1`/`0` or the text `true`/`false`.
    pub fn exists<N>(name: N, column_type: ColumnType) -> JsonExistsColumn
    where
        N: IntoName,
    {
        JsonExistsColumn {
            name: name.into_name(),
            column_type,
            path: None,
            on_error: None,
        }
    }

    /// `NESTED PATH path COLUMNS (..)`: a row per item `path` finds under the
    /// parent row's, joined to it as an outer join would be. Takes the first
    /// column, as [`Func::json_table`](crate::Func::json_table) does.
    pub fn nested<P, C>(path: P, column: C) -> JsonNestedColumns
    where
        P: Into<String>,
        C: Into<JsonTableColumn>,
    {
        JsonNestedColumns {
            path: path.into(),
            path_name: None,
            columns: vec![column.into()],
        }
    }
}

/// A scalar column of a [`JsonTable`]. Built by
/// [`JsonTableColumn::value`].
// [spec:pgorm:def:sql.ast.json-table+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonValueColumn {
    pub(crate) name: Name,
    pub(crate) column_type: ColumnType,
    pub(crate) path: Option<String>,
    pub(crate) on_empty: Option<JsonValueBehavior>,
    pub(crate) on_error: Option<JsonValueBehavior>,
}

impl JsonValueColumn {
    /// Read the column at `path`, relative to the row's item. Without one it
    /// is read at `$."name"`, the column's own name exactly as written.
    pub fn path<P>(mut self, path: P) -> Self
    where
        P: Into<String>,
    {
        self.path = Some(path.into());
        self
    }

    /// What the column holds when its path finds nothing (`ON EMPTY`).
    pub fn on_empty(mut self, behavior: JsonValueBehavior) -> Self {
        self.on_empty = Some(behavior);
        self
    }

    /// What the column holds when its path fails or its value does not
    /// convert (`ON ERROR`).
    pub fn on_error(mut self, behavior: JsonValueBehavior) -> Self {
        self.on_error = Some(behavior);
        self
    }
}

impl From<JsonValueColumn> for JsonTableColumn {
    fn from(column: JsonValueColumn) -> Self {
        Self(ColumnKind::Value(column))
    }
}

/// A JSON column of a [`JsonTable`]. Built by [`JsonTableColumn::query`].
// [spec:pgorm:def:sql.ast.json-table+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonQueryColumn {
    pub(crate) name: Name,
    pub(crate) column_type: ColumnType,
    pub(crate) path: Option<String>,
    pub(crate) shaping: Option<JsonShaping>,
    pub(crate) on_empty: Option<JsonQueryBehavior>,
    pub(crate) on_error: Option<JsonQueryBehavior>,
}

impl JsonQueryColumn {
    /// Read the column at `path`, as [`JsonValueColumn::path`] does.
    pub fn path<P>(mut self, path: P) -> Self
    where
        P: Into<String>,
    {
        self.path = Some(path.into());
        self
    }

    /// `WITH UNCONDITIONAL WRAPPER`, as
    /// [`JsonQuery::with_wrapper`](crate::JsonQuery::with_wrapper) is.
    pub fn with_wrapper(mut self) -> Self {
        self.shaping = Some(JsonShaping::Wrapped);
        self
    }

    /// `WITH CONDITIONAL WRAPPER`, as
    /// [`JsonQuery::with_conditional_wrapper`](crate::JsonQuery::with_conditional_wrapper)
    /// is.
    pub fn with_conditional_wrapper(mut self) -> Self {
        self.shaping = Some(JsonShaping::ConditionallyWrapped);
        self
    }

    /// `OMIT QUOTES`, as
    /// [`JsonQuery::omit_quotes`](crate::JsonQuery::omit_quotes) is.
    pub fn omit_quotes(mut self) -> Self {
        self.shaping = Some(JsonShaping::Unquoted);
        self
    }

    /// What the column holds when its path finds nothing (`ON EMPTY`).
    pub fn on_empty(mut self, behavior: JsonQueryBehavior) -> Self {
        self.on_empty = Some(behavior);
        self
    }

    /// What the column holds when its path fails (`ON ERROR`).
    pub fn on_error(mut self, behavior: JsonQueryBehavior) -> Self {
        self.on_error = Some(behavior);
        self
    }
}

impl From<JsonQueryColumn> for JsonTableColumn {
    fn from(column: JsonQueryColumn) -> Self {
        Self(ColumnKind::Query(column))
    }
}

/// An existence column of a [`JsonTable`]. Built by
/// [`JsonTableColumn::exists`].
// [spec:pgorm:def:sql.ast.json-table+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonExistsColumn {
    pub(crate) name: Name,
    pub(crate) column_type: ColumnType,
    pub(crate) path: Option<String>,
    pub(crate) on_error: Option<JsonExistsBehavior>,
}

impl JsonExistsColumn {
    /// Test `path`, as [`JsonValueColumn::path`] reads it.
    pub fn path<P>(mut self, path: P) -> Self
    where
        P: Into<String>,
    {
        self.path = Some(path.into());
        self
    }

    /// What the column holds when evaluating its path fails (`ON ERROR`).
    /// There is no `ON EMPTY`: finding nothing is `false`.
    pub fn on_error(mut self, behavior: JsonExistsBehavior) -> Self {
        self.on_error = Some(behavior);
        self
    }
}

impl From<JsonExistsColumn> for JsonTableColumn {
    fn from(column: JsonExistsColumn) -> Self {
        Self(ColumnKind::Exists(column))
    }
}

/// A `NESTED PATH` of a [`JsonTable`], with its own columns. Built by
/// [`JsonTableColumn::nested`].
// [spec:pgorm:def:sql.ast.json-table+3]
#[derive(Debug, Clone, PartialEq)]
pub struct JsonNestedColumns {
    pub(crate) path: String,
    pub(crate) path_name: Option<Name>,
    pub(crate) columns: Vec<JsonTableColumn>,
}

impl JsonNestedColumns {
    /// Append a column.
    pub fn column<C>(mut self, column: C) -> Self
    where
        C: Into<JsonTableColumn>,
    {
        self.columns.push(column.into());
        self
    }

    /// Name the nested path: `AS name`, in the namespace the column names
    /// share.
    pub fn path_name<N>(mut self, name: N) -> Self
    where
        N: IntoName,
    {
        self.path_name = Some(name.into_name());
        self
    }
}

impl From<JsonNestedColumns> for JsonTableColumn {
    fn from(columns: JsonNestedColumns) -> Self {
        Self(ColumnKind::Nested(columns))
    }
}
