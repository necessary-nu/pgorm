use crate::{Condition, Expr, IntoCondition, SimpleExpr};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CaseStatementCondition {
    pub(crate) condition: Condition,
    pub(crate) result: SimpleExpr,
}

/// One arm of a [`SimpleCaseStatement`]: the value the operand is compared
/// with, and the result that comparison selects.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SimpleCaseArm {
    pub(crate) value: SimpleExpr,
    pub(crate) result: SimpleExpr,
}

/// The searched form of `CASE`: `CASE WHEN <condition> THEN <result> … END`.
///
/// Each arm holds a condition. The *simple* form, which compares one operand
/// against each arm's value, is [`SimpleCaseStatement`] — a separate type,
/// because its arms hold values instead and the two shapes do not mix.
///
/// [`Expr::case`] is the only way to begin one, and it takes the first arm:
/// PostgreSQL's grammar requires at least one `WHEN`, so a searched CASE with
/// none has no constructor, just as the simple form's [`CaseOperand`] converts
/// into nothing before its first arm.
///
/// ```compile_fail,E0599
/// use pgorm_query::*;
///
/// let armless = CaseStatement::new().finally("x");
/// ```
///
/// ```compile_fail,E0599
/// use pgorm_query::*;
///
/// let armless = CaseStatement::default();
/// ```
// [spec:pgorm:def:sql.ast.case+3]
// [spec:pgorm:def:sql.ast.case+3/test]    the two `compile_fail,E0599` examples above:
// neither an armless `new()` nor a `Default` exists to build a CASE with no WHEN
#[derive(Debug, Clone, PartialEq)]
pub struct CaseStatement {
    pub(crate) when: Vec<CaseStatementCondition>,
    pub(crate) r#else: Option<SimpleExpr>,
}

impl CaseStatement {
    /// Adds new `CASE WHEN` to existing case statement.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .expr_as(
    ///             Expr::case(
    ///                 Expr::col((Glyph::Table, Glyph::Aspect)).gt(0),
    ///                 "positive"
    ///              )
    ///             .case(
    ///                 Expr::col((Glyph::Table, Glyph::Aspect)).lt(0),
    ///                 "negative"
    ///              )
    ///             .finally("zero"),
    ///          Name::runtime("polarity")
    ///     )
    ///     .from(Glyph::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT (CASE WHEN ("glyph"."aspect" > 0) THEN 'positive' WHEN ("glyph"."aspect" < 0) THEN 'negative' ELSE 'zero' END) AS "polarity" FROM "glyph""#
    /// );    
    /// ```
    pub fn case<C, T>(mut self, cond: C, then: T) -> Self
    where
        C: IntoCondition,
        T: Into<SimpleExpr>,
    {
        self.when.push(CaseStatementCondition {
            condition: cond.into_condition(),
            result: then.into(),
        });
        self
    }

