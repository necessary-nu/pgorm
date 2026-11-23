//! A `CHECK` constraint: its expression, the name it is given, and whether the
//! server enforces it.

use crate::SimpleExpr;
use crate::types::{IntoName, Name};

use super::Enforcement;

/// A `CHECK` constraint:
/// `[CONSTRAINT "name" ]CHECK (<expr>)[ ENFORCED | NOT ENFORCED]`.
///
/// The expression is the constraint, so the constructor takes it; a name and
/// an [`Enforcement`] are what it may add. A column holds one with
/// [`ColumnDef::check`](crate::ColumnDef::check), a table with
/// [`TableCreateStatement::check`](crate::TableCreateStatement::check), and a
/// table that exists gains one with
/// [`TableAlterStatement::add_check`](crate::TableAlterStatement::add_check).
/// Each of those takes an expression as well, which is the unnamed, enforced
/// constraint ([`IntoCheck`]).
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// assert_eq!(
///     Table::create(Glyph::Table)
///         .col(ColumnDef::new(Glyph::Aspect).integer().check(Expr::col(Glyph::Aspect).gt(0)))
///         .check(
///             Check::new(Expr::col(Glyph::Aspect).lt(100))
///                 .name(Name::runtime("aspect_small"))
///                 .enforcement(Enforcement::NotEnforced),
///         )
///         .to_string(),
///     [
///         r#"CREATE TABLE "glyph" ( "aspect" integer CHECK ("aspect" > 0),"#,
///         r#"CONSTRAINT "aspect_small" CHECK ("aspect" < 100) NOT ENFORCED )"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// A `CHECK` has no deferrability: PostgreSQL never defers one, `NOT ENFORCED`
/// or not (`0A000`), so there is no method to ask for it.
// [spec:pgorm:req:sql.ddl.enforcement+1]
#[derive(Debug, Clone)]
pub struct Check {
    pub(crate) expr: SimpleExpr,
    pub(crate) name: Option<Name>,
    pub(crate) enforcement: Option<Enforcement>,
    pub(crate) no_inherit: bool,
}

impl Check {
    /// A `CHECK` over `expr`, unnamed and enforced as the server's default is.
    pub fn new<E>(expr: E) -> Self
    where
        E: Into<SimpleExpr>,
    {
        Self {
            expr: expr.into(),
            name: None,
            enforcement: None,
            no_inherit: false,
        }
    }

    /// Name the constraint: `CONSTRAINT "name" CHECK (...)`. Unnamed,
    /// PostgreSQL derives one from the table and the columns it reads.
    #[must_use]
    pub fn name<N>(mut self, name: N) -> Self
    where
        N: IntoName,
    {
        self.name = Some(name.into_name());
        self
    }

    /// Say whether the server holds rows to the constraint
    /// (`[spec:pgorm:req:sql.ddl.enforcement+1]`), replacing any already set.
    // [spec:pgorm:req:sql.ddl.enforcement+1]
    #[must_use]
    pub fn enforcement(mut self, enforcement: Enforcement) -> Self {
        self.enforcement = Some(enforcement);
        self
    }

    /// Keep the constraint from tables that inherit this one: `CHECK (...) NO
    /// INHERIT`, written before the enforcement, where a column's `CHECK`
    /// takes it alone (`42601` after `NOT ENFORCED`). A partitioned table's
    /// constraints always reach its partitions, so the server refuses it
    /// there (`42P16`).
    // [spec:pgorm:req:sql.ddl.create-table+16]
    #[must_use]
    pub fn no_inherit(mut self) -> Self {
        self.no_inherit = true;
        self
    }

    /// The condition every row is held to.
    pub fn get_expr(&self) -> &SimpleExpr {
        &self.expr
    }

    /// The constraint's name, if it was given one.
    pub fn get_name(&self) -> Option<&Name> {
        self.name.as_ref()
    }

    /// Whether the server enforces the constraint, if the caller said.
    pub fn get_enforcement(&self) -> Option<Enforcement> {
        self.enforcement
    }

    /// Whether the constraint is kept from inheriting tables.
    // [spec:pgorm:req:sql.ddl.create-table+16]
    pub fn is_no_inherit(&self) -> bool {
        self.no_inherit
    }
}

/// An expression, which is the unnamed and enforced `CHECK` over it, or a
/// [`Check`] already built: what each position that holds a `CHECK` takes.
// [spec:pgorm:req:sql.ddl.enforcement+1]
pub trait IntoCheck {
    /// The constraint.
    fn into_check(self) -> Check;
}

impl IntoCheck for Check {
    fn into_check(self) -> Check {
        self
    }
}

impl<T> IntoCheck for T
where
    T: Into<SimpleExpr>,
{
    fn into_check(self) -> Check {
        Check::new(self)
    }
}
