//! For calling built-in SQL functions.

use crate::{Condition, IntoCondition, expr::*, types::*};

#[path = "func_json.rs"]
mod json;
#[path = "func_set.rs"]
mod set;
#[path = "func_uuid.rs"]
mod uuid;

/// Functions
///
/// A cast is not one of them: `CAST` is [`SimpleExpr::AsEnum`], so matching a
/// `FunctionCall` never has to account for a cast.
// [spec:pgorm:def:sql.ast.func+12]
// [spec:pgorm:req:sql.ast.cast-shape+2]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Function {
    Max,
    Min,
    Sum,
    Avg,
    PercentileCont,
    PercentileDisc,
    Abs,
    Count,
    IfNull,
    CharLength,
    /// A function this enum has no variant for, called by quoted name.
    Named(Name),
    Coalesce,
    Lower,
    Upper,
    BitAnd,
    BitOr,
    Random,
    Round,
    ToTsquery,
    ToTsvector,
    PhrasetoTsquery,
    PlaintoTsquery,
    WebsearchToTsquery,
    TsRank,
    TsRankCd,
    StartsWith,
    GenRandomUUID,
    UuidV4,
    UuidV7,
    UuidExtractTimestamp,
    UuidExtractVersion,
    /// `GENERATE_SERIES`, a set-returning function.
    GenerateSeries,
    /// `UNNEST`, a set-returning function.
    Unnest,
    /// `JSON_TO_RECORD`, a function returning `record`.
    JsonToRecord,
    /// `JSONB_TO_RECORD`, a function returning `record`.
    JsonbToRecord,
    /// `JSON_TO_RECORDSET`, a set-returning function of `record`s.
    JsonToRecordset,
    /// `JSONB_TO_RECORDSET`, a set-returning function of `record`s.
    JsonbToRecordset,
    Any,
    Some,
    All,
}

/// Function call.
// [spec:pgorm:def:sql.ast.func+12]
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionCall {
    pub(crate) func: Function,
    pub(crate) args: Vec<SimpleExpr>,
    pub(crate) mods: Vec<FuncArgMod>,
    /// The aggregate modifier clauses, boxed and absent until one is set: a
    /// `FunctionCall` is held inline by `SimpleExpr`, which several AST nodes
    /// hold inline in turn, so a scalar call paying for clauses it will never
    /// carry is a cost the whole expression tree pays.
    pub(crate) aggregate: Option<Box<AggregateMods>>,
}