    /// Ends the case statement with the final `ELSE` result.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .expr_as(
    ///         Expr::case(
    ///             Condition::any()
    ///                 .add(Expr::col((Character::Table, Character::FontSize)).gt(48))
    ///                 .add(Expr::col((Character::Table, Character::SizeW)).gt(500)),
    ///             "large"
    ///         )
    ///         .case(
    ///             Condition::any()
    ///                 .add(Expr::col((Character::Table, Character::FontSize)).between(24,48))
    ///                 .add(Expr::col((Character::Table, Character::SizeW)).between(300,500)),
    ///             "medium"
    ///         )
    ///         .finally("small"),
    ///         Name::runtime("char_size"))
    ///     .from(Character::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"SELECT"#,
    ///         r#"(CASE WHEN ("character"."font_size" > 48 OR "character"."size_w" > 500) THEN 'large'"#,
    ///         r#"WHEN ("character"."font_size" BETWEEN 24 AND 48 OR "character"."size_w" BETWEEN 300 AND 500) THEN 'medium'"#,
    ///         r#"ELSE 'small' END) AS "char_size""#,
    ///         r#"FROM "character""#
    ///     ]
    ///     .join(" ")
    /// );    
    /// ```
    pub fn finally<E>(mut self, r#else: E) -> Self
    where
        E: Into<SimpleExpr>,
    {
        self.r#else = Some(r#else.into());
        self
    }

    /// The arms in order, each as its condition and its result. A condition
    /// is given as the expression it means, a `Condition` tree joined by its
    /// `AND`s and `OR`s and negated where it is, so one built from a single
    /// expression is that expression.
    // [spec:pgorm:def:sql.ast.case+3]
    pub fn get_arms(&self) -> impl Iterator<Item = (SimpleExpr, &SimpleExpr)> {
        self.when
            .iter()
            .map(|arm| (arm.condition.to_simple_expr(), &arm.result))
    }

    /// The `ELSE` result, if [`finally`](Self::finally) set one.
    // [spec:pgorm:def:sql.ast.case+3]
    pub fn get_else(&self) -> Option<&SimpleExpr> {
        self.r#else.as_ref()
    }
}

#[allow(clippy::from_over_into)]
impl Into<SimpleExpr> for CaseStatement {
    fn into(self) -> SimpleExpr {
        SimpleExpr::Case(Box::new(self))
    }
}

/// A simple-form `CASE` holding its operand and no arm yet, as returned by
/// [`Expr::case_of`].
///
/// It has one method, [`when`](Self::when), which adds the first arm and
/// yields a [`SimpleCaseStatement`]. It does not convert into an expression:
/// PostgreSQL's grammar requires at least one `WHEN`, so a CASE with none has
/// no expression to become.
// [spec:pgorm:def:sql.ast.case+3]
#[derive(Debug, Clone, PartialEq)]
pub struct CaseOperand {
    operand: SimpleExpr,
}

impl CaseOperand {
    /// Adds the first `WHEN <value> THEN <result>` arm.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .expr_as(
    ///         Expr::case_of(Expr::col(Glyph::Aspect)).when(1, "one"),
    ///         Name::runtime("name"),
    ///     )
    ///     .from(Glyph::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT (CASE "aspect" WHEN 1 THEN 'one' END) AS "name" FROM "glyph""#
    /// );
    /// ```
    pub fn when<V, T>(self, value: V, then: T) -> SimpleCaseStatement
    where
        V: Into<SimpleExpr>,
        T: Into<SimpleExpr>,
    {
        SimpleCaseStatement {
            operand: self.operand,
            when: vec![SimpleCaseArm {
                value: value.into(),
                result: then.into(),
            }],
            r#else: None,
        }
    }
}

/// The simple form of `CASE`: `CASE <operand> WHEN <value> THEN <result> …
/// END`, which compares one operand against each arm's value in turn.
///
/// Built from [`Expr::case_of`] and its first [`when`](CaseOperand::when), so
/// it always holds at least one arm. The comparison is `=`, which makes a NULL
/// operand match no arm — not even `WHEN NULL` — and fall through to the
/// `ELSE`, where the searched form's `WHEN x IS NULL` would match. The
/// searched form, whose arms hold conditions, is [`CaseStatement`].
// [spec:pgorm:def:sql.ast.case+3]
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleCaseStatement {
    pub(crate) operand: SimpleExpr,
    pub(crate) when: Vec<SimpleCaseArm>,
    pub(crate) r#else: Option<SimpleExpr>,
}

impl SimpleCaseStatement {
    /// Adds a further `WHEN <value> THEN <result>` arm. Arms are tried in the
    /// order they were added, and the first whose value equals the operand
    /// wins.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .expr_as(
    ///         Expr::case_of(Expr::col(Glyph::Aspect))
    ///             .when(1, "one")
    ///             .when(2, "two")
    ///             .finally("many"),
    ///         Name::runtime("name"),
    ///     )
    ///     .from(Glyph::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT (CASE "aspect" WHEN 1 THEN 'one' WHEN 2 THEN 'two' ELSE 'many' END) AS "name" FROM "glyph""#
    /// );
    /// ```
    pub fn when<V, T>(mut self, value: V, then: T) -> Self
    where
        V: Into<SimpleExpr>,
        T: Into<SimpleExpr>,
    {
        self.when.push(SimpleCaseArm {
            value: value.into(),
            result: then.into(),
        });
        self
    }

