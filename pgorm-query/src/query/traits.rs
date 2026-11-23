use std::fmt::{Debug, Display};

use crate::{SqlWriter, SqlWriterValues, SubQueryStatement, value::Values};

// [spec:pgorm:req:sql.ast.build+3]
pub trait QueryStatementBuilder: Debug + Display {
    /// Build the SQL statement, collecting query parameters into a vector
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let (query, params) = Query::select()
    ///     .column(Glyph::Aspect)
    ///     .from(Glyph::Table)
    ///     .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
    ///     .order_by(Glyph::Image, Order::Desc)
    ///     .order_by((Glyph::Table, Glyph::Aspect), Order::Asc)
    ///     .build();
    ///
    /// assert_eq!(
    ///     query,
    ///     r#"SELECT "aspect" FROM "glyph" WHERE COALESCE("aspect", $1) > $2 ORDER BY "image" DESC, "glyph"."aspect" ASC"#
    /// );
    /// assert_eq!(
    ///     params,
    ///     Values(vec![Value::Int(Some(0)), Value::Int(Some(2))])
    /// );
    /// ```
    ///
    /// The value-inlined rendering is the statement's [`Display`] form:
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .column(Glyph::Aspect)
    ///     .from(Glyph::Table)
    ///     .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
    ///     .order_by(Glyph::Image, Order::Desc)
    ///     .order_by((Glyph::Table, Glyph::Aspect), Order::Asc)
    ///     .to_string();
    ///
    /// assert_eq!(
    ///     query,
    ///     r#"SELECT "aspect" FROM "glyph" WHERE COALESCE("aspect", 0) > 2 ORDER BY "image" DESC, "glyph"."aspect" ASC"#
    /// );
    /// ```
    fn build(&self) -> (String, Values) {
        let mut sql = SqlWriterValues::new("$", true);
        self.build_collect_into(&mut sql);
        sql.into_parts()
    }

    /// Build the SQL statement into the given sink, returning the sink's text
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .column(Glyph::Aspect)
    ///     .from(Glyph::Table)
    ///     .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
    ///     .order_by(Glyph::Image, Order::Desc)
    ///     .order_by((Glyph::Table, Glyph::Aspect), Order::Asc)
    ///     .to_owned();
    ///
    /// let mut sql = SqlWriterValues::new("$", true);
    ///
    /// assert_eq!(
    ///     query.build_collect(&mut sql),
    ///     r#"SELECT "aspect" FROM "glyph" WHERE COALESCE("aspect", $1) > $2 ORDER BY "image" DESC, "glyph"."aspect" ASC"#
    /// );
    ///
    /// let (sql, values) = sql.into_parts();
    /// assert_eq!(
    ///     values,
    ///     Values(vec![Value::Int(Some(0)), Value::Int(Some(2))])
    /// );
    /// ```
    fn build_collect(&self, sql: &mut dyn SqlWriter) -> String {
        self.build_collect_into(sql);
        sql.to_string()
    }

    /// Build the SQL statement into the given sink
    fn build_collect_into(&self, sql: &mut dyn SqlWriter);
}

/// Conversion of a statement into the [`SubQueryStatement`] that a common
/// table expression and an expression subquery hold.
///
/// Every statement kind converts, because PostgreSQL takes each as a common
/// table expression's body: SELECT, and INSERT, UPDATE, DELETE and MERGE,
/// whose rows are those their RETURNING list yields. Only a SELECT nests as
/// an expression subquery, and the typed constructors of one take a
/// [`SelectStatement`](crate::SelectStatement).
// [spec:pgorm:req:sql.ast+5]
pub trait IntoSubQueryStatement {
    /// Wrap the statement in the [`SubQueryStatement`] variant for its kind
    fn into_sub_query_statement(self) -> SubQueryStatement;
}