/// The two clauses PostgreSQL admits after an aggregate's argument list.
// [spec:pgorm:def:sql.ast.func+12]
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct AggregateMods {
    pub(crate) within_group: Vec<OrderExpr>,
    pub(crate) filter: Option<Condition>,
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct FuncArgMod {
    pub distinct: bool,
}

impl FunctionCall {
    pub(crate) fn new(func: Function) -> Self {
        Self {
            func,
            args: Vec::new(),
            mods: Vec::new(),
            aggregate: None,
        }
    }

    /// Whether the call is its function and arguments alone: no argument
    /// modifier (`DISTINCT`) and no aggregate clause (`WITHIN GROUP`,
    /// `FILTER`), so a reader holding the two has read the whole call.
    // [spec:pgorm:def:sql.ast.func+12]
    pub fn is_plain(&self) -> bool {
        self.aggregate.is_none() && self.mods.iter().all(|m| !m.distinct)
    }

    /// Append an argument to the function call
    pub fn arg<T>(self, arg: T) -> Self
    where
        T: Into<SimpleExpr>,
    {
        self.arg_with(arg, Default::default())
    }

    pub(crate) fn arg_with<T>(mut self, arg: T, mod_: FuncArgMod) -> Self
    where
        T: Into<SimpleExpr>,
    {
        self.args.push(arg.into());
        self.mods.push(mod_);
        self
    }

    /// Replace the arguments of the function call
    pub fn args<I>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = SimpleExpr>,
    {
        self.args = args.into_iter().collect();
        self.mods = vec![Default::default(); self.args.len()];
        self
    }

    /// Restrict an aggregate to the rows satisfying `condition`, rendering
    /// `FILTER (WHERE ..)` after the argument list.
    ///
    /// This is the standard spelling of conditional aggregation, and says
    /// what `SUM(CASE WHEN c THEN x END)` only implies: rows failing the
    /// condition are not fed to the aggregate at all, rather than being fed
    /// to it as nulls. The two agree for `SUM` and `AVG`; they disagree for
    /// `COUNT`, where the CASE form's `ELSE NULL` is what keeps the count
    /// honest and an `ELSE 0` quietly would not.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::count(Expr::col(Character::Id)).filter(Expr::col(Character::FontSize).gt(12)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT COUNT("id") FILTER (WHERE "font_size" > 12) FROM "character""#
    /// );
    /// ```
    ///
    /// Each call replaces the previous filter; a conjunction is written as one
    /// [`Condition`].
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::sum(Expr::col(Character::SizeW)).filter(
    ///             Condition::all()
    ///                 .add(Expr::col(Character::FontSize).gt(12))
    ///                 .add(Expr::col(Character::Ascii).eq(true)),
    ///         ))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT SUM("size_w") FILTER (WHERE "font_size" > 12 AND "ascii" = TRUE) FROM "character""#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.func+12]
    pub fn filter<C>(mut self, condition: C) -> Self
    where
        C: IntoCondition,
    {
        self.aggregate.get_or_insert_default().filter = Some(condition.into_condition());
        self
    }

    /// Supply the ordering an ordered-set aggregate is defined over,
    /// rendering `WITHIN GROUP (ORDER BY ..)` after the argument list.
    ///
    /// Ordered-set aggregates — `percentile_cont`, `percentile_disc`, `mode`,
    /// `rank` and friends — take their direct arguments in the parentheses
    /// and the column they rank against here.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::percentile_cont(0.5).within_group(Character::SizeW, Order::Asc))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY "size_w" ASC) FROM "character""#
    /// );
    /// ```
    ///
    /// Repeated calls accumulate, for the hypothetical-set aggregates that
    /// rank against several columns at once.
    // [spec:pgorm:def:sql.ast.func+12]
    pub fn within_group<T>(self, col: T, order: Order) -> Self
    where
        T: IntoColumnRef,
    {
        self.within_group_expr(SimpleExpr::Column(col.into_column_ref()), order)
    }

    /// [`within_group`][Self::within_group] over an arbitrary expression
    /// rather than a plain column.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(
    ///             Func::percentile_disc(0.9)
    ///                 .within_group_expr(Expr::col(Character::SizeW).mul(2), Order::Desc)
    ///         )
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT PERCENTILE_DISC(0.9) WITHIN GROUP (ORDER BY "size_w" * 2 DESC) FROM "character""#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.func+12]
    pub fn within_group_expr<T>(mut self, expr: T, order: Order) -> Self
    where
        T: Into<SimpleExpr>,
    {
        self.aggregate
            .get_or_insert_default()
            .within_group
            .push(OrderExpr {
                expr: expr.into(),
                order,
                nulls: None,
            });
        self
    }

    pub fn get_func(&self) -> &Function {
        &self.func
    }

    pub fn get_args(&self) -> &[SimpleExpr] {
        &self.args
    }

    /// The ordering supplied by [`within_group`][Self::within_group], empty
    /// when the call has none.
    pub fn get_within_group(&self) -> &[OrderExpr] {
        self.aggregate
            .as_ref()
            .map_or(&[][..], |mods| &mods.within_group)
    }

    /// The condition supplied by [`filter`][Self::filter], if any.
    pub fn get_filter(&self) -> Option<&Condition> {
        self.aggregate
            .as_ref()
            .and_then(|mods| mods.filter.as_ref())
    }

    pub fn get_mods(&self) -> &[FuncArgMod] {
        &self.mods
    }
}

/// Function call helper.
// [spec:pgorm:def:sql.ast.func+12]
#[derive(Debug, Clone)]
pub struct Func;

impl Func {
    /// Call a function this vocabulary has no spelling for, by name.
    ///
    /// The name is an identifier, not SQL text: it is spelled bare only when
    /// it is a lowercase name that is no keyword PostgreSQL restricts, and
    /// quoted otherwise. So a name carrying anything else — an upper-case
    /// letter, a space, a payload, a keyword such as `not` — reaches
    /// PostgreSQL as a function it can look up and fail to find, never as
    /// syntax it executes. The exceptions are `coalesce`, `greatest`,
    /// `least` and `nullif`, which are written bare because the grammar
    /// builds them itself and they are not functions to look up
    /// (`sql.types.type-name`).
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// struct MyFunction;
    ///
    /// impl SqlName for MyFunction {
    ///     fn unquoted(&self, s: &mut dyn std::fmt::Write) {
    ///         write!(s, "my_function").unwrap();
    ///     }
    /// }
    ///
    /// let query = Query::select()
    ///     .expr(Func::named(MyFunction).arg("hello"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT my_function('hello')"#
    /// );
    /// ```
    ///
    /// A name that cannot be spelled bare is quoted, case preserved:
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Func::named(Name::runtime("MyFunction")).arg("hello"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "MyFunction"('hello')"#
    /// );
    /// ```
    ///
    /// So is a keyword, which bare would be read as the grammar's own syntax
    /// — `not(1)` is a boolean NOT — except the four call forms:
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Func::named(Name::runtime("not")).arg(1))
    ///     .expr(Func::named(Name::runtime("coalesce")).arg(1).arg(2))
    ///     .to_owned();
    ///
    /// assert_eq!(query.to_string(), r#"SELECT "not"(1), coalesce(1, 2)"#);
    /// ```
    pub fn named<T>(func: T) -> FunctionCall
    where
        T: IntoName,
    {
        FunctionCall::new(Function::Named(func.into_name()))
    }

    /// Call `MAX` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::max(Expr::col(Character::Id)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT MAX("id") FROM "character""#
    /// );
    /// ```
    pub fn max<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Max).arg(expr)
    }

    /// Call `MIN` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::min(Expr::col(Character::Id)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT MIN("id") FROM "character""#
    /// );
    /// ```
    pub fn min<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Min).arg(expr)
    }

    /// Call `SUM` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::sum(Expr::col(Character::Id)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT SUM("id") FROM "character""#
    /// );
    /// ```
    pub fn sum<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Sum).arg(expr)
    }

    /// Call `AVG` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::avg(Expr::col(Character::Id)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT AVG("id") FROM "character""#
    /// );
    /// ```
    pub fn avg<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Avg).arg(expr)
    }

    /// Call `PERCENTILE_CONT`, the continuous percentile: the value at
    /// `fraction` through the ordering, interpolating between the two
    /// neighbouring rows when the fraction lands between them.
    ///
    /// It is an ordered-set aggregate, so it is incomplete until it is given
    /// an ordering by [`within_group`][FunctionCall::within_group].
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::percentile_cont(0.5).within_group(Character::SizeW, Order::Asc))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY "size_w" ASC) FROM "character""#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.func+12]
    pub fn percentile_cont<T>(fraction: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::PercentileCont).arg(fraction)
    }

    /// Call `PERCENTILE_DISC`, the discrete percentile: the first value in the
    /// ordering whose cumulative distribution reaches `fraction`. Unlike
    /// [`percentile_cont`][Self::percentile_cont] it never interpolates, so it
    /// always returns a value that is actually in the column.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::percentile_disc(0.5).within_group(Character::SizeW, Order::Asc))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT PERCENTILE_DISC(0.5) WITHIN GROUP (ORDER BY "size_w" ASC) FROM "character""#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.func+12]
    pub fn percentile_disc<T>(fraction: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::PercentileDisc).arg(fraction)
    }

    /// Call `ABS` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::abs(Expr::col(Character::Id)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT ABS("id") FROM "character""#
    /// );
    /// ```
    pub fn abs<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Abs).arg(expr)
    }

    /// Call `COUNT` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::count(Expr::col(Character::Id)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT COUNT("id") FROM "character""#
    /// );
    /// ```
    pub fn count<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Count).arg(expr)
    }

    /// Call `COUNT` function with the `DISTINCT` modifier.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::count_distinct(Expr::col(Character::Id)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT COUNT(DISTINCT "id") FROM "character""#
    /// );
    /// ```
    pub fn count_distinct<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Count).arg_with(expr, FuncArgMod { distinct: true })
    }

    /// Call `CHAR_LENGTH` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::char_length(Expr::col(Character::Character)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT CHAR_LENGTH("character") FROM "character""#
    /// );
    /// ```
    pub fn char_length<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::CharLength).arg(expr)
    }

    /// Call `IF NULL` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::if_null(
    ///             Expr::col(Character::Character),
    ///             Expr::val("default")
    ///         ))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT COALESCE("character", 'default') FROM "character""#
    /// );
    /// ```
    pub fn if_null<A, B>(a: A, b: B) -> FunctionCall
    where
        A: Into<SimpleExpr>,
        B: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::IfNull).args([a.into(), b.into()])
    }

    /// Call `COALESCE` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Func::coalesce([
    ///         Expr::col(Char::SizeW).into(),
    ///         Expr::col(Char::SizeH).into(),
    ///         Expr::val(12).into(),
    ///     ]))
    ///     .from(Char::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT COALESCE("size_w", "size_h", 12) FROM "character""#
    /// );
    /// ```
    pub fn coalesce<I>(args: I) -> FunctionCall
    where
        I: IntoIterator<Item = SimpleExpr>,
    {
        FunctionCall::new(Function::Coalesce).args(args)
    }

    /// Call `LOWER` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::lower(Expr::col(Character::Character)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT LOWER("character") FROM "character""#
    /// );
    /// ```
    pub fn lower<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Lower).arg(expr)
    }

    /// Call `UPPER` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::upper(Expr::col(Character::Character)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT UPPER("character") FROM "character""#
    /// );
    /// ```
    pub fn upper<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Upper).arg(expr)
    }

    /// Call the `BIT_AND` aggregate: the bitwise AND of the argument across
    /// the rows of the group.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::bit_and(Expr::col(Character::Id)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT BIT_AND("id") FROM "character""#
    /// );
    /// ```
    pub fn bit_and<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::BitAnd).arg(expr)
    }

    /// Call the `BIT_OR` aggregate: the bitwise OR of the argument across
    /// the rows of the group.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::bit_or(Expr::col(Character::Id)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT BIT_OR("id") FROM "character""#
    /// );
    /// ```
    pub fn bit_or<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::BitOr).arg(expr)
    }

    /// Call `ROUND` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::round(Expr::col(Character::Id)))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT ROUND("id") FROM "character""#
    /// );
    /// ```
    pub fn round<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Round).arg(expr)
    }

    /// Call `ROUND` function with the precision.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::round_with_precision(
    ///             Expr::col(Character::Id),
    ///             2
    ///         ))
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT ROUND("id", 2) FROM "character""#
    /// );
    /// ```
    pub fn round_with_precision<T, U>(expr: T, precision: U) -> FunctionCall
    where
        T: Into<SimpleExpr>,
        U: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Round).args([expr.into(), precision.into()])
    }

    /// Call `RANDOM` function.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::random())
    ///         .from(Character::Table)
    ///         .to_string(),
    ///     r#"SELECT RANDOM() FROM "character""#
    /// );
    /// ```
    pub fn random() -> FunctionCall {
        FunctionCall::new(Function::Random)
    }

    /// Call `TO_TSQUERY` function. Postgres only.
    ///
    /// The parameter `regconfig` represents the OID of the text search configuration.
    /// If the value is `None` the argument is omitted from the query, and hence the database default used.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Func::to_tsquery("a & b", None))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT TO_TSQUERY('a & b')"#
    /// );
    /// ```
    pub fn to_tsquery<T>(expr: T, regconfig: Option<u32>) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        match regconfig {
            Some(config) => {
                let config = SimpleExpr::Value(config.into());
                FunctionCall::new(Function::ToTsquery).args([config, expr.into()])
            }
            None => FunctionCall::new(Function::ToTsquery).arg(expr),
        }
    }

    /// Call `TO_TSVECTOR` function. Postgres only.
    ///
    /// The parameter `regconfig` represents the OID of the text search configuration.
    /// If the value is `None` the argument is omitted from the query, and hence the database default used.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Func::to_tsvector("a b", None))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT TO_TSVECTOR('a b')"#
    /// );
    /// ```
    pub fn to_tsvector<T>(expr: T, regconfig: Option<u32>) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        match regconfig {
            Some(config) => {
                let config = SimpleExpr::Value(config.into());
                FunctionCall::new(Function::ToTsvector).args([config, expr.into()])
            }
            None => FunctionCall::new(Function::ToTsvector).arg(expr),
        }
    }

    /// Call `PHRASE_TO_TSQUERY` function. Postgres only.
    ///
    /// The parameter `regconfig` represents the OID of the text search configuration.
    /// If the value is `None` the argument is omitted from the query, and hence the database default used.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Func::phraseto_tsquery("a b", None))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT PHRASETO_TSQUERY('a b')"#
    /// );
    /// ```
    pub fn phraseto_tsquery<T>(expr: T, regconfig: Option<u32>) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        match regconfig {
            Some(config) => {
                let config = SimpleExpr::Value(config.into());
                FunctionCall::new(Function::PhrasetoTsquery).args([config, expr.into()])
            }
            None => FunctionCall::new(Function::PhrasetoTsquery).arg(expr),
        }
    }

    /// Call `PLAIN_TO_TSQUERY` function. Postgres only.
    ///
    /// The parameter `regconfig` represents the OID of the text search configuration.
    /// If the value is `None` the argument is omitted from the query, and hence the database default used.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Func::plainto_tsquery("a b", None))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT PLAINTO_TSQUERY('a b')"#
    /// );
    /// ```
    pub fn plainto_tsquery<T>(expr: T, regconfig: Option<u32>) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        match regconfig {
            Some(config) => {
                let config = SimpleExpr::Value(config.into());
                FunctionCall::new(Function::PlaintoTsquery).args([config, expr.into()])
            }
            None => FunctionCall::new(Function::PlaintoTsquery).arg(expr),
        }
    }

    /// Call `WEBSEARCH_TO_TSQUERY` function. Postgres only.
    ///
    /// The parameter `regconfig` represents the OID of the text search configuration.
    /// If the value is `None` the argument is omitted from the query, and hence the database default used.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Func::websearch_to_tsquery("a b", None))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT WEBSEARCH_TO_TSQUERY('a b')"#
    /// );
    /// ```
    pub fn websearch_to_tsquery<T>(expr: T, regconfig: Option<u32>) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        match regconfig {
            Some(config) => {
                let config = SimpleExpr::Value(config.into());
                FunctionCall::new(Function::WebsearchToTsquery).args([config, expr.into()])
            }
            None => FunctionCall::new(Function::WebsearchToTsquery).arg(expr),
        }
    }

    /// Call `TS_RANK` function. Postgres only.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Func::ts_rank("a b", "a&b"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT TS_RANK('a b', 'a&b')"#
    /// );
    /// ```
    pub fn ts_rank<T>(vector: T, query: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::TsRank).args([vector.into(), query.into()])
    }

    /// Call `TS_RANK_CD` function. Postgres only.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Func::ts_rank_cd("a b", "a&b"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT TS_RANK_CD('a b', 'a&b')"#
    /// );
    /// ```
    pub fn ts_rank_cd<T>(vector: T, query: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::TsRankCd).args([vector.into(), query.into()])
    }

    /// `ANY` over an array. Postgres only.
    ///
    /// A quantifier, not a function: it is SQL only as the right operand of an
    /// operator PostgreSQL reads one after — a comparison, LIKE and ILIKE and
    /// their negations, the regular-expression matches, containment and
    /// overlap, the range positions, `@@` and `@?` — and PostgreSQL rejects it
    /// anywhere else. Reach for [`ExprTrait::eq_any`](crate::ExprTrait::eq_any) for the equality case, which is
    /// the one worth a name; this is the escape hatch for the other operators.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::SizeW).gt(Func::any(vec![0, 1])))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "size_w" > ANY(ARRAY [0,1])"#
    /// );
    /// ```
    pub fn any<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Any).arg(expr)
    }

    /// `SOME` over an array, PostgreSQL's synonym for [`Func::any`]. Postgres
    /// only, and likewise valid only after a comparison operator.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::SizeW).gt(Func::some(vec![0, 1])))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "size_w" > SOME(ARRAY [0,1])"#
    /// );
    /// ```
    pub fn some<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Some).arg(expr)
    }

    /// `ALL` over an array. Postgres only, and likewise valid only after an
    /// operator PostgreSQL reads a quantifier after, as [`Func::any`] lists.
    /// Reach for [`ExprTrait::ne_all`](crate::ExprTrait::ne_all) for the inequality case.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .column(Char::Id)
    ///     .from(Char::Table)
    ///     .and_where(Expr::col(Char::SizeW).gt(Func::all(vec![0, 1])))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "character" WHERE "size_w" > ALL(ARRAY [0,1])"#
    /// );
    /// ```
    pub fn all<T>(expr: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::All).arg(expr)
    }

    /// Call `STARTS_WITH` function. Postgres only.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .expr(Func::starts_with("123", "1"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT STARTS_WITH('123', '1')"#
    /// );
    /// ```
    pub fn starts_with<T, P>(text: T, prefix: P) -> FunctionCall
    where
        T: Into<SimpleExpr>,
        P: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::StartsWith).args([text.into(), prefix.into()])
    }

    /// Call `GEN_RANDOM_UUID` function. Postgres only.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select().expr(Func::gen_random_uuid()).to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT GEN_RANDOM_UUID()"#
    /// );
    /// ```
    pub fn gen_random_uuid() -> FunctionCall {
        FunctionCall::new(Function::GenRandomUUID)
    }
}
