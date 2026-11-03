//! Building blocks of SQL statements.
//!
//! [`Expr`] representing the primitive building block in the expressions.
//!
//! [`SimpleExpr`] is the expression common among select fields, where clauses and many other places.

use crate::{SqlJson, error::Result, func::*, query::*, template::SqlTemplate, types::*, value::*};

#[path = "expr_collate.rs"]
mod collate;
#[path = "expr_json.rs"]
mod json;
#[path = "expr_keyword_operators.rs"]
mod keyword_operators;
#[path = "expr_membership.rs"]
mod membership;
#[path = "expr_overlap.rs"]
mod overlap;
#[path = "expr_range_cast.rs"]
mod range_cast;
#[path = "expr_subscript.rs"]
mod subscript;

pub use subscript::Subscript;

/// Helper to build a [`SimpleExpr`].
// [spec:pgorm:def:sql.ast.expr+7]
#[derive(Debug, Clone)]
pub struct Expr {
    pub(crate) left: SimpleExpr,
    pub(crate) right: Option<SimpleExpr>,
    pub(crate) uopr: Option<UnOper>,
    pub(crate) bopr: Option<BinOper>,
}

/// Represents a Simple Expression in SQL.
///
/// [`SimpleExpr`] is a node in the expression tree and can represent identifiers, function calls,
/// various operators and sub-queries.
// [spec:pgorm:def:sql.ast.expr+7]
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
    // [spec:pgorm:req:sql.ast.expr.subscript]
    Subscript(Box<SimpleExpr>, Box<Subscript>),
    /// The first expression under a named collation: `(a COLLATE "C")`. The
    /// right-hand side is a collation's *name*, not an expression.
    // [spec:pgorm:req:sql.ast.expr.collate]
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

pub(crate) mod private {
    use crate::{BinOper, LikeExpr, SimpleExpr, UnOper};

    pub trait Expression: Sized {
        fn un_op(self, o: UnOper) -> SimpleExpr;

        fn bin_op<O, T>(self, op: O, right: T) -> SimpleExpr
        where
            O: Into<BinOper>,
            T: Into<SimpleExpr>;

        fn like_like<O>(self, op: O, like: LikeExpr) -> SimpleExpr
        where
            O: Into<BinOper>,
        {
            self.bin_op(op, SimpleExpr::LikePattern(like))
        }
    }
}

use private::Expression;

impl Expression for Expr {
    fn un_op(mut self, o: UnOper) -> SimpleExpr {
        self.uopr = Some(o);
        self.into()
    }

    fn bin_op<O, T>(mut self, op: O, right: T) -> SimpleExpr
    where
        O: Into<BinOper>,
        T: Into<SimpleExpr>,
    {
        self.bopr = Some(op.into());
        self.right = Some(right.into());
        self.into()
    }
}

