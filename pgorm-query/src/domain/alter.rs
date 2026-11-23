//! `ALTER DOMAIN`: a domain awaiting its action, and the statement choosing
//! one makes.

use crate::{
    ConstraintDrop, IntoAddedConstraint, IntoConstraintDrop, IntoName, Name, SimpleExpr,
    extension::TypeRef,
};

use super::DomainCheck;

/// What an `ALTER DOMAIN` does: one action, the grammar taking no list.
// [spec:pgorm:req:sql.ddl.domain+4]
#[derive(Debug, Clone)]
pub(crate) enum DomainAlterAction {
    SetDefault(SimpleExpr),
    DropDefault,
    SetNotNull,
    DropNotNull,
    /// The constraint, and whether it is added `NOT VALID`.
    AddCheck(DomainCheck, bool),
    DropConstraint(ConstraintDrop),
    RenameConstraint(Name, Name),
    ValidateConstraint(Name),
    Rename(Name),
}

/// A domain awaiting its action.
///
/// PostgreSQL has no spelling for an `ALTER DOMAIN` that does nothing, and
/// none for one doing two things at once (`42601` at the comma), so this is
/// what [`Domain::alter`](super::Domain::alter) returns: naming the domain is
/// not yet a statement, and each action method consumes it to yield a
/// [`DomainAlterStatement`] carrying exactly that action.
///
/// ```compile_fail,E0599
/// use pgorm_query::*;
///
/// Domain::alter(Name::runtime("d")).to_string();
/// ```
///
/// `ALTER DOMAIN` has no `IF EXISTS` (`42601`), so neither the pending alter
/// nor the statement offers one; a missing domain is `42704`. Two forms are
/// not built: `SET SCHEMA`, which the builder has for no table, sequence or
/// type either, and `OWNER TO`, which names a role.
// [spec:pgorm:req:sql.ddl.domain+4]
#[derive(Debug, Clone)]
pub struct PendingDomainAlter {
    name: TypeRef,
}

impl PendingDomainAlter {
    pub(super) fn new(name: TypeRef) -> Self {
        Self { name }
    }

    fn with(self, action: DomainAlterAction) -> DomainAlterStatement {
        DomainAlterStatement {
            name: self.name,
            action,
        }
    }

    /// `SET DEFAULT <expr>`: the default a column of the domain takes from
    /// now on, where it has none of its own. Rows already written keep their
    /// values.
    pub fn set_default<E>(self, expr: E) -> DomainAlterStatement
    where
        E: Into<SimpleExpr>,
    {
        self.with(DomainAlterAction::SetDefault(expr.into()))
    }

    /// `DROP DEFAULT`
    pub fn drop_default(self) -> DomainAlterStatement {
        self.with(DomainAlterAction::DropDefault)
    }

    /// `SET NOT NULL`: refused while a column of the domain holds a null
    /// (`23502`).
    pub fn set_not_null(self) -> DomainAlterStatement {
        self.with(DomainAlterAction::SetNotNull)
    }

    /// `DROP NOT NULL`
    pub fn drop_not_null(self) -> DomainAlterStatement {
        self.with(DomainAlterAction::DropNotNull)
    }

    /// `ADD [CONSTRAINT "name" ]CHECK (<expr>)[ NOT VALID]`.
    ///
    /// Takes an expression or a [`DomainCheck`], checked against every value
    /// the domain's columns already hold (`23514` for one that fails), or a
    /// [`DomainCheck::not_valid`], which leaves those unchecked until
    /// [`validate_constraint`](Self::validate_constraint) and holds every value
    /// written from then on.
    pub fn add_check<C>(self, check: C) -> DomainAlterStatement
    where
        C: IntoAddedConstraint<DomainCheck>,
    {
        let (check, not_valid) = check.into_added_constraint();
        self.with(DomainAlterAction::AddCheck(check, not_valid))
    }

    /// `DROP CONSTRAINT [IF EXISTS ]"name"[ CASCADE | RESTRICT]`: a name, or a
    /// [`ConstraintDrop`] carrying `IF EXISTS` and a behavior. A name the
    /// domain has no constraint under is `42704`, a notice under `IF
    /// EXISTS`.
    pub fn drop_constraint<D>(self, drop: D) -> DomainAlterStatement
    where
        D: IntoConstraintDrop,
    {
        self.with(DomainAlterAction::DropConstraint(
            drop.into_constraint_drop(),
        ))
    }

    /// `RENAME CONSTRAINT "from" TO "to"`. A missing constraint is `42704`
    /// and a name already taken `42710`.
    pub fn rename_constraint<F, T>(self, from: F, to: T) -> DomainAlterStatement
    where
        F: IntoName,
        T: IntoName,
    {
        self.with(DomainAlterAction::RenameConstraint(
            from.into_name(),
            to.into_name(),
        ))
    }

    /// `VALIDATE CONSTRAINT "name"`: check the values a `NOT VALID` constraint
    /// left unchecked, refused while any fails (`23514`).
    pub fn validate_constraint<N>(self, name: N) -> DomainAlterStatement
    where
        N: IntoName,
    {
        self.with(DomainAlterAction::ValidateConstraint(name.into_name()))
    }

    /// `RENAME TO "new"`. The new name is a bare identifier, because a rename
    /// leaves the domain in the schema it is in (`42601` for a qualified
    /// one); a name a type already has is `42710`.
    pub fn rename_to<N>(self, name: N) -> DomainAlterStatement
    where
        N: IntoName,
    {
        self.with(DomainAlterAction::Rename(name.into_name()))
    }
}

/// Alter a domain
///
/// A statement of this type always names a domain and carries exactly one
/// action: it is reachable only by choosing one on a [`PendingDomainAlter`].
///
/// ```
/// use pgorm_query::*;
///
/// assert_eq!(
///     Domain::alter((Name::runtime("billing"), Name::runtime("quantity")))
///         .validate_constraint(Name::runtime("small"))
///         .to_string(),
///     r#"ALTER DOMAIN "billing"."quantity" VALIDATE CONSTRAINT "small""#
/// );
/// ```
// [spec:pgorm:req:sql.ddl.domain+4]
#[derive(Debug, Clone)]
pub struct DomainAlterStatement {
    pub(crate) name: TypeRef,
    pub(crate) action: DomainAlterAction,
}

impl DomainAlterStatement {
    /// The domain altered
    pub fn get_name(&self) -> &TypeRef {
        &self.name
    }
}
