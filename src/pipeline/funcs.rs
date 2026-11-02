//! Aggregate and window functions, `case`, `null`, and the cast type set.
//!
//! Every argument takes `impl Into<Expr>`, so a column, an alias token, a
//! Rust literal or an expression all read the same way: `sum(O::Total)`.

use super::adapter;
use super::expr::{Expr, branded, name};

/// The `NULL` literal.
///
/// The one literal with no Rust spelling: comparing against it with
/// [`eq`](super::ExprOps::eq) / [`ne`](super::ExprOps::ne) emits
/// `IS [NOT] NULL`. Every other literal is written as itself — `1`, `1.5`,
/// `"text"`, `true` — and inlines into the SQL exactly as a literal in PRQL
/// text does.
pub fn null<'brand>() -> Expr<'brand> {
    branded(adapter::lit_null())
}

fn unary_fn<'brand>(func: &str, expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    branded(adapter::call(func, vec![expr.into().node]))
}

/// `SUM(expr)`.
pub fn sum<'brand>(expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    unary_fn("sum", expr)
}

/// `MIN(expr)`. An entity column whose domain is over an enum is cast to the
/// enum, PostgreSQL resolving no `min` for the domain itself.
// [spec:pgorm:sem:entity.traits.column.enum-cast+13]
pub fn min<'brand>(expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    branded(adapter::call("min", vec![expr.into().compared()]))
}

/// `MAX(expr)`. An entity column whose domain is over an enum is cast to the
/// enum, PostgreSQL resolving no `max` for the domain itself.
// [spec:pgorm:sem:entity.traits.column.enum-cast+13]
pub fn max<'brand>(expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    branded(adapter::call("max", vec![expr.into().compared()]))
}

/// `AVG(expr)`.
pub fn average<'brand>(expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    unary_fn("average", expr)
}

/// `STDDEV(expr)`.
pub fn stddev<'brand>(expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    unary_fn("stddev", expr)
}

/// `COUNT(expr)`: how many rows the expression is not null on.
///
/// The counterpart of [`count_rows`], and a different answer wherever the
/// expression is nullable — which is why they are two functions and not one
/// ([spec:pgorm:sem:pipeline.count-argument]).
// [spec:pgorm:sem:pipeline.count-argument]
pub fn count<'brand>(expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    unary_fn("count", expr)
}

/// `COUNT(*)`: how many rows there are, nulls and all.
// [spec:pgorm:sem:pipeline.count-argument]
pub fn count_rows<'brand>() -> Expr<'brand> {
    unary_fn("count", name("this"))
}

/// `COUNT(DISTINCT expr)`.
pub fn count_distinct<'brand>(expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    unary_fn("count_distinct", expr)
}

/// `ROW_NUMBER()` over the enclosing window.
pub fn row_number<'brand>() -> Expr<'brand> {
    unary_fn("row_number", name("this"))
}

/// `RANK()` over the enclosing window.
///
/// PRQL's `rank` takes the ranked column (the rank itself comes from the
/// window's ordering; the argument names what is being ranked).
pub fn rank<'brand>(expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    unary_fn("rank", expr)
}

/// `DENSE_RANK()` over the enclosing window; argument as in [`rank`].
pub fn rank_dense<'brand>(expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    unary_fn("rank_dense", expr)
}

/// `LAG(expr, offset)` over the enclosing window.
pub fn lag<'brand>(offset: i64, expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    branded(adapter::call(
        "lag",
        vec![adapter::lit_int(offset), expr.into().node],
    ))
}

/// `LEAD(expr, offset)` over the enclosing window.
pub fn lead<'brand>(offset: i64, expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    branded(adapter::call(
        "lead",
        vec![adapter::lit_int(offset), expr.into().node],
    ))
}

/// `FIRST_VALUE(expr)` over the enclosing window.
pub fn first<'brand>(expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    unary_fn("first", expr)
}

/// `LAST_VALUE(expr)` over the enclosing window.
pub fn last<'brand>(expr: impl Into<Expr<'brand>>) -> Expr<'brand> {
    unary_fn("last", expr)
}

/// A `CASE WHEN ... THEN ... ELSE ... END` expression.
///
/// Each arm is a `(condition, value)` pair, tried in order; `otherwise` is
/// mandatory, so an unmatched row yields the stated fallback rather than an
/// implicit `NULL` (make the fallback [`null`] to say that explicitly).
///
/// ```
/// # use pgorm::pipeline::{ExprOps, case};
/// # use pgorm::tests_cfg::cake::Column as C;
/// let bucket = case([(C::Id.gt(100), "big")], "small");
/// ```
pub fn case<'brand, I, W, T>(arms: I, otherwise: impl Into<Expr<'brand>>) -> Expr<'brand>
where
    I: IntoIterator<Item = (W, T)>,
    W: Into<Expr<'brand>>,
    T: Into<Expr<'brand>>,
{
    let mut pairs: Vec<_> = arms
        .into_iter()
        .map(|(condition, value)| (condition.into().node, value.into().node))
        .collect();
    pairs.push((adapter::lit_bool(true), otherwise.into().node));
    branded(adapter::case(pairs))
}

/// The types [`cast`](super::ExprOps::cast) can target.
///
/// A closed set: the name reaches the SQL text verbatim as `CAST(x AS name)`,
/// so an open string here would be an interpolation hole. Types outside this
/// list are outside the v1 surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CastType {
    /// `smallint`
    SmallInt,
    /// `integer`
    Integer,
    /// `bigint`
    BigInt,
    /// `real`
    Real,
    /// `float8` (double precision)
    Double,
    /// `numeric`
    Numeric,
    /// `text`
    Text,
    /// `boolean`
    Boolean,
    /// `date`
    Date,
    /// `timestamp`
    Timestamp,
    /// `timestamptz`
    Timestamptz,
    /// `interval`
    Interval,
    /// `uuid`
    Uuid,
    /// `json`
    Json,
    /// `jsonb`
    Jsonb,
}

impl CastType {
    pub(super) fn name(self) -> &'static str {
        match self {
            CastType::SmallInt => "smallint",
            CastType::Integer => "integer",
            CastType::BigInt => "bigint",
            CastType::Real => "real",
            CastType::Double => "float8",
            CastType::Numeric => "numeric",
            CastType::Text => "text",
            CastType::Boolean => "boolean",
            CastType::Date => "date",
            CastType::Timestamp => "timestamp",
            CastType::Timestamptz => "timestamptz",
            CastType::Interval => "interval",
            CastType::Uuid => "uuid",
            CastType::Json => "json",
            CastType::Jsonb => "jsonb",
        }
    }
}
