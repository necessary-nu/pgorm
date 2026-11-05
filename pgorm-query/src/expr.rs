//! Building blocks of SQL statements.
//!
//! [`Expr`] is where an expression starts: a column, a value, a template, a
//! subquery test. [`SimpleExpr`] is the expression tree node every clause
//! holds. What an expression can become — compared, combined, cast,
//! aggregated — is [`ExprTrait`](crate::ExprTrait)'s, which both implement.

use crate::{
    SqlJson, Subscript, error::Result, func::*, query::*, template::SqlTemplate, types::*, value::*,
};

/// The entry point of an expression: a column, a value, another expression,
/// a template or a subquery test, to be built on with
/// [`ExprTrait`](crate::ExprTrait)'s methods.
// [spec:pgorm:def:sql.ast.expr+8]
#[derive(Debug, Clone)]
pub struct Expr(SimpleExpr);

/// Represents a Simple Expression in SQL.
///
/// [`SimpleExpr`] is a node in the expression tree and can represent identifiers, function calls,
/// various operators and sub-queries.
// [spec:pgorm:def:sql.ast.expr+8]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SimpleExpr {
    Column(ColumnRef),
    Tuple(Vec<SimpleExpr>),
    Unary(UnOper, Box<SimpleExpr>),
    FunctionCall(FunctionCall),
    Binary(Box<SimpleExpr>, BinOper, Box<SimpleExpr>),
    SubQuery(Option<SubQueryOper>, Box<SubQueryStatement>),
    Value(Value),
    Values(Vec<Value>),
    /// Verbatim SQL: program text the author wrote, written out unchanged.
    Raw(&'static str),
    /// A validated template and the expressions it substitutes.
    Template(SqlTemplate),
    Keyword(Keyword),
    /// THE cast: `CAST(operand AS type)`, and the only node shape a cast has.
    // [spec:pgorm:req:sql.ast.cast-shape+2]
    AsEnum(Box<TypeName>, Box<SimpleExpr>),
    /// The searched `CASE`, whose arms each test a condition.
    Case(Box<CaseStatement>),
    /// The simple `CASE`, whose arms each compare one operand with a value.
    // [spec:pgorm:def:sql.ast.case+3]
    SimpleCase(Box<SimpleCaseStatement>),
    /// An array subscript or slice of the first expression: `a[i]`, `a[l:u]`.
    // [spec:pgorm:req:sql.ast.expr.subscript+2]
    Subscript(Box<SimpleExpr>, Box<Subscript>),
    /// The first expression under a named collation: `(a COLLATE "C")`. The
    /// right-hand side is a collation's *name*, not an expression.
    // [spec:pgorm:req:sql.ast.expr.collate+2]
    Collate(Box<SimpleExpr>, Box<Collation>),
    /// `GROUPING(a, …)`, which reads a row's grouping set.
    // [spec:pgorm:def:sql.ast.select.grouping]
    Grouping(Grouping),
    Constant(Value),
    /// The right operand of a `LIKE` / `ILIKE`: a pattern and the optional
    /// `ESCAPE` character, which the grammar admits only as a pattern's tail.
    LikePattern(LikeExpr),
    /// An SQL/JSON function, constructor or `IS JSON` predicate.
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    SqlJson(Box<SqlJson>),
}

impl Expr {
    fn new_with_left<T>(left: T) -> Self
    where
        T: Into<SimpleExpr>,
    {
        Self(left.into())
    }