impl Expr {
    fn new_with_left<T>(left: T) -> Self
    where
        T: Into<SimpleExpr>,
    {
        let left = left.into();
        Self {
            left,
            right: None,
            uopr: None,
            bopr: None,
        }
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

    /// Express an equal (`=`) expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::val("What!").eq("Nothing"))
    ///     .and_where(Expr::col(Char::Id).eq(1))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 'What!' = 'Nothing' AND "id" = 1"#
    /// );
    /// ```
    pub fn eq<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::Equal, v)
    }

    /// Express a not equal (`<>`) expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::val("Morning").ne("Good"))
    ///     .and_where(Expr::col(Char::Id).ne(1))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 'Morning' <> 'Good' AND "id" <> 1"#
    /// );
    /// ```
    pub fn ne<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::NotEqual, v)
    }

    /// Express a equal expression between two table columns,
    /// you will mainly use this to relate identical value between two table columns.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id)))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."font_id" = "font"."id""#
    /// );
    /// ```
    pub fn equals<C>(self, col: C) -> SimpleExpr
    where
        C: IntoColumnRef,
    {
        self.binary(BinOper::Equal, col.into_column_ref())
    }

    /// Express a not equal expression between two table columns,
    /// you will mainly use this to relate identical value between two table columns.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id)))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."font_id" = "font"."id""#
    /// );
    /// ```
    pub fn not_equals<C>(self, col: C) -> SimpleExpr
    where
        C: IntoColumnRef,
    {
        self.binary(BinOper::NotEqual, col.into_column_ref())
    }

    /// Express a greater than (`>`) expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).gt(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" > 2"#
    /// );
    /// ```
    pub fn gt<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::GreaterThan, v.into())
    }

    /// Express a greater than or equal (`>=`) expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).gte(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" >= 2"#
    /// );
    /// ```
    pub fn gte<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::GreaterThanOrEqual, v)
    }

    /// Express a less than (`<`) expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).lt(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" < 2"#
    /// );
    /// ```
    pub fn lt<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::SmallerThan, v)
    }

    /// Express a less than or equal (`<=`) expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).lte(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" <= 2"#
    /// );
    /// ```
    pub fn lte<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::SmallerThanOrEqual, v)
    }

    /// Express an arithmetic addition operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::val(1).add(1).eq(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 1 + 1 = 2"#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn add<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::Add, v)
    }

    /// Express an arithmetic subtraction operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::val(1).sub(1).eq(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 1 - 1 = 2"#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn sub<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::Sub, v)
    }

    /// Express an arithmetic multiplication operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::val(1).mul(1).eq(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 1 * 1 = 2"#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn mul<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::Mul, v)
    }

    /// Express an arithmetic division operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::val(1).div(1).eq(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 1 / 1 = 2"#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn div<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::Div, v)
    }

    /// Express an arithmetic modulo operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::val(1).modulo(1).eq(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 1 % 1 = 2"#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn modulo<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::Mod, v)
    }

    /// Express a bitwise left shift.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::val(1).left_shift(1).eq(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 1 << 1 = 2"#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn left_shift<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::LShift, v)
    }

    /// Express a bitwise right shift.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::val(1).right_shift(1).eq(2))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 1 >> 1 = 2"#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn right_shift<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::RShift, v)
    }

    /// Express a `BETWEEN` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).between(1, 10))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" BETWEEN 1 AND 10"#
    /// );
    /// ```
    pub fn between<V>(self, a: V, b: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.between_bounds(BinOper::Between, a, b)
    }

    /// Express a `NOT BETWEEN` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).not_between(1, 10))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" NOT BETWEEN 1 AND 10"#
    /// );
    /// ```
    pub fn not_between<V>(self, a: V, b: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.between_bounds(BinOper::NotBetween, a, b)
    }

    /// The shared shape of the BETWEEN family: a ternary construct encoded as
    /// a binary whose right operand is an `AND` binary, which the renderer
    /// unwraps for any operator its `is_between` predicate recognises.
    fn between_bounds<V>(self, op: BinOper, a: V, b: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(
            op,
            SimpleExpr::Binary(Box::new(a.into()), BinOper::And, Box::new(b.into())),
        )
    }

    /// Express a `LIKE` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::Character)).like("Ours'%"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."character" LIKE E'Ours\'%'"#
    /// );
    /// ```
    ///
    /// Like with ESCAPE
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::Character)).like(LikeExpr::new(r"|_Our|_").escape('|')))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."character" LIKE '|_Our|_' ESCAPE '|'"#
    /// );
    /// ```
    pub fn like<L: IntoLikeExpr>(self, like: L) -> SimpleExpr {
        self.like_like(BinOper::Like, like.into_like_expr())
    }

    /// Express a `NOT LIKE` expression
    pub fn not_like<L: IntoLikeExpr>(self, like: L) -> SimpleExpr {
        self.like_like(BinOper::NotLike, like.into_like_expr())
    }

    /// Express a `IS NULL` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).is_null())
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" IS NULL"#
    /// );
    /// ```
    #[allow(clippy::wrong_self_convention)]
    pub fn is_null(self) -> SimpleExpr {
        self.binary(BinOper::Is, Keyword::Null)
    }

    /// Express a `IS` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::Ascii)).is(true))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."ascii" IS TRUE"#
    /// );
    /// ```
    pub fn is<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::Is, v)
    }

    /// Express a `IS NOT NULL` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).is_not_null())
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" IS NOT NULL"#
    /// );
    /// ```
    #[allow(clippy::wrong_self_convention)]
    pub fn is_not_null(self) -> SimpleExpr {
        self.binary(BinOper::IsNot, Keyword::Null)
    }

    /// Express a `IS NOT` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::Ascii)).is_not(true))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."ascii" IS NOT TRUE"#
    /// );
    /// ```
    pub fn is_not<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::IsNot, v)
    }

    /// Create any binary operation
    ///
    /// # Examples
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .cond_where(
    ///         Condition::all()
    ///             .add(Expr::col(Char::SizeW).binary(BinOper::SmallerThan, 10))
    ///             .add(Expr::col(Char::SizeW).binary(BinOper::GreaterThan, Expr::col(Char::SizeH))),
    ///     )
    ///     .to_owned();
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "size_w" < 10 AND "size_w" > "size_h""#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.expr.operators+5]
    pub fn binary<O, T>(self, op: O, right: T) -> SimpleExpr
    where
        O: Into<BinOper>,
        T: Into<SimpleExpr>,
    {
        self.bin_op(op, right)
    }

    /// Negates an expression with `NOT`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::expr(Expr::col((Char::Table, Char::SizeW)).is_null()).not())
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE NOT "character"."size_w" IS NULL"#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> SimpleExpr {
        self.un_op(UnOper::Not)
    }

    /// Express a `MAX` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col((Char::Table, Char::SizeW)).max())
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT MAX("character"."size_w") FROM "character""#
    /// );
    /// ```
    pub fn max(self) -> SimpleExpr {
        Func::max(self.left).into()
    }

    /// Express a `MIN` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col((Char::Table, Char::SizeW)).min())
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT MIN("character"."size_w") FROM "character""#
    /// );
    /// ```
    pub fn min(self) -> SimpleExpr {
        Func::min(self.left).into()
    }

    /// Express a `SUM` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col((Char::Table, Char::SizeW)).sum())
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT SUM("character"."size_w") FROM "character""#
    /// );
    /// ```
    pub fn sum(self) -> SimpleExpr {
        Func::sum(self.left).into()
    }

    /// Express a `COUNT` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col((Char::Table, Char::SizeW)).count())
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT COUNT("character"."size_w") FROM "character""#
    /// );
    /// ```
    pub fn count(self) -> SimpleExpr {
        Func::count(self.left).into()
    }

    /// Express a `COUNT` function with the DISTINCT modifier.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col((Char::Table, Char::SizeW)).count_distinct())
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT COUNT(DISTINCT "character"."size_w") FROM "character""#
    /// );
    /// ```
    pub fn count_distinct(self) -> SimpleExpr {
        Func::count_distinct(self.left).into()
    }

    /// Express a `IF NULL` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col((Char::Table, Char::SizeW)).if_null(0))
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT COALESCE("character"."size_w", 0) FROM "character""#
    /// );
    /// ```
    pub fn if_null<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        Func::if_null(self.left, v).into()
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

    /// Express a `ALL` sub-query expression.
    pub fn all(sel: SelectStatement) -> SimpleExpr {
        SimpleExpr::SubQuery(
            Some(SubQueryOper::All),
            Box::new(sel.into_sub_query_statement()),
        )
    }
    /// Express a `AS enum` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col(Char::FontSize).as_enum(Name::runtime("text")))
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT CAST("font_size" AS text) FROM "character""#
    /// );
    ///
    /// let query = Query::insert()
    ///     .into_table(Char::Table)
    ///     .columns([Char::FontSize])
    ///     .values_panic([Expr::val("large").as_enum(Name::runtime("font_size_enum"))])
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"INSERT INTO "character" ("font_size") VALUES (CAST('large' AS font_size_enum))"#
    /// );
    /// ```
    pub fn as_enum<T>(self, type_name: T) -> SimpleExpr
    where
        T: IntoName,
    {
        self.cast_as_type(TypeName::new(type_name))
    }

    /// Cast to a structured [`TypeName`] — schema qualification and array
    /// suffix included. Every cast spelling funnels here, and the node it
    /// builds is the only shape a cast has.
    // [spec:pgorm:req:sql.ast.cast-shape+2]
    pub fn cast_as_type(self, type_name: TypeName) -> SimpleExpr {
        SimpleExpr::AsEnum(Box::new(type_name), Box::new(self.into()))
    }

    /// Cast to a type EXPRESSION rendered verbatim — `BIT(8)`,
    /// `numeric(12, 2)` — the escape hatch for spellings that are grammar
    /// rather than a name. The text is the caller's own SQL: nothing is
    /// quoted or escaped. The `&'static str` bound is what keeps that safe —
    /// only a literal written in the calling source can reach it, never a
    /// runtime string a value could have reached. A *name*, including one
    /// that arrives at runtime, belongs in [`cast_as`](Self::cast_as) or
    /// [`ColumnType::named`](crate::ColumnType::named), both of which quote
    /// it.
    ///
    /// The verbatim text rides in the [`TypeName`], so this builds the same
    /// node as every other cast.
    // [spec:pgorm:req:sql.ast.cast-shape+2]
    pub fn cast_as_raw(self, type_expr: &'static str) -> SimpleExpr {
        self.cast_as_type(TypeName::raw(type_expr))
    }

    /// Express a `CAST AS` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::val("1").cast_as(Name::runtime("integer")))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT CAST('1' AS integer)"#
    /// );
    /// ```
    pub fn cast_as<T>(self, type_name: T) -> SimpleExpr
    where
        T: IntoName,
    {
        self.cast_as_type(TypeName::new(type_name))
    }

    /// Keyword `CURRENT_TIMESTAMP`.
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

    /// Keyword `CURRENT_TIMESTAMP`.
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

    /// Express an postgres concatenate (`||`) expression.
    ///
    /// `||` concatenates strings, arrays and `jsonb` documents alike, so this
    /// is also the JSON merge operator and the JSON operator family carries no
    /// duplicate of it.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Font::Name, Font::Variant, Font::Language])
    ///     .from(Font::Table)
    ///     .and_where(Expr::val("a").concat("b").concat("c").concat("d"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "name", "variant", "language" FROM "font" WHERE 'a' || 'b' || 'c' || 'd'"#
    /// );
    /// ```
    pub fn concat<T>(self, right: T) -> SimpleExpr
    where
        T: Into<SimpleExpr>,
    {
        self.bin_op(BinOper::Concatenate, right)
    }

    /// Express an postgres fulltext search matches (`@@`) expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Font::Name, Font::Variant, Font::Language])
    ///     .from(Font::Table)
    ///     .and_where(Expr::val("a & b").matches("a b"))
    ///     .and_where(Expr::col(Font::Name).matches("a b"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "name", "variant", "language" FROM "font" WHERE 'a & b' @@ 'a b' AND "name" @@ 'a b'"#
    /// );
    /// ```
    pub fn matches<T>(self, expr: T) -> SimpleExpr
    where
        T: Into<SimpleExpr>,
    {
        self.bin_op(BinOper::Matches, expr)
    }

    /// Express a postgres containment (`@>`) expression.
    ///
    /// One operator serves every type PostgreSQL defines containment for —
    /// arrays, ranges, `tsquery`, and `jsonb` — so this is also the JSON
    /// containment test and the JSON operator family carries no duplicate of
    /// it. A `jsonb` right operand is any `serde_json::Value`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::UserData).contains(json!({ "a": 1 })))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "user_data" @> '{"a":1}'"#
    /// );
    /// ```
    pub fn contains<T>(self, expr: T) -> SimpleExpr
    where
        T: Into<SimpleExpr>,
    {
        self.bin_op(BinOper::Contains, expr)
    }

    /// Express a postgres containment (`<@`) expression, with the operands the
    /// other way round from [`Expr::contains`].
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::UserData).contained(json!({ "a": 1, "b": 2 })))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "user_data" <@ '{"a":1,"b":2}'"#
    /// );
    /// ```
    pub fn contained<T>(self, expr: T) -> SimpleExpr
    where
        T: Into<SimpleExpr>,
    {
        self.bin_op(BinOper::Contained, expr)
    }

    /// Express a `ILIKE` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::Character)).ilike("Ours'%"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."character" ILIKE E'Ours\'%'"#
    /// );
    /// ```
    pub fn ilike<L>(self, like: L) -> SimpleExpr
    where
        L: IntoLikeExpr,
    {
        self.like_like(BinOper::ILike, like.into_like_expr())
    }

    /// Express a `NOT ILIKE` expression
    pub fn not_ilike<L>(self, like: L) -> SimpleExpr
    where
        L: IntoLikeExpr,
    {
        self.like_like(BinOper::NotILike, like.into_like_expr())
    }
}

