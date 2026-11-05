//! A set-returning function read as a relation: one call or several zipped
//! side by side under `ROWS FROM (..)`, optionally numbered `WITH
//! ORDINALITY`, under an alias that may name its columns, a call returning
//! `record` carrying the column definition list that gives its columns.

use crate::{ColumnType, FromItem, FunctionCall, IntoName, Name};

/// A function in `FROM`, as PostgreSQL's `func_table` spells it: one call,
/// `f(..)`, or `ROWS FROM (f(..), g(..))`, which reads the calls' rows side
/// by side and pads the shorter ones with NULLs. [`with_ordinality`] adds a
/// final `bigint` column numbering the rows from 1, and
/// [`columns`](Self::columns) names the columns in the alias,
/// `AS "t"("a", "b")`, the ordinal column last. A call to a function
/// returning `record` — [`Func::json_to_record`](crate::Func::json_to_record)
/// and its kin, or a function declared `RETURNS record` — has no columns
/// until [`column_definition`](Self::column_definition) defines them.
///
/// [`alias`](Self::alias) finishes it as the [`FromItem`] that `from` and
/// the joins take, as [`JsonTable::alias`](crate::JsonTable::alias) does.
/// A lone call with no ordinal, no column names and no column definitions is
/// exactly [`FromItem::FunctionCall`], and `alias` returns it as one, so the
/// two spellings of `f(..) AS "t"` are one value.
///
/// # Examples
///
/// ```
/// use pgorm_query::*;
///
/// let series = Func::named(Name::runtime("generate_series")).arg(10).arg(12);
/// let query = Query::select()
///     .column(Asterisk)
///     .from(
///         TableFunction::new(series)
///             .with_ordinality()
///             .columns([Name::runtime("v"), Name::runtime("n")])
///             .alias(Name::runtime("t")),
///     )
///     .to_owned();
///
/// assert_eq!(
///     query.to_string(),
///     r#"SELECT * FROM generate_series(10, 12) WITH ORDINALITY AS "t"("v", "n")"#
/// );
/// ```
///
/// [`with_ordinality`]: Self::with_ordinality
// [spec:pgorm:req:sql.ast.select.table-function+3]
#[derive(Debug, Clone, PartialEq)]
pub struct TableFunction {
    pub(crate) functions: Vec<TableCall>,
    pub(crate) rows_from: bool,
    pub(crate) ordinality: bool,
    pub(crate) columns: Vec<Name>,
}

/// One call of a [`TableFunction`], and the column definition list giving
/// the columns of the `record` it returns, empty for a function whose result
/// type names its own.
// [spec:pgorm:req:sql.ast.select.column-definitions]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TableCall {
    pub(crate) call: FunctionCall,
    pub(crate) definitions: Vec<(Name, ColumnType)>,
}

impl From<FunctionCall> for TableCall {
    fn from(call: FunctionCall) -> Self {
        Self {
            call,
            definitions: Vec::new(),
        }
    }
}

impl TableFunction {
    /// One set-returning call, `f(..)`.
    pub fn new(function: FunctionCall) -> Self {
        Self {
            functions: vec![function.into()],
            rows_from: false,
            ordinality: false,
            columns: Vec::new(),
        }
    }

    /// `ROWS FROM (f(..))`, to which [`and`](Self::and) adds calls. Over one
    /// call it differs from [`new`](Self::new) only for a function returning
    /// a composite, whose columns `ROWS FROM` expands.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// let query = Query::select()
    ///     .column(Asterisk)
    ///     .from(
    ///         TableFunction::rows_from(
    ///             Func::named(Name::runtime("generate_series")).arg(1).arg(2),
    ///         )
    ///         .and(Func::named(Name::runtime("generate_series")).arg(1).arg(3))
    ///         .with_ordinality()
    ///         .columns([Name::runtime("x"), Name::runtime("y"), Name::runtime("n")])
    ///         .alias(Name::runtime("t")),
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT * FROM ROWS FROM (generate_series(1, 2), generate_series(1, 3)) WITH ORDINALITY AS "t"("x", "y", "n")"#
    /// );
    /// ```
    pub fn rows_from(function: FunctionCall) -> Self {
        Self {
            rows_from: true,
            ..Self::new(function)
        }
    }