    /// Express the target column without table prefix.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::SizeW).eq(1))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "size_w" = 1"#
    /// );
    /// ```
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).eq(1))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" = 1"#
    /// );
    /// ```
    pub fn col<T>(n: T) -> Self
    where
        T: IntoColumnRef,
    {
        Self::new_with_left(n.into_column_ref())
    }

    /// Wraps tuple of `SimpleExpr`, can be used for tuple comparison
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(
    ///         Expr::tuple([Expr::col(Char::SizeW).into(), Expr::value(100)])
    ///             .lt(Expr::tuple([Expr::value(500), Expr::value(100)])))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE ("size_w", 100) < (500, 100)"#
    /// );
    /// ```
    pub fn tuple<I>(n: I) -> Self
    where
        I: IntoIterator<Item = SimpleExpr>,
    {
        Expr::expr(SimpleExpr::Tuple(
            n.into_iter().collect::<Vec<SimpleExpr>>(),
        ))
    }

    /// Express a [`Value`], returning a [`Expr`].
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::val(1).into())
    ///     .and_where(Expr::val(2.5).into())
    ///     .and_where(Expr::val("3").into())
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 1 AND 2.5 AND '3'"#
    /// );
    /// ```
    pub fn val<V>(v: V) -> Self
    where
        V: Into<Value>,
    {
        Self::new_with_left(v)
    }

    /// Wrap a [`SimpleExpr`] and perform some operation on it.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::expr(Expr::col(Char::SizeW).if_null(0)).gt(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE COALESCE("size_w", 0) > 2"#
    /// );
    /// ```
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::Character)
    ///     .from(Char::Table)
    ///     .and_where(Expr::expr(Func::lower(Expr::col(Char::Character))).is_in(["a", "b"]))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character" FROM "character" WHERE LOWER("character") IN ('a', 'b')"#
    /// );
    /// ```
    #[allow(clippy::self_named_constructors)]
    pub fn expr<T>(expr: T) -> Self
    where
        T: Into<SimpleExpr>,
    {
        Self::new_with_left(expr)
    }

    /// Express a [`Value`], returning a [`SimpleExpr`].
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::value(1))
    ///     .and_where(Expr::value(2.5))
    ///     .and_where(Expr::value("3"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 1 AND 2.5 AND '3'"#
    /// );
    /// ```
    pub fn value<V>(v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        v.into()
    }

    /// Write SQL verbatim.
    ///
    /// The `&'static str` bound is the contract: the text is program text the
    /// author already controls, never data, so rendering it as SQL adds no
    /// reach that writing the SQL by hand would not have. Text that arrives at
    /// runtime cannot get here — a fragment carrying substitutions takes
    /// [`Expr::template`], which validates its placeholder census.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::raw("1 = 1"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 1 = 1"#
    /// );
    /// ```
    #[doc(alias = "cust")]
    pub fn raw(s: &'static str) -> SimpleExpr {
        SimpleExpr::Raw(s)
    }

    /// A validated SQL template and the [`Value`]s it substitutes. Use this
    /// if your fragment needs variables.
    ///
    /// The template and its values are paired by [`SqlTemplate::new`], which
    /// refuses a template whose `$N` census is not exactly `1..=v.len()`, so
    /// an arity mistake is a value you handle here rather than a panic later
    /// when the query is rendered.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    /// # fn main() -> pgorm_query::error::Result<()> {
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::Id).eq(1))
    ///     .and_where(Expr::template("6 = $1 * $2", [2, 3])?)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "id" = 1 AND (6 = 2 * 3)"#
    /// );
    /// # Ok(())
    /// # }
    /// ```
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    /// # fn main() -> pgorm_query::error::Result<()> {
    /// let query = Query::select()
    ///     .expr(Expr::template("6 = $1 * $2", [2, 3])?)
    ///     .to_owned();
    ///
    /// assert_eq!(query.to_string(), r#"SELECT 6 = 2 * 3"#);
    /// # Ok(())
    /// # }
    /// ```
    /// Postgres only: use `$$` to escape `$`
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    /// # fn main() -> pgorm_query::error::Result<()> {
    /// let query = Query::select()
    ///     .expr(Expr::template("$1 $$ $2", ["a", "b"])?)
    ///     .to_owned();
    ///
    /// assert_eq!(query.to_string(), r#"SELECT 'a' $ 'b'"#);
    /// # Ok(())
    /// # }
    /// ```
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    /// # fn main() -> pgorm_query::error::Result<()> {
    /// let query = Query::select()
    ///     .expr(Expr::template("data @? ($1::JSONPATH)", ["hello"])?)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT data @? ('hello'::JSONPATH)"#
    /// );
    /// # Ok(())
    /// # }
    /// ```
    /// A template that names more values than it was given is refused where it
    /// is written:
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert!(Expr::template("6 = $1 * $2", [2]).is_err());
    /// ```
    // [spec:pgorm:req:sql.render.custom-expr+3]
    #[doc(alias = "cust_with_values")]
    pub fn template<T, V, I>(s: T, v: I) -> Result<SimpleExpr>
    where
        T: Into<String>,
        V: Into<Value>,
        I: IntoIterator<Item = V>,
    {
        SqlTemplate::new(s, v.into_iter().map(|v| Into::<Value>::into(v).into()))
            .map(SimpleExpr::Template)
    }

    /// A validated SQL template substituting one [`SimpleExpr`]. Use this if
    /// your fragment needs another expression.
    ///
    /// The single expression is `$1`; see [`Expr::template`] for how
    /// the template's census is checked.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    /// # fn main() -> pgorm_query::error::Result<()> {
    /// let query = Query::select()
    ///     .expr(Expr::val(1).add(2))
    ///     .expr(Expr::template_with_expr("data @? ($1::JSONPATH)", "hello")?)
    ///     .to_owned();
    /// let (sql, values) = query.build();
    ///
    /// assert_eq!(sql, r#"SELECT $1 + $2, data @? ($3::JSONPATH)"#);
    /// assert_eq!(
    ///     values,
    ///     Values(vec![1i32.into(), 2i32.into(), "hello".into()])
    /// );
    /// # Ok(())
    /// # }
    /// ```
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    /// # fn main() -> pgorm_query::error::Result<()> {
    /// let query = Query::select()
    ///     .expr(Expr::template_with_expr(
    ///         "json_agg(DISTINCT $1)",
    ///         Expr::col(Char::Character),
    ///     )?)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT json_agg(DISTINCT "character")"#
    /// );
    /// # Ok(())
    /// # }
    /// ```
    // [spec:pgorm:req:sql.render.custom-expr+3]
    #[doc(alias = "cust_with_expr")]
    pub fn template_with_expr<T, E>(s: T, expr: E) -> Result<SimpleExpr>
    where
        T: Into<String>,
        E: Into<SimpleExpr>,
    {
        SqlTemplate::new(s, [expr.into()]).map(SimpleExpr::Template)
    }

    /// A validated SQL template substituting several [`SimpleExpr`]s. Use this
    /// if your fragment needs other expressions.
    ///
    /// See [`Expr::template`] for how the template's census is checked.
    // [spec:pgorm:req:sql.render.custom-expr+3]
    #[doc(alias = "cust_with_exprs")]
    pub fn template_with_exprs<T, I>(s: T, v: I) -> Result<SimpleExpr>
    where
        T: Into<String>,
        I: IntoIterator<Item = SimpleExpr>,
    {
        SqlTemplate::new(s, v).map(SimpleExpr::Template)
    }

    /// Express a `EXISTS` sub-query expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .expr_as(Expr::exists(Query::select().column(Char::Id).from(Char::Table).take()), Name::runtime("character_exists"))
    ///     .expr_as(Expr::exists(Query::select().column(Glyph::Id).from(Glyph::Table).take()), Name::runtime("glyph_exists"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT EXISTS(SELECT "id" FROM "character") AS "character_exists", EXISTS(SELECT "id" FROM "glyph") AS "glyph_exists""#
    /// );
    /// ```
    pub fn exists(sel: SelectStatement) -> SimpleExpr {
        SimpleExpr::SubQuery(
            Some(SubQueryOper::Exists),
            Box::new(sel.into_sub_query_statement()),
        )
    }

    /// Express a `ANY` sub-query expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::Id).eq(Expr::any(
    ///         Query::select().column(Char::Id).from(Char::Table).take(),
    ///     )))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "id" = ANY(SELECT "id" FROM "character")"#
    /// );
    /// ```
    pub fn any(sel: SelectStatement) -> SimpleExpr {
        SimpleExpr::SubQuery(
            Some(SubQueryOper::Any),
            Box::new(sel.into_sub_query_statement()),
        )
    }

    /// Express a `SOME` sub-query expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::Id).ne(Expr::some(
    ///         Query::select().column(Char::Id).from(Char::Table).take(),
    ///     )))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "id" <> SOME(SELECT "id" FROM "character")"#
    /// );
    /// ```
    pub fn some(sel: SelectStatement) -> SimpleExpr {
        SimpleExpr::SubQuery(
            Some(SubQueryOper::Some),
            Box::new(sel.into_sub_query_statement()),
        )
    }

    /// Express an `ARRAY` sub-query expression: the subquery's one column,
    /// collected into an array.
    ///
    /// PostgreSQL refuses a subquery of more than one column here (`42601`),
    /// as it does in `IN`, so an array of it can stand where membership is
    /// tested — `= ANY(ARRAY(..))` — and be cast as one value, which a row
    /// set cannot.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::Id).eq(Func::any(Expr::array(
    ///         Query::select().column(Char::Id).from(Char::Table).take(),
    ///     ))))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "id" = ANY(ARRAY(SELECT "id" FROM "character"))"#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr+8]
    pub fn array(sel: SelectStatement) -> SimpleExpr {
        SimpleExpr::SubQuery(
            Some(SubQueryOper::Array),
            Box::new(sel.into_sub_query_statement()),
        )
    }

    /// Express a `ALL` sub-query expression.
    pub fn all(sel: SelectStatement) -> SimpleExpr {
        SimpleExpr::SubQuery(
            Some(SubQueryOper::All),
            Box::new(sel.into_sub_query_statement()),
        )
    }

    /// Keyword `CURRENT_DATE`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// let query = Query::select().expr(Expr::current_date()).to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT CURRENT_DATE"#
    /// );
    /// ```
    pub fn current_date() -> Expr {
        Expr::new_with_left(Keyword::CurrentDate)
    }

    /// Keyword `CURRENT_TIME`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// let query = Query::select().expr(Expr::current_time()).to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT CURRENT_TIME"#
    /// );
    /// ```
    pub fn current_time() -> Expr {
        Expr::new_with_left(Keyword::CurrentTime)
    }

    /// Keyword `CURRENT_TIMESTAMP`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// let query = Query::select().expr(Expr::current_timestamp()).to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT CURRENT_TIMESTAMP"#
    /// );
    /// ```
    pub fn current_timestamp() -> Expr {
        Expr::new_with_left(Keyword::CurrentTimestamp)
    }
}

impl From<Expr> for SimpleExpr {
    fn from(src: Expr) -> Self {
        src.0
    }
}

impl<T> From<T> for SimpleExpr
where
    T: Into<Value>,
{
    fn from(v: T) -> Self {
        SimpleExpr::Value(v.into())
    }
}

impl From<FunctionCall> for SimpleExpr {
    fn from(func: FunctionCall) -> Self {
        SimpleExpr::FunctionCall(func)
    }
}

impl From<ColumnRef> for SimpleExpr {
    fn from(col: ColumnRef) -> Self {
        SimpleExpr::Column(col)
    }
}

impl From<Keyword> for SimpleExpr {
    fn from(k: Keyword) -> Self {
        SimpleExpr::Keyword(k)
    }
}

impl SimpleExpr {
    pub(crate) fn get_bin_oper(&self) -> Option<&BinOper> {
        match self {
            Self::Binary(_, oper, _) => Some(oper),
            _ => None,
        }
    }
}