impl From<Expr> for SimpleExpr {
    /// Convert into SimpleExpr
    fn from(src: Expr) -> Self {
        if let Some(uopr) = src.uopr {
            SimpleExpr::Unary(uopr, Box::new(src.left))
        } else if let Some(bopr) = src.bopr {
            SimpleExpr::Binary(Box::new(src.left), bopr, Box::new(src.right.unwrap()))
        } else {
            src.left
        }
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

impl Expression for SimpleExpr {
    fn un_op(self, op: UnOper) -> SimpleExpr {
        SimpleExpr::Unary(op, Box::new(self))
    }

    fn bin_op<O, T>(self, op: O, right: T) -> SimpleExpr
    where
        O: Into<BinOper>,
        T: Into<SimpleExpr>,
    {
        SimpleExpr::Binary(Box::new(self), op.into(), Box::new(right.into()))
    }
}

impl SimpleExpr {
    /// Negates an expression with `NOT`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::SizeW)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::SizeW).eq(1).not())
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "size_w" FROM "character" WHERE NOT "size_w" = 1"#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> SimpleExpr {
        self.un_op(UnOper::Not)
    }

    /// Express a logical `AND` operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .cond_where(Condition::any()
    ///         .add(Expr::col(Char::SizeW).eq(1).and(Expr::col(Char::SizeH).eq(2)))
    ///         .add(Expr::col(Char::SizeW).eq(3).and(Expr::col(Char::SizeH).eq(4))))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE ("size_w" = 1 AND "size_h" = 2) OR ("size_w" = 3 AND "size_h" = 4)"#
    /// );
    /// ```
    pub fn and(self, right: SimpleExpr) -> Self {
        self.binary(BinOper::And, right)
    }

    /// Express a logical `OR` operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::SizeW).eq(1).or(Expr::col(Char::SizeH).eq(2)))
    ///     .and_where(Expr::col(Char::SizeW).eq(3).or(Expr::col(Char::SizeH).eq(4)))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE ("size_w" = 1 OR "size_h" = 2) AND ("size_w" = 3 OR "size_h" = 4)"#
    /// );
    /// ```
    pub fn or(self, right: SimpleExpr) -> Self {
        self.binary(BinOper::Or, right)
    }

    /// Express an equal (`=`) expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::value("What!").eq("Nothing"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 'What!' = 'Nothing'"#
    /// );
    /// ```
    pub fn eq<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::Equal, v)
    }

    /// Express a not equal (`<>`) expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::value("Morning").ne("Good"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 'Morning' <> 'Good'"#
    /// );
    /// ```
    pub fn ne<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        self.binary(BinOper::NotEqual, v)
    }

    /// Perform addition with another [`SimpleExpr`].
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(
    ///         Expr::col(Char::SizeW)
    ///             .max()
    ///             .add(Expr::col(Char::SizeH).max()),
    ///     )
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT MAX("size_w") + MAX("size_h") FROM "character""#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn add<T>(self, right: T) -> Self
    where
        T: Into<SimpleExpr>,
    {
        self.binary(BinOper::Add, right)
    }

    /// Perform multiplication with another [`SimpleExpr`].
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(
    ///         Expr::col(Char::SizeW)
    ///             .max()
    ///             .mul(Expr::col(Char::SizeH).max()),
    ///     )
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT MAX("size_w") * MAX("size_h") FROM "character""#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn mul<T>(self, right: T) -> Self
    where
        T: Into<SimpleExpr>,
    {
        self.binary(BinOper::Mul, right.into())
    }

    /// Perform division with another [`SimpleExpr`].
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(
    ///         Expr::col(Char::SizeW)
    ///             .max()
    ///             .div(Expr::col(Char::SizeH).max()),
    ///     )
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT MAX("size_w") / MAX("size_h") FROM "character""#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn div<T>(self, right: T) -> Self
    where
        T: Into<SimpleExpr>,
    {
        self.binary(BinOper::Div, right.into())
    }

    /// Perform subtraction with another [`SimpleExpr`].
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(
    ///         Expr::col(Char::SizeW)
    ///             .max()
    ///             .sub(Expr::col(Char::SizeW).min()),
    ///     )
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT MAX("size_w") - MIN("size_w") FROM "character""#
    /// );
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn sub<T>(self, right: T) -> Self
    where
        T: Into<SimpleExpr>,
    {
        self.binary(BinOper::Sub, right)
    }

    /// Express a `CAST AS` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::value("1").cast_as(Name::runtime("integer")))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT CAST('1' AS integer)"#
    /// );
    /// ```
    pub fn cast_as<T>(self, type_name: T) -> Self
    where
        T: IntoName,
    {
        Self::AsEnum(Box::new(TypeName::new(type_name)), Box::new(self))
    }

    /// Cast to a structured [`TypeName`], the [`Expr::cast_as_type`] of an
    /// already-built expression.
    pub fn cast_as_type(self, type_name: TypeName) -> Self {
        Self::AsEnum(Box::new(type_name), Box::new(self))
    }

    /// Create any binary operation
    ///
    /// # Examples
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .cond_where(
    ///         Condition::all()
    ///             .add(Expr::value(10).binary(BinOper::SmallerThan, Expr::col(Char::SizeW)))
    ///             .add(Expr::value(20).binary(BinOper::GreaterThan, Expr::col(Char::SizeH))),
    ///     )
    ///     .to_owned();
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE 10 < "size_w" AND 20 > "size_h""#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.expr.operators+5]
    pub fn binary<O, T>(self, op: O, right: T) -> Self
    where
        O: Into<BinOper>,
        T: Into<SimpleExpr>,
    {
        self.bin_op(op, right)
    }

    /// Express a `LIKE` expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::FontId)).cast_as(Name::runtime("text")).like("a%"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE CAST("character"."font_id" AS text) LIKE 'a%'"#
    /// );
    /// ```
    pub fn like<L: IntoLikeExpr>(self, like: L) -> Self {
        self.like_like(BinOper::Like, like.into_like_expr())
    }

    /// Express a `NOT LIKE` expression
    pub fn not_like<L: IntoLikeExpr>(self, like: L) -> Self {
        self.like_like(BinOper::NotLike, like.into_like_expr())
    }

    pub(crate) fn get_bin_oper(&self) -> Option<&BinOper> {
        match self {
            Self::Binary(_, oper, _) => Some(oper),
            _ => None,
        }
    }

    /// Express an postgres concatenate (`||`) expression.
    ///
    /// `||` concatenates strings, arrays and `jsonb` documents alike, so this
    /// is also the JSON merge operator and the JSON operator family carries no
    /// duplicate of it.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Font::Name, Font::Variant, Font::Language])
    ///     .from(Font::Table)
    ///     .and_where(Expr::val("a").concat("b").concat("c").concat("d"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "name", "variant", "language" FROM "font" WHERE 'a' || 'b' || 'c' || 'd'"#
    /// );
    /// ```
    pub fn concat<T>(self, right: T) -> Self
    where
        T: Into<SimpleExpr>,
    {
        self.bin_op(BinOper::Concatenate, right)
    }
}