    /// Add a call read beside the others. Several calls are `ROWS FROM`
    /// whichever constructor began it, since the grammar has no other
    /// spelling for them.
    pub fn and(mut self, function: FunctionCall) -> Self {
        self.functions.push(function.into());
        self.rows_from = true;
        self
    }

    /// Define a column of the `record` the call added last returns: its
    /// name and type, appended to that call's column definition list,
    /// `AS ("a" integer, "b" text)`. A function returning `record` has no
    /// columns of its own, so PostgreSQL requires the list of it (`42601`),
    /// and refuses one on any other function (`42601`); which a function
    /// returns is the server's to know.
    ///
    /// A lone call writes its list in the alias, `f(..) AS "t"("a" integer)`.
    /// PostgreSQL refuses that spelling beside `WITH ORDINALITY` (`42601`),
    /// and the alias holds either the list or column names, so a lone call
    /// with either is written `ROWS FROM (f(..) AS ("a" integer))`, the one
    /// spelling that means it, as several calls are.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// let record = Func::json_to_record(Expr::val(r#"{"a": 1, "b": "x"}"#));
    /// let query = Query::select()
    ///     .column(Asterisk)
    ///     .from(
    ///         TableFunction::new(record.clone())
    ///             .column_definition(Name::runtime("a"), ColumnType::Integer)
    ///             .column_definition(Name::runtime("b"), ColumnType::Text)
    ///             .alias(Name::runtime("t")),
    ///     )
    ///     .to_owned();
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT * FROM JSON_TO_RECORD('{"a": 1, "b": "x"}') AS "t"("a" integer, "b" text)"#
    /// );
    ///
    /// let query = Query::select()
    ///     .column(Asterisk)
    ///     .from(
    ///         TableFunction::new(record)
    ///             .column_definition(Name::runtime("a"), ColumnType::Integer)
    ///             .and(Func::generate_series(1, 2))
    ///             .with_ordinality()
    ///             .alias(Name::runtime("t")),
    ///     )
    ///     .to_owned();
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"SELECT * FROM ROWS FROM (JSON_TO_RECORD('{"a": 1, "b": "x"}') AS ("a" integer),"#,
    ///         r#"GENERATE_SERIES(1, 2)) WITH ORDINALITY AS "t""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.select.column-definitions]
    pub fn column_definition<N>(mut self, name: N, column_type: ColumnType) -> Self
    where
        N: IntoName,
    {
        if let Some(last) = self.functions.last_mut() {
            last.definitions.push((name.into_name(), column_type));
        }
        self
    }

    /// Whether the relation is written `ROWS FROM (..)`: when asked, over
    /// several calls, and over a lone call whose column definition list
    /// cannot stand in the alias because an ordinal column or column names
    /// would stand beside it.
    // [spec:pgorm:req:sql.ast.select.column-definitions]
    pub(crate) fn spelled_rows_from(&self) -> bool {
        self.rows_from
            || self.functions.len() > 1
            || ((self.ordinality || !self.columns.is_empty())
                && self
                    .functions
                    .iter()
                    .any(|call| !call.definitions.is_empty()))
    }

    /// Number the rows from 1 in a final `bigint` column: `WITH ORDINALITY`.
    pub fn with_ordinality(mut self) -> Self {
        self.ordinality = true;
        self
    }

    /// Name the relation's columns in its alias, `AS "t"("a", "b")`,
    /// replacing any names already given. The ordinal column, when there is
    /// one, comes last; naming fewer columns than the relation has leaves the
    /// rest their own names, and naming more is PostgreSQL's refusal
    /// (`42P10`).
    pub fn columns<I, N>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = N>,
        N: IntoName,
    {
        self.columns = columns.into_iter().map(IntoName::into_name).collect();
        self
    }

    /// Finish the relation under `alias`, the [`FromItem`] `from` and the
    /// joins take.
    pub fn alias<A>(mut self, alias: A) -> FromItem
    where
        A: IntoName,
    {
        let alias = alias.into_name();
        if !self.rows_from
            && !self.ordinality
            && self.columns.is_empty()
            && self
                .functions
                .iter()
                .all(|call| call.definitions.is_empty())
            && let Some(function) = self.functions.pop()
        {
            return FromItem::FunctionCall(function.call, alias);
        }
        FromItem::TableFunction(Box::new(self), alias)
    }
}
