//! A domain's `CHECK` constraint, and the `VALUE` it tests.

use crate::{Expr, IntoAddedConstraint, IntoName, Keyword, Name, NotValid, SimpleExpr};

impl Expr {
    /// The keyword `VALUE`: inside a domain's `CHECK`, the value being
    /// tested.
    ///
    /// PostgreSQL reads `VALUE` there as the domain's value and anywhere else
    /// as a reference to a column named `value`, so this is for
    /// [`DomainCheck`] and [`DomainCreateStatement::check`](crate::DomainCreateStatement::check).
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// assert_eq!(
    ///     Domain::create(Name::runtime("percent"), ColumnType::SmallInteger)
    ///         .check(Expr::domain_value().between(0, 100))
    ///         .to_string(),
    ///     r#"CREATE DOMAIN "percent" AS smallint CHECK (VALUE BETWEEN 0 AND 100)"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.domain+4]
    pub fn domain_value() -> Expr {
        Expr::expr(SimpleExpr::Keyword(Keyword::Value))
    }
}

/// A domain's `CHECK` constraint: `[CONSTRAINT "name" ]CHECK (<expr>)`, its
/// expression over [`Expr::domain_value`].
///
/// The expression is the constraint, so the constructor takes it; a name is
/// what it may add. Unnamed, PostgreSQL derives one from the domain's
/// (`<domain>_check`). A table's [`Check`](crate::Check) is a different type
/// because a domain's takes neither of the clauses it carries: `NO INHERIT`
/// (`42P17`) and `ENFORCED` / `NOT ENFORCED` (`42P17` at `CREATE DOMAIN`,
/// `0A000` at `ALTER DOMAIN`).
///
/// ```
/// use pgorm_query::*;
///
/// assert_eq!(
///     Domain::alter(Name::runtime("quantity"))
///         .add_check(
///             DomainCheck::new(Expr::domain_value().lt(1000))
///                 .name(Name::runtime("small"))
///                 .not_valid(),
///         )
///         .to_string(),
///     r#"ALTER DOMAIN "quantity" ADD CONSTRAINT "small" CHECK (VALUE < 1000) NOT VALID"#
/// );
/// ```
// [spec:pgorm:req:sql.ddl.domain+4]
#[derive(Debug, Clone)]
pub struct DomainCheck {
    pub(crate) expr: SimpleExpr,
    pub(crate) name: Option<Name>,
}

impl DomainCheck {
    /// A `CHECK` over `expr`, unnamed.
    pub fn new<E>(expr: E) -> Self
    where
        E: Into<SimpleExpr>,
    {
        Self {
            expr: expr.into(),
            name: None,
        }
    }

    /// Name the constraint: `CONSTRAINT "name" CHECK (...)`, the name
    /// [`PendingDomainAlter`](crate::PendingDomainAlter)'s constraint
    /// actions take.
    #[must_use]
    pub fn name<N>(mut self, name: N) -> Self
    where
        N: IntoName,
    {
        self.name = Some(name.into_name());
        self
    }

    /// Add this `CHECK` `NOT VALID`, leaving the values columns of the domain
    /// already hold unchecked until
    /// [`validate_constraint`](crate::PendingDomainAlter::validate_constraint):
    /// what [`add_check`](crate::PendingDomainAlter::add_check) takes, and
    /// [`DomainCreateStatement::check`](crate::DomainCreateStatement::check)
    /// does not.
    pub fn not_valid(self) -> NotValid<Self> {
        NotValid { constraint: self }
    }

    /// The condition every value is held to.
    pub fn get_expr(&self) -> &SimpleExpr {
        &self.expr
    }

    /// The constraint's name, if it was given one.
    pub fn get_name(&self) -> Option<&Name> {
        self.name.as_ref()
    }
}

/// An expression, which is the unnamed `CHECK` over it, or a [`DomainCheck`]
/// already built: what each position that holds a domain's `CHECK` takes.
// [spec:pgorm:req:sql.ddl.domain+4]
pub trait IntoDomainCheck {
    /// The constraint.
    fn into_domain_check(self) -> DomainCheck;
}

impl IntoDomainCheck for DomainCheck {
    fn into_domain_check(self) -> DomainCheck {
        self
    }
}

impl<T> IntoDomainCheck for T
where
    T: Into<SimpleExpr>,
{
    fn into_domain_check(self) -> DomainCheck {
        DomainCheck::new(self)
    }
}

impl<C> IntoAddedConstraint<DomainCheck> for C
where
    C: IntoDomainCheck,
{
    fn into_added_constraint(self) -> (DomainCheck, bool) {
        (self.into_domain_check(), false)
    }
}