    /// Ends the case expression with the `ELSE` result, which is what an
    /// operand equal to no arm's value — a NULL operand included — yields.
    /// Without one, such an operand yields NULL.
    pub fn finally<E>(mut self, r#else: E) -> Self
    where
        E: Into<SimpleExpr>,
    {
        self.r#else = Some(r#else.into());
        self
    }
}

impl From<SimpleCaseStatement> for SimpleExpr {
    fn from(case: SimpleCaseStatement) -> Self {
        SimpleExpr::SimpleCase(Box::new(case))
    }
}

// `Expr`'s own file is at its function cap, so the entry points to both
// forms sit here, beside the types they lead to.
impl Expr {
    /// Starts a searched `CASE` with its first `WHEN <condition> THEN
    /// <result>` arm: `CASE WHEN … END`.
    ///
    /// This is the only constructor of a [`CaseStatement`], so a searched CASE
    /// always holds an arm; [`case`](CaseStatement::case) appends the rest.
    /// For arms that each compare one operand with a value, use
    /// [`Expr::case_of`].
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .expr_as(
    ///         Expr::case(
    ///                 Expr::col((Glyph::Table, Glyph::Aspect)).is_in([2, 4]),
    ///                 true
    ///              )
    ///             .finally(false),
    ///          Name::runtime("is_even")
    ///     )
    ///     .from(Glyph::Table)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT (CASE WHEN ("glyph"."aspect" IN (2, 4)) THEN TRUE ELSE FALSE END) AS "is_even" FROM "glyph""#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.case+3]
    pub fn case<C, T>(cond: C, then: T) -> CaseStatement
    where
        C: IntoCondition,
        T: Into<SimpleExpr>,
    {
        CaseStatement {
            when: vec![CaseStatementCondition {
                condition: cond.into_condition(),
                result: then.into(),
            }],
            r#else: None,
        }
    }

    /// Starts a simple-form `CASE` over `operand`: `CASE <operand> WHEN …`.
    ///
    /// Each [`when`](CaseOperand::when) arm compares the operand with a value
    /// by `=`, so a NULL operand matches no arm and takes the `ELSE`. For arms
    /// that each test their own condition, use [`Expr::case`].
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .column(Glyph::Id)
    ///     .from(Glyph::Table)
    ///     .and_where(
    ///         Expr::expr(
    ///             Expr::case_of(Expr::col(Glyph::Image))
    ///                 .when("a", 1)
    ///                 .when("b", 2)
    ///                 .finally(0),
    ///         )
    ///         .gt(0),
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "id" FROM "glyph" WHERE (CASE "image" WHEN 'a' THEN 1 WHEN 'b' THEN 2 ELSE 0 END) > 0"#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.case+3]
    pub fn case_of<T>(operand: T) -> CaseOperand
    where
        T: Into<SimpleExpr>,
    {
        CaseOperand {
            operand: operand.into(),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::*;

    #[test]
    fn test_where_case_eq() {
        let case_statement: SimpleExpr = Expr::case(
            Expr::col(Name::runtime("col")).lt(5),
            Expr::col(Name::runtime("othercol")),
        )
        .finally(Expr::col(Name::runtime("finalcol")))
        .into();

        let result = Query::select()
            .column(Asterisk)
            .from(Name::runtime("tbl"))
            .and_where(case_statement.eq(10))
            .to_string();
        assert_eq!(
            result,
            r#"SELECT * FROM "tbl" WHERE (CASE WHEN ("col" < 5) THEN "othercol" ELSE "finalcol" END) = 10"#
        );
    }
}
