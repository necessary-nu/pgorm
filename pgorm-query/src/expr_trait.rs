//! The operator vocabulary every expression has: comparison, arithmetic,
//! pattern matching, membership, casts, the aggregate shorthands, the JSON
//! operators, subscripts and collation.
//!
//! One trait carries it, and each type that is an expression in its own right
//! implements it, so what a method builds can be built on in turn: a function
//! call, a cast and an aggregate compare and combine exactly as a column does.

use crate::{
    ArrayElement, ArrayType, BinOper, CaseStatement, Expr, Func, FunctionCall, Grouping,
    IntoCollation, IntoColumnRef, IntoLikeExpr, IntoName, IntoValueTuple, JsonArray, JsonArrayAgg,
    JsonArrayQuery, JsonExists, JsonInput, JsonObject, JsonObjectAgg, JsonParse, JsonQuery,
    JsonSerialize, JsonTest, JsonValue, Keyword, Name, SelectStatement, SimpleCaseStatement,
    SimpleExpr, SqlJson, TypeName, UnOper, Value,
    value_range_text::{multirange_text, range_text},
};

/// `fn op(self, v) -> SimpleExpr`: the receiver and `v` under one binary
/// operator.
macro_rules! binary_operators {
    ($( $(#[$meta:meta])* $name:ident => $op:ident, )*) => {
        $(
            $(#[$meta])*
            fn $name<V>(self, v: V) -> SimpleExpr
            where
                V: Into<SimpleExpr>,
            {
                self.binary(BinOper::$op, v)
            }
        )*
    };
}

/// `fn op(self, pattern) -> SimpleExpr`: the LIKE family, whose right operand
/// is a pattern with its optional `ESCAPE`.
macro_rules! pattern_operators {
    ($( $(#[$meta:meta])* $name:ident => $op:ident, )*) => {
        $(
            $(#[$meta])*
            fn $name<L>(self, pattern: L) -> SimpleExpr
            where
                L: IntoLikeExpr,
            {
                self.binary(BinOper::$op, SimpleExpr::LikePattern(pattern.into_like_expr()))
            }
        )*
    };
}

/// `fn op(self, a, b) -> SimpleExpr`: the BETWEEN family, whose bounds are
/// one `a AND b` operand. The bounds are typed apart, since one may be a
/// value and the other an expression built on.
macro_rules! between_operators {
    ($( $(#[$meta:meta])* $name:ident => $op:ident, )*) => {
        $(
            $(#[$meta])*
            fn $name<A, B>(self, a: A, b: B) -> SimpleExpr
            where
                A: Into<SimpleExpr>,
                B: Into<SimpleExpr>,
            {
                self.binary(
                    BinOper::$op,
                    SimpleExpr::Binary(Box::new(a.into()), BinOper::And, Box::new(b.into())),
                )
            }
        )*
    };
}

/// `fn op(self, items) -> SimpleExpr`: a JSON operator whose right operand is
/// one `text[]` parameter, a path or a list of keys.
macro_rules! text_array_operators {
    ($( $(#[$meta:meta])* $name:ident($items:ident) => $op:ident, )*) => {
        $(
            $(#[$meta])*
            fn $name<S, I>(self, $items: I) -> SimpleExpr
            where
                S: Into<String>,
                I: IntoIterator<Item = S>,
            {
                self.binary(BinOper::$op, Value::array($items.into_iter().map(Into::into)))
            }
        )*
    };
}

/// `fn agg(self) -> SimpleExpr`: the receiver as the one argument of an
/// aggregate function.
macro_rules! aggregates {
    ($( $(#[$meta:meta])* $name:ident, )*) => {
        $(
            $(#[$meta])*
            fn $name(self) -> SimpleExpr {
                Func::$name(self).into()
            }
        )*
    };
}

/// The comparisons, operators, casts, aggregates and predicates every
/// expression has.
///
/// It is implemented by [`Expr`], by the [`SimpleExpr`] each of its methods
/// returns, by a [`FunctionCall`] such as `Func::upper(..)`, by a `CASE`
/// ([`CaseStatement`], [`SimpleCaseStatement`]), by `GROUPING(..)`
/// ([`Grouping`]) and by the SQL/JSON constructors. A function call, a cast
/// and an aggregate therefore compare and combine as a column does, with no
/// [`Expr::expr`] around them: `Func::lower(..).eq(..)`,
/// `Expr::col(..).count().gt(2)`, `Expr::col(..).cast_as(..).lt(10)`.
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let query = Query::select()
///     .column(Char::FontSize)
///     .expr(Func::upper(Expr::col(Char::Character)).concat("!"))
///     .from(Char::Table)
///     .and_where(Func::lower(Expr::col(Char::Character)).ne("a"))
///     .and_where(Expr::col(Char::SizeW).cast_as(Name::runtime("text")).like("1%"))
///     .group_by_columns([Char::FontSize, Char::Character])
///     .and_having(Expr::col(Char::Id).count().gt(2))
///     .to_owned();
///
/// assert_eq!(
///     query.to_string(),
///     [
///         r#"SELECT "font_size", UPPER("character") || '!' FROM "character""#,
///         r#"WHERE LOWER("character") <> 'a' AND CAST("size_w" AS text) LIKE '1%'"#,
///         r#"GROUP BY "font_size", "character" HAVING COUNT("id") > 2"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// The trait is exported from the crate root, so `use pgorm_query::*` brings
/// it, as pgorm's prelude does; a module that names its imports one by one
/// names `ExprTrait` beside `Expr`. Every method consumes its receiver and
/// builds a new [`SimpleExpr`]: an expression is a value under construction,
/// never inspected, which is also why the predicates (`is_null`, `is_in`)
/// take `self`. Taking `self` by value is what makes `eq`, `ne` and the
/// orderings this trait's rather than `PartialEq`'s or `PartialOrd`'s, which
/// borrow.
// [spec:pgorm:def:sql.ast.expr+8]
// [spec:pgorm:req:sql.ast.expr.operators+6]
#[allow(clippy::wrong_self_convention)]
pub trait ExprTrait: Into<SimpleExpr> + Sized {
    binary_operators! {
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
        ///
        /// On an expression already built:
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
        eq => Equal,
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
        ///
        /// On an expression already built:
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
        ne => NotEqual,
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
        gt => GreaterThan,
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
        gte => GreaterThanOrEqual,
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
        lt => SmallerThan,
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
        lte => SmallerThanOrEqual,
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
        ///
        /// On an expression already built:
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
        add => Add,
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
        ///
        /// On an expression already built:
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
        sub => Sub,
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
        ///
        /// On an expression already built:
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
        mul => Mul,
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
        ///
        /// On an expression already built:
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
        div => Div,
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
        modulo => Mod,
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
        left_shift => LShift,
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
        right_shift => RShift,
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
        is => Is,
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
        is_not => IsNot,
        /// Express an `IS DISTINCT FROM` expression: inequality that reads NULL as
        /// a value rather than as unknown.
        ///
        /// `<>` is NULL whenever either side is, so a row with a NULL passes
        /// neither `a <> b` nor its negation. This operator answers true or false
        /// for every pair — two NULLs are not distinct, a NULL and a non-NULL are
        /// — which is what makes it the comparison to reach for when the column
        /// is nullable and the query means "changed".
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{*, tests_cfg::*};
        ///
        /// let query = Query::select()
        ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
        ///     .from(Char::Table)
        ///     .and_where(Expr::col((Char::Table, Char::SizeW)).is_distinct_from(1))
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" IS DISTINCT FROM 1"#
        /// );
        /// ```
        is_distinct_from => IsDistinctFrom,
        /// Express an `IS NOT DISTINCT FROM` expression: the null-safe equality
        /// complementing [`is_distinct_from`](ExprTrait::is_distinct_from).
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{*, tests_cfg::*};
        ///
        /// let query = Query::select()
        ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
        ///     .from(Char::Table)
        ///     .and_where(Expr::col((Char::Table, Char::SizeW)).is_not_distinct_from(1))
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" IS NOT DISTINCT FROM 1"#
        /// );
        /// ```
        is_not_distinct_from => IsNotDistinctFrom,
        /// Express an `AT TIME ZONE` expression, reinterpreting a timestamp in the
        /// named zone.
        ///
        /// The direction follows the left operand's type, as PostgreSQL's operator
        /// does: applied to a `timestamptz` it yields the local `timestamp` in that
        /// zone, and applied to a `timestamp` it reads the value *as* that zone's
        /// local time and yields a `timestamptz`.
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{*, tests_cfg::*};
        ///
        /// let query = Query::select()
        ///     .expr(Expr::col(Char::CreatedAt).at_time_zone("UTC"))
        ///     .from(Char::Table)
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "created_at" AT TIME ZONE 'UTC' FROM "character""#
        /// );
        /// ```
        at_time_zone => AtTimeZone,
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
        concat => Concatenate,
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
        matches => Matches,
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
        contains => Contains,
        /// Express a postgres containment (`<@`) expression, with the operands the
        /// other way round from [`contains`](ExprTrait::contains).
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
        contained => Contained,
        /// Express a postgres overlap (`&&`) expression: whether the two operands
        /// have a value in common.
        ///
        /// One operator serves ranges, multiranges (against a range or another
        /// multirange) and arrays, so this is each of their overlap tests. A bound
        /// right operand beside a range column is typed as that range by the
        /// server, as it is for [`contains`](ExprTrait::contains): an element is not an
        /// operand of `&&` at all (`42883`), and the empty range overlaps nothing.
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{*, tests_cfg::*};
        ///
        /// let query = Query::select()
        ///     .column(Char::Id)
        ///     .from(Char::Table)
        ///     .and_where(Expr::col(Char::Character).overlaps(Range::from(1..5)))
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "id" FROM "character" WHERE "character" && int4range(1, 5, '[)')"#
        /// );
        /// assert_eq!(
        ///     query.build().0,
        ///     r#"SELECT "id" FROM "character" WHERE "character" && $1"#
        /// );
        /// ```
        // [spec:pgorm:req:sql.ast.expr.operators+6]
        overlaps => Overlap,
        /// Express a postgres retrieves JSON field as JSON value (`->`).
        ///
        /// The right operand is any expression, because `->` selects an object key
        /// by text *or* an array element by integer, and the two are the same
        /// operator. Drilling more than one level means re-entering the builder —
        /// `Expr::expr(a.get_json_field("x")).get_json_field("y")` — so
        /// [`get_json_path`](ExprTrait::get_json_path) is the paved road for a multi-step path.
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{tests_cfg::*, *};
        ///
        /// let query = Query::select()
        ///     .column(Font::Variant)
        ///     .from(Font::Table)
        ///     .and_where(Expr::col(Font::Variant).get_json_field("a"))
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "variant" FROM "font" WHERE "variant" -> 'a'"#
        /// );
        /// ```
        // [spec:pgorm:req:sql.ast.expr.json+3]
        get_json_field => GetJsonField,
        /// Express a postgres retrieves JSON field and casts it to an appropriate SQL type (`->>`).
        ///
        /// The result is `text`, not JSON, which is what makes this the end of a
        /// chain: no JSON operator applies to what it returns.
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{tests_cfg::*, *};
        ///
        /// let query = Query::select()
        ///     .column(Font::Variant)
        ///     .from(Font::Table)
        ///     .and_where(Expr::col(Font::Variant).cast_json_field("a"))
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "variant" FROM "font" WHERE "variant" ->> 'a'"#
        /// );
        /// ```
        // [spec:pgorm:req:sql.ast.expr.json+3]
        cast_json_field => CastJsonField,
    }

    pattern_operators! {
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
        ///
        /// On an expression already built:
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
        like => Like,
        /// Express a `NOT LIKE` expression
        not_like => NotLike,
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
        ilike => ILike,
        /// Express a `NOT ILIKE` expression
        not_ilike => NotILike,
        /// Express a `SIMILAR TO` expression: whether the whole string matches a
        /// pattern in SQL's regular-expression dialect, which is LIKE's `%` and
        /// `_` with `|`, `*`, `+`, `?`, `{m,n}`, grouping and bracket classes
        /// added. The pattern is bound like [`like`](Self::like)'s, and a
        /// [`LikeExpr`] carries an `ESCAPE` character the same way; the default
        /// escape is a backslash, and an escape longer than one character is
        /// PostgreSQL's refusal (`22025`).
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{*, tests_cfg::*};
        ///
        /// let (sql, values) = Query::select()
        ///     .column(Char::Character)
        ///     .from(Char::Table)
        ///     .and_where(Expr::col(Char::Character).similar_to("(A|B)%"))
        ///     .and_where(Expr::col(Char::Character).not_similar_to(LikeExpr::new("!%%").escape('!')))
        ///     .build();
        ///
        /// assert_eq!(
        ///     sql,
        ///     r#"SELECT "character" FROM "character" WHERE "character" SIMILAR TO $1 AND "character" NOT SIMILAR TO $2 ESCAPE '!'"#
        /// );
        /// assert_eq!(values.0, vec!["(A|B)%".into(), "!%%".into()]);
        /// ```
        // [spec:pgorm:req:sql.ast.expr.similar-to+3]
        similar_to => SimilarTo,
        /// Express a `NOT SIMILAR TO` expression, the complement of
        /// [`similar_to`](ExprTrait::similar_to).
        // [spec:pgorm:req:sql.ast.expr.similar-to+3]
        not_similar_to => NotSimilarTo,
    }

    between_operators! {
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
        between => Between,
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
        not_between => NotBetween,
        /// Express a `BETWEEN SYMMETRIC` expression, which sorts its two bounds
        /// before testing and so holds whichever order they are given in. Plain
        /// `BETWEEN` matches nothing when the larger bound is written first.
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{*, tests_cfg::*};
        ///
        /// let query = Query::select()
        ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
        ///     .from(Char::Table)
        ///     .and_where(Expr::col((Char::Table, Char::SizeW)).between_symmetric(10, 1))
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" BETWEEN SYMMETRIC 10 AND 1"#
        /// );
        /// ```
        between_symmetric => BetweenSymmetric,
        /// Express a `NOT BETWEEN SYMMETRIC` expression, the complement of
        /// [`between_symmetric`](ExprTrait::between_symmetric).
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{*, tests_cfg::*};
        ///
        /// let query = Query::select()
        ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
        ///     .from(Char::Table)
        ///     .and_where(Expr::col((Char::Table, Char::SizeW)).not_between_symmetric(10, 1))
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "character"."size_w" NOT BETWEEN SYMMETRIC 10 AND 1"#
        /// );
        /// ```
        not_between_symmetric => NotBetweenSymmetric,
    }

    text_array_operators! {
        /// Express a postgres retrieves the JSON value at a path (`#>`).
        ///
        /// One operator walks the whole path, where `->` walks one step, so a
        /// three-deep lookup is one node instead of three nested ones. The path is
        /// a `text[]`, gathered into a single parameter the way
        /// [`eq_any`](ExprTrait::eq_any) gathers a list: depth varies without varying the
        /// statement text.
        ///
        /// Path steps are text even when they index into a JSON array, because the
        /// array is `text[]` and can hold nothing else; PostgreSQL reads a step
        /// that parses as an integer as a subscript. An empty path selects the
        /// document itself.
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{tests_cfg::*, *};
        ///
        /// let query = Query::select()
        ///     .column(Char::Id)
        ///     .from(Char::Table)
        ///     .and_where(Expr::expr(Expr::col(Char::UserData).get_json_path(["a", "b"])).is_not_null())
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "id" FROM "character" WHERE ("user_data" #> ARRAY ['a','b']) IS NOT NULL"#
        /// );
        ///
        /// let (sql, values) = query.build();
        /// assert_eq!(
        ///     sql,
        ///     r#"SELECT "id" FROM "character" WHERE ("user_data" #> $1) IS NOT NULL"#
        /// );
        /// assert_eq!(values.0.len(), 1);
        /// ```
        // [spec:pgorm:req:sql.ast.expr.json+3]
        get_json_path(path) => GetJsonPath,
        /// Express a postgres retrieves the value at a JSON path as text (`#>>`).
        ///
        /// [`get_json_path`](ExprTrait::get_json_path) with the `->>` ending: the same path walk, the
        /// same single `text[]` parameter, but the result is `text` rather than
        /// JSON, so it compares against Rust strings without a cast.
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{tests_cfg::*, *};
        ///
        /// let query = Query::select()
        ///     .column(Char::Id)
        ///     .from(Char::Table)
        ///     .and_where(Expr::expr(Expr::col(Char::UserData).cast_json_path(["a", "b"])).eq("x"))
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "id" FROM "character" WHERE ("user_data" #>> ARRAY ['a','b']) = 'x'"#
        /// );
        /// ```
        // [spec:pgorm:req:sql.ast.expr.json+3]
        cast_json_path(path) => CastJsonPath,
        /// Express a postgres any-key existence test (`?|`).
        ///
        /// [`has_json_key`](ExprTrait::has_json_key) over a list, in one `text[]` parameter. An empty
        /// list is false for every operand: no key of none can be present.
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{tests_cfg::*, *};
        ///
        /// let query = Query::select()
        ///     .column(Char::Id)
        ///     .from(Char::Table)
        ///     .and_where(Expr::col(Char::UserData).has_any_json_keys(["a", "b"]))
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "id" FROM "character" WHERE "user_data" ?| ARRAY ['a','b']"#
        /// );
        /// ```
        /// Empty key list
        /// ```
        /// use pgorm_query::{tests_cfg::*, *};
        ///
        /// let query = Query::select()
        ///     .column(Char::Id)
        ///     .from(Char::Table)
        ///     .and_where(Expr::col(Char::UserData).has_any_json_keys(Vec::<String>::new()))
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "id" FROM "character" WHERE "user_data" ?| ARRAY []::text[]"#
        /// );
        /// ```
        // [spec:pgorm:req:sql.ast.expr.json+3]
        has_any_json_keys(keys) => HasAnyJsonKeys,
        /// Express a postgres all-keys existence test (`?&`).
        ///
        /// The conjunctive counterpart of [`has_any_json_keys`](ExprTrait::has_any_json_keys), and its
        /// vacuous mirror: an empty list is true for every operand, because nothing
        /// is missing from a document when nothing was asked for. The same
        /// asymmetry [`eq_any`](ExprTrait::eq_any) and [`ne_all`](ExprTrait::ne_all) carry over the empty
        /// array.
        ///
        /// # Examples
        ///
        /// ```
        /// use pgorm_query::{tests_cfg::*, *};
        ///
        /// let query = Query::select()
        ///     .column(Char::Id)
        ///     .from(Char::Table)
        ///     .and_where(Expr::col(Char::UserData).has_all_json_keys(["a", "b"]))
        ///     .to_owned();
        ///
        /// assert_eq!(
        ///     query.to_string(),
        ///     r#"SELECT "id" FROM "character" WHERE "user_data" ?& ARRAY ['a','b']"#
        /// );
        ///
        /// let (sql, values) = query.build();
        /// assert_eq!(sql, r#"SELECT "id" FROM "character" WHERE "user_data" ?& $1"#);
        /// assert_eq!(values.0.len(), 1);
        /// ```
        // [spec:pgorm:req:sql.ast.expr.json+3]
        has_all_json_keys(keys) => HasAllJsonKeys,
    }

    aggregates! {
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
        max,
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
        min,
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
        sum,
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
        count,
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
        count_distinct,
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
    ///
    /// On an expression already built:
    ///
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
    // [spec:pgorm:req:sql.ast.expr.operators+6]
    fn binary<O, T>(self, op: O, right: T) -> SimpleExpr
    where
        O: Into<BinOper>,
        T: Into<SimpleExpr>,
    {
        SimpleExpr::Binary(Box::new(self.into()), op.into(), Box::new(right.into()))
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
    fn equals<C>(self, col: C) -> SimpleExpr
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
    fn not_equals<C>(self, col: C) -> SimpleExpr
    where
        C: IntoColumnRef,
    {
        self.binary(BinOper::NotEqual, col.into_column_ref())
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
    fn and(self, right: SimpleExpr) -> SimpleExpr {
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
    fn or(self, right: SimpleExpr) -> SimpleExpr {
        self.binary(BinOper::Or, right)
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
    ///
    /// On an expression already built:
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
    fn not(self) -> SimpleExpr {
        SimpleExpr::Unary(UnOper::Not, Box::new(self.into()))
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
    fn is_null(self) -> SimpleExpr {
        self.binary(BinOper::Is, Keyword::Null)
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
    fn is_not_null(self) -> SimpleExpr {
        self.binary(BinOper::IsNot, Keyword::Null)
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
    fn if_null<V>(self, v: V) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
    {
        Func::if_null(self, v).into()
    }

    /// Express a `IN` expression.
    ///
    /// One placeholder per element, so each list length is a distinct
    /// statement: fine for a short literal list, and [`eq_any`](ExprTrait::eq_any) is the
    /// PostgreSQL-idiomatic alternative when the length varies at runtime.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Id])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).is_in([1, 2, 3]))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "character"."size_w" IN (1, 2, 3)"#
    /// );
    /// ```
    /// Empty value list
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Id])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).is_in(Vec::<i32>::new()))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE 'a' = 'b'"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.expr.in+1]
    fn is_in<V, I>(self, v: I) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
        I: IntoIterator<Item = V>,
    {
        self.binary(
            BinOper::In,
            SimpleExpr::Tuple(v.into_iter().map(Into::into).collect()),
        )
    }

    /// Express a `NOT IN` expression. See [`ne_all`](ExprTrait::ne_all) for the
    /// array-parameter alternative.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Id])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).is_not_in([1, 2, 3]))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "character"."size_w" NOT IN (1, 2, 3)"#
    /// );
    /// ```
    /// Empty value list
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Id])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).is_not_in(Vec::<i32>::new()))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE 'a' = 'a'"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.expr.in+1]
    fn is_not_in<V, I>(self, v: I) -> SimpleExpr
    where
        V: Into<SimpleExpr>,
        I: IntoIterator<Item = V>,
    {
        self.binary(
            BinOper::NotIn,
            SimpleExpr::Tuple(v.into_iter().map(Into::into).collect()),
        )
    }

    /// Express a `= ANY` membership test against one array parameter.
    ///
    /// The PostgreSQL-idiomatic counterpart of [`is_in`](ExprTrait::is_in): where `IN`
    /// spends one placeholder per element — so every list length is a distinct
    /// statement, and a long list walks toward the 65535-parameter ceiling —
    /// `= ANY` spends exactly one, whatever the list length. The SQL text is
    /// then the same for two elements and for two thousand, which is what a
    /// per-connection prepared-statement cache keys on.
    ///
    /// Reach for `is_in` when the list is a short literal written into the
    /// query, and for `eq_any` when its length varies at runtime.
    ///
    /// An empty list needs no special case: `= ANY` over an empty array is
    /// false for every operand, NULL included, which is the vacuous truth
    /// `is_in` has to fall back to a constant comparison to express.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Id])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).eq_any([1, 2, 3]))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "character"."size_w" = ANY(ARRAY [1,2,3])"#
    /// );
    ///
    /// let (sql, values) = query.build();
    /// assert_eq!(
    ///     sql,
    ///     r#"SELECT "id" FROM "character" WHERE "character"."size_w" = ANY($1)"#
    /// );
    /// assert_eq!(values.0.len(), 1);
    /// ```
    /// Empty value list
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Id])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).eq_any(Vec::<i32>::new()))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "character"."size_w" = ANY(ARRAY []::int4[])"#
    /// );
    /// ```
    /// The elements are any [`ArrayElement`], so string slices make a `text[]`
    /// as `eq` takes one:
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Id])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::Character).eq_any(["A", "B"]))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "character" = ANY(ARRAY ['A','B'])"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.expr.eq-any+2]
    fn eq_any<V, I>(self, v: I) -> SimpleExpr
    where
        V: ArrayElement,
        I: IntoIterator<Item = V>,
    {
        self.binary(BinOper::Equal, Func::any(Value::array(v)))
    }

    /// Express a `<> ALL` non-membership test against one array parameter.
    ///
    /// The negation of [`eq_any`](ExprTrait::eq_any), spelled the way PostgreSQL itself
    /// spells it: its parser reads `x NOT IN (…)` as the `<>` operator over the
    /// list, never as a `NOT` wrapped around `x = ANY(…)`. The two are
    /// equivalent under three-valued logic — a NULL element makes both NULL,
    /// exactly as `NOT IN` does — so the choice is which shape to hand the
    /// planner, and `<> ALL` is one operator node where the wrapped form is a
    /// boolean node around a second one.
    ///
    /// An empty list is true for every operand, NULL included: nothing fails a
    /// test applied to nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Id])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col((Char::Table, Char::SizeW)).ne_all([1, 2, 3]))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "character"."size_w" <> ALL(ARRAY [1,2,3])"#
    /// );
    ///
    /// let (sql, values) = query.build();
    /// assert_eq!(
    ///     sql,
    ///     r#"SELECT "id" FROM "character" WHERE "character"."size_w" <> ALL($1)"#
    /// );
    /// assert_eq!(values.0.len(), 1);
    /// ```
    // [spec:pgorm:req:sql.ast.expr.eq-any+2]
    fn ne_all<V, I>(self, v: I) -> SimpleExpr
    where
        V: ArrayElement,
        I: IntoIterator<Item = V>,
    {
        self.binary(BinOper::NotEqual, Func::all(Value::array(v)))
    }

    /// Express a `IN` sub expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::FontId])
    ///     .from(Char::Table)
    ///     .and_where(
    ///         Expr::tuple([
    ///             Expr::col(Char::Character).into(),
    ///             Expr::col(Char::FontId).into(),
    ///         ])
    ///         .in_tuples([(1, String::from("1")), (2, String::from("2"))])
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "font_id" FROM "character" WHERE ("character", "font_id") IN ((1, '1'), (2, '2'))"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.expr.in+1]
    fn in_tuples<V, I>(self, v: I) -> SimpleExpr
    where
        V: IntoValueTuple,
        I: IntoIterator<Item = V>,
    {
        let rows = v
            .into_iter()
            .map(|row| SimpleExpr::Values(row.into_value_tuple().into_iter().collect()));
        self.binary(BinOper::In, SimpleExpr::Tuple(rows.collect()))
    }

    /// Express a `IN` sub-query expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::SizeW).in_subquery(
    ///         Query::select()
    ///             .expr(Expr::raw("3 + 2 * 2"))
    ///             .take()
    ///     ))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "size_w" IN (SELECT 3 + 2 * 2)"#
    /// );
    /// ```
    fn in_subquery(self, sel: SelectStatement) -> SimpleExpr {
        self.binary(
            BinOper::In,
            SimpleExpr::SubQuery(None, Box::new(sel.into_sub_query_statement())),
        )
    }

    /// Express a `NOT IN` sub-query expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .columns([Char::Character, Char::SizeW, Char::SizeH])
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::SizeW).not_in_subquery(
    ///         Query::select()
    ///             .expr(Expr::raw("3 + 2 * 2"))
    ///             .take()
    ///     ))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "size_w" NOT IN (SELECT 3 + 2 * 2)"#
    /// );
    /// ```
    fn not_in_subquery(self, sel: SelectStatement) -> SimpleExpr {
        self.binary(
            BinOper::NotIn,
            SimpleExpr::SubQuery(None, Box::new(sel.into_sub_query_statement())),
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
    fn as_enum<T>(self, type_name: T) -> SimpleExpr
    where
        T: IntoName,
    {
        self.cast_as_type(TypeName::new(type_name))
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
    ///
    /// On an expression already built:
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
    fn cast_as<T>(self, type_name: T) -> SimpleExpr
    where
        T: IntoName,
    {
        self.cast_as_type(TypeName::new(type_name))
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
    fn cast_as_raw(self, type_expr: &'static str) -> SimpleExpr {
        self.cast_as_type(TypeName::raw(type_expr))
    }

    /// Cast to a structured [`TypeName`] — schema qualification and array
    /// suffix included. Every cast spelling funnels here, and the node it
    /// builds is the only shape a cast has.
    // [spec:pgorm:req:sql.ast.cast-shape+2]
    fn cast_as_type(self, type_name: TypeName) -> SimpleExpr {
        SimpleExpr::AsEnum(Box::new(type_name), Box::new(self.into()))
    }

    /// Cast to a range or multirange type a schema created, named in full: a
    /// range value is written as its text form first, because PostgreSQL has
    /// no cast between two range types (`42846`) and the text form is what
    /// every range type reads.
    ///
    /// A [`Value::Range`] or [`Value::Multirange`] operand becomes the text
    /// `[1,5)` (`NULL` staying `NULL`), so a built-in `int4range` value
    /// lands in a range type created over `int4`; any other operand — the
    /// text a [`Range`](crate::Range)'s `Display` writes, a column — is cast
    /// as it is. A text operand is a bound parameter pinned to `text`, or an
    /// escaped string literal inline, so the cast reads it through the range
    /// type's input function and the two renderings agree.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let slot = TypeName::new(Name::runtime("slot")).schema(Name::runtime("booking"));
    /// let query = Query::insert()
    ///     .into_table(Char::Table)
    ///     .columns([Char::Character])
    ///     .values_panic([Expr::val(Range::from(1..5)).as_range(slot)])
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"INSERT INTO "character" ("character") VALUES (CAST('[1,5)' AS booking.slot))"#
    /// );
    /// assert_eq!(
    ///     query.build().0,
    ///     r#"INSERT INTO "character" ("character") VALUES (CAST($1::text AS booking.slot))"#
    /// );
    /// ```
    // [spec:pgorm:def:sql.value.created-range+9]
    // [spec:pgorm:req:sql.ast.cast-shape+2]
    fn as_range(self, type_name: TypeName) -> SimpleExpr {
        let operand = match self.into() {
            SimpleExpr::Value(value) => SimpleExpr::Value(range_as_text(value)),
            operand => operand,
        };
        SimpleExpr::AsEnum(Box::new(type_name), Box::new(operand))
    }

    /// Express a postgres JSON key existence test (`?`).
    ///
    /// True when the operand is an object holding `key`, an array holding it as
    /// a string element, or the string itself — one operator over the three
    /// readings of "contains this string at the top level".
    ///
    /// The key is text and nothing else: `jsonb ? integer` is not an operator
    /// PostgreSQL defines, so the bound is `Into<String>` rather than
    /// `Into<SimpleExpr>` and a numeric key fails to compile instead of failing
    /// on the server. The family is `jsonb`-only — `json` has no `?` — so a
    /// `json` column needs a cast, which [`cast_as`](ExprTrait::cast_as) supplies.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::UserData).has_json_key("a"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "user_data" ? 'a'"#
    /// );
    ///
    /// let (sql, values) = query.build();
    /// assert_eq!(sql, r#"SELECT "id" FROM "character" WHERE "user_data" ? $1"#);
    /// ```
    // [spec:pgorm:req:sql.ast.expr.json+3]
    fn has_json_key<S>(self, key: S) -> SimpleExpr
    where
        S: Into<String>,
    {
        let key: String = key.into();
        self.binary(BinOper::HasJsonKey, key)
    }

    /// Express a postgres JSON path existence test (`@?`): whether the
    /// `jsonpath` returns any item for the `jsonb` operand.
    ///
    /// The path is a value, bound as `text` and cast to `jsonpath` as
    /// [`Func::json_exists`] binds its path, so it never becomes part of the
    /// statement's text, and the driver, which writes a value in the format
    /// of the type it is, never has to write a `jsonpath`. The operator is not
    /// a spelling of `JSON_EXISTS`: a GIN index on the column serves `@?` and
    /// serves no `JSON_EXISTS`, and where a strict-mode path raises an error
    /// the operator answers NULL.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::UserData).json_path_exists(r#"$.tags[*] ? (@ == "blue")"#))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.build().0,
    ///     r#"SELECT "id" FROM "character" WHERE "user_data" @? CAST($1::text AS jsonpath)"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.expr.json+3]
    fn json_path_exists<P>(self, path: P) -> SimpleExpr
    where
        P: Into<String>,
    {
        let path =
            SimpleExpr::from(path.into()).cast_as_type(TypeName::new(Name::runtime("jsonpath")));
        self.binary(BinOper::JsonPathExists, path)
    }

    /// Mark this expression as JSON text for an SQL/JSON position:
    /// `FORMAT JSON`.
    ///
    /// A `text` value then embeds as the JSON it spells, and a `bytea` value
    /// is read as UTF-8 JSON, which unmarked it cannot be.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::json_object().entry("doc", Expr::col(Char::Character).format_json()))
    ///         .from(Char::Table)
    ///         .to_string(),
    ///     r#"SELECT JSON_OBJECT('doc'::text : "character" FORMAT JSON) FROM "character""#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    fn format_json(self) -> JsonInput {
        JsonInput {
            expr: self.into(),
            format_json: true,
        }
    }

    /// Express the SQL/JSON predicate `IS JSON`: whether the operand — `text`,
    /// `json`, `jsonb` or `bytea` — is JSON of the tested kind.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::Character).is_json(JsonKind::Object.with_unique_keys()))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE ("character" IS JSON OBJECT WITH UNIQUE KEYS)"#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    fn is_json<T>(self, test: T) -> SimpleExpr
    where
        T: Into<JsonTest>,
    {
        SqlJson::Is {
            operand: self.into(),
            test: test.into(),
            negated: false,
        }
        .into()
    }

    /// Express `IS NOT JSON`, the negation of [`is_json`](ExprTrait::is_json): like it, `NULL`
    /// for a `NULL` operand.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .column(Char::Id)
    ///         .from(Char::Table)
    ///         .and_where(Expr::col(Char::Character).is_not_json(JsonKind::Value))
    ///         .to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE ("character" IS NOT JSON)"#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.expr.sql-json+2]
    fn is_not_json<T>(self, test: T) -> SimpleExpr
    where
        T: Into<JsonTest>,
    {
        SqlJson::Is {
            operand: self.into(),
            test: test.into(),
            negated: true,
        }
        .into()
    }

    /// Put this expression under the named collation: `(expr COLLATE "name")`.
    ///
    /// A collation decides how text compares and sorts, so the clause changes
    /// what `=`, `<` and `ORDER BY` answer about the same values: under `"C"`
    /// text orders by byte, every upper-case letter before every lower-case
    /// one, where a linguistic collation interleaves them. The right-hand side
    /// is a collation's *name*, quoted like every other identifier — see
    /// [`Collation`] for the qualified form — and never an expression, so
    /// `a COLLATE b` for an arbitrary `b` has no construction.
    ///
    /// `ORDER BY` has no collation slot of its own: PostgreSQL's grammar reads
    /// the clause as part of the sort key's expression, so an ordering takes
    /// the collated expression through
    /// [`order_by_expr`](crate::OrderedStatement::order_by_expr).
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .column(Glyph::Image)
    ///     .from(Glyph::Table)
    ///     .and_where(Expr::col(Glyph::Image).collate(Name::runtime("C")).lt("b"))
    ///     .order_by_expr(
    ///         Expr::col(Glyph::Image).collate(Name::runtime("C")).into(),
    ///         Order::Desc,
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"SELECT "image" FROM "glyph""#,
    ///         r#"WHERE ("image" COLLATE "C") < 'b'"#,
    ///         r#"ORDER BY ("image" COLLATE "C") DESC"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.expr.collate+2]
    fn collate<C>(self, collation: C) -> SimpleExpr
    where
        C: IntoCollation,
    {
        SimpleExpr::Collate(Box::new(self.into()), Box::new(collation.into_collation()))
    }

    /// Subscript one element: `a[index]`.
    ///
    /// PostgreSQL arrays count from 1, and an index outside the array's
    /// bounds yields NULL rather than an error. Chained indexes are one
    /// multi-dimensional access — `.index(1).index(2)` is `a[1][2]`, the
    /// element at row 1, column 2 of a two-dimensional array. A bound index
    /// is a placeholder the server types as `int4`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col(Glyph::Tokens).index(1))
    ///     .from(Glyph::Table)
    ///     .and_where(Expr::col(Glyph::Tokens).index(2).gt(0))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "tokens"[1] FROM "glyph" WHERE "tokens"[2] > 0"#
    /// );
    /// ```
    fn index<T>(self, index: T) -> SimpleExpr
    where
        T: Into<SimpleExpr>,
    {
        self.subscript(Subscript::Index(index.into()))
    }

    /// Subscript a slice between two bounds, both inclusive: `a[lower:upper]`.
    ///
    /// A slice is an array, and a slice reaching past the array's bounds is
    /// cut to them — an empty array when none of it overlaps — rather than
    /// NULL. PostgreSQL treats *every* subscript in a chain as a slice once
    /// any of them is one, reading a plain index `i` beside a slice as `1:i`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col(Glyph::Tokens).slice(2, 3))
    ///     .from(Glyph::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "tokens"[2:3] FROM "glyph""#
    /// );
    /// ```
    fn slice<L, U>(self, lower: L, upper: U) -> SimpleExpr
    where
        L: Into<SimpleExpr>,
        U: Into<SimpleExpr>,
    {
        self.subscript(Subscript::Slice(Some(lower.into()), Some(upper.into())))
    }

    /// Subscript a slice from `lower` to the array's end: `a[lower:]`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col(Glyph::Tokens).slice_from(2))
    ///     .from(Glyph::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(query.to_string(), r#"SELECT "tokens"[2:] FROM "glyph""#);
    /// ```
    fn slice_from<L>(self, lower: L) -> SimpleExpr
    where
        L: Into<SimpleExpr>,
    {
        self.subscript(Subscript::Slice(Some(lower.into()), None))
    }

    /// Subscript a slice from the array's start to `upper`: `a[:upper]`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col(Glyph::Tokens).slice_to(2))
    ///     .from(Glyph::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(query.to_string(), r#"SELECT "tokens"[:2] FROM "glyph""#);
    /// ```
    fn slice_to<U>(self, upper: U) -> SimpleExpr
    where
        U: Into<SimpleExpr>,
    {
        self.subscript(Subscript::Slice(None, Some(upper.into())))
    }

    /// Apply any [`Subscript`], including the slice with neither bound,
    /// `a[:]`, which the four shorthands do not spell.
    ///
    /// A column reference or an already-subscripted expression takes the
    /// subscript directly; anything else — a function call, a cast, a bound
    /// value — is parenthesised first, because PostgreSQL's grammar admits a
    /// subscript only after a column, a parameter or a parenthesised
    /// expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .expr(Expr::col(Glyph::Tokens).subscript(Subscript::Slice(None, None)))
    ///     .expr(Expr::expr(Func::named(Name::runtime("string_to_array"))
    ///         .arg(Expr::col(Glyph::Image))
    ///         .arg(",")).index(1))
    ///     .from(Glyph::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "tokens"[:], (string_to_array("image", ','))[1] FROM "glyph""#
    /// );
    /// ```
    fn subscript(self, subscript: Subscript) -> SimpleExpr {
        SimpleExpr::Subscript(Box::new(self.into()), Box::new(subscript))
    }
}

/// What goes between the brackets of an array subscript.
///
/// PostgreSQL arrays count from 1. An [`Index`](Self::Index) selects one
/// element and answers NULL when it is out of range; a
/// [`Slice`](Self::Slice) selects the elements between its bounds, both
/// inclusive, and answers an array — cut to the array's bounds, and empty
/// rather than NULL when nothing overlaps. An omitted slice bound runs to
/// that end of the array, so `Slice(None, None)` is `[:]`, the whole of that
/// dimension.
// [spec:pgorm:req:sql.ast.expr.subscript+2]
#[derive(Debug, Clone, PartialEq)]
pub enum Subscript {
    /// `[index]`
    Index(SimpleExpr),
    /// `[lower:upper]`, either bound omissible
    Slice(Option<SimpleExpr>, Option<SimpleExpr>),
}

/// `value` as the text a range type a schema created reads: a range or a
/// multirange as its text form, an array of them as the array of their
/// texts, which casts to the type's array as one text casts to the type,
/// and anything else as it is.
// [spec:pgorm:def:sql.value.created-range+9]
fn range_as_text(value: Value) -> Value {
    match value {
        Value::Range(_, range) => Value::String(range.map(|range| Box::new(range_text(&range)))),
        Value::Multirange(_, multirange) => {
            Value::String(multirange.map(|multirange| Box::new(multirange_text(&multirange))))
        }
        Value::Array(ArrayType::Range(_) | ArrayType::Multirange(_), items) => Value::Array(
            ArrayType::String,
            items.map(|items| Box::new(items.into_iter().map(range_as_text).collect())),
        ),
        value => value,
    }
}

impl ExprTrait for Expr {}
impl ExprTrait for SimpleExpr {}
impl ExprTrait for FunctionCall {}
impl ExprTrait for CaseStatement {}
impl ExprTrait for SimpleCaseStatement {}
impl ExprTrait for Grouping {}
impl ExprTrait for SqlJson {}
impl ExprTrait for JsonExists {}
impl ExprTrait for JsonValue {}
impl ExprTrait for JsonQuery {}
impl ExprTrait for JsonObject {}
impl ExprTrait for JsonArray {}
impl ExprTrait for JsonArrayQuery {}
impl ExprTrait for JsonObjectAgg {}
impl ExprTrait for JsonArrayAgg {}
impl ExprTrait for JsonParse {}
impl ExprTrait for JsonSerialize {}
