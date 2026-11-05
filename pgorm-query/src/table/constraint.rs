//! Constraints `ALTER TABLE` adds, validates, alters and drops by name: a
//! table-level `NOT NULL`, whether a constraint is enforced, the changes
//! `ALTER CONSTRAINT` makes to one that exists, and the `DROP CONSTRAINT`
//! that removes one of any kind.

use crate::TableForeignKey;
use crate::types::{IntoName, Name};

use super::{Check, IntoCheck};

/// A `NOT NULL` constraint added to a table that exists:
/// `ADD [CONSTRAINT "name" ]NOT NULL "column"[ NO INHERIT][ NOT VALID]`.
///
/// It is PostgreSQL 18's table-level spelling of the constraint a column
/// declares with [`ColumnDef::not_null`](crate::ColumnDef::not_null), over one
/// column, which the constructor takes. Only this spelling can be added
/// `NOT VALID`: the column-level clause has no place for it (`42601`), and a
/// `CREATE TABLE` creates every not-null constraint valid whatever it says,
/// since the new table has no rows to check. So this is what
/// [`TableAlterStatement::add_not_null`](crate::TableAlterStatement::add_not_null)
/// takes, and `CREATE TABLE` has no table-level `NOT NULL`: the column's own
/// clause carries the same constraint, its name and its `NO INHERIT` included.
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// assert_eq!(
///     Table::alter(Glyph::Table)
///         .add_not_null(
///             NotNullConstraint::new(Glyph::Aspect)
///                 .name(Name::runtime("glyph_aspect_present"))
///                 .not_valid(),
///         )
///         .to_string(),
///     r#"ALTER TABLE "glyph" ADD CONSTRAINT "glyph_aspect_present" NOT NULL "aspect" NOT VALID"#,
/// );
/// ```
// [spec:pgorm:req:sql.ddl.alter-table+15]
#[derive(Debug, Clone)]
pub struct NotNullConstraint {
    pub(crate) column: Name,
    pub(crate) name: Option<Name>,
    pub(crate) no_inherit: bool,
    pub(crate) not_valid: bool,
}

impl NotNullConstraint {
    /// A `NOT NULL` over `column`, unnamed, inherited and checked against the
    /// rows already there.
    pub fn new<C>(column: C) -> Self
    where
        C: IntoName,
    {
        Self {
            column: column.into_name(),
            name: None,
            no_inherit: false,
            not_valid: false,
        }
    }

    /// Name the constraint: `CONSTRAINT "name" NOT NULL ...`. Unnamed,
    /// PostgreSQL derives `<table>_<column>_not_null`.
    #[must_use]
    pub fn name<N>(mut self, name: N) -> Self
    where
        N: IntoName,
    {
        self.name = Some(name.into_name());
        self
    }

    /// Keep the constraint from the tables that inherit this one:
    /// `NO INHERIT`. The server refuses it on a partitioned table (`0A000`),
    /// and over a column whose inheritable `NOT NULL` already exists
    /// (`55000`).
    #[must_use]
    pub fn no_inherit(mut self) -> Self {
        self.no_inherit = true;
        self
    }

    /// Add the constraint without checking the rows already there:
    /// `NOT VALID`. New rows are held to it at once (`23502`), and
    /// [`validate_constraint`](crate::TableAlterStatement::validate_constraint)
    /// checks the rest later.
    #[must_use]
    pub fn not_valid(mut self) -> Self {
        self.not_valid = true;
        self
    }

    /// The column the constraint holds to.
    pub fn get_column(&self) -> &Name {
        &self.column
    }

    /// The constraint's name, if it was given one.
    pub fn get_name(&self) -> Option<&Name> {
        self.name.as_ref()
    }

    /// Whether the constraint is kept from inheriting tables.
    pub fn is_no_inherit(&self) -> bool {
        self.no_inherit
    }

    /// Whether the rows already there are left unchecked.
    pub fn is_not_valid(&self) -> bool {
        self.not_valid
    }
}

/// Whether the server holds rows to a constraint: PostgreSQL 18's `ENFORCED`
/// and `NOT ENFORCED`.
///
/// A foreign key takes it through
/// [`TableForeignKey::enforcement`](crate::TableForeignKey::enforcement) and a
/// `CHECK` through [`Check::enforcement`](crate::Check::enforcement). Nothing
/// else can: PostgreSQL refuses either word on a primary key, a unique key or a
/// `NOT NULL` (`0A000` at table level, `42601` on a column), so
/// [`TableKey`](crate::TableKey) and the not-null spellings have no method for
/// it.
///
/// A closed pair rather than a flag, as
/// [`Deferrability`](crate::Deferrability) is a closed choice: unset, the
/// clause renders nothing and the server's default, `ENFORCED`, holds, so
/// [`Enforced`](Self::Enforced) renders only because a caller said it.
// [spec:pgorm:req:sql.ddl.enforcement]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enforcement {
    /// `ENFORCED`: every row written is checked, as by default.
    Enforced,
    /// `NOT ENFORCED`: the constraint is recorded and never checked. It is
    /// never valid, a foreign key's `ON DELETE` and `ON UPDATE` actions never
    /// fire, and `VALIDATE CONSTRAINT` refuses it (`55000`); a foreign key, or
    /// on PostgreSQL 19 a `CHECK`, is enforced again with
    /// [`ConstraintChange::Enforced`](crate::ConstraintChange::Enforced).
    NotEnforced,
}

impl Enforcement {
    /// The clause this state renders as, with the space that separates it
    /// from the constraint it follows.
    // [spec:pgorm:req:sql.ddl.enforcement]
    pub(crate) fn clause(self) -> &'static str {
        match self {
            Self::Enforced => " ENFORCED",
            Self::NotEnforced => " NOT ENFORCED",
        }
    }
}

/// What `ALTER CONSTRAINT "name" ...` changes about a constraint that exists.
///
/// The server alone knows which kind of constraint a name holds, and each
/// change applies to the kinds its variant names, so asking another kind for
/// it is refused there (`42809`). Where those kinds differ by release, the
/// variant says so: the builder cannot tell a `CHECK`'s name from a foreign
/// key's, so no target can rule the difference out by type.
// [spec:pgorm:req:sql.ddl.alter-table+15]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConstraintChange {
    /// `INHERIT`: a `NOT NULL` constraint passes to inheriting tables again,
    /// each child that lacks one taking it.
    Inherit,
    /// `NO INHERIT`: a `NOT NULL` constraint is kept from inheriting tables
    /// from now on. Each child keeps the copy it already has, as its own. A
    /// partitioned table's constraint cannot be kept from its partitions
    /// (`0A000`).
    NoInherit,
    /// `ENFORCED`: a foreign key, or from PostgreSQL 19 a `CHECK`, is
    /// checked again, every row already there included — one that breaks it
    /// refuses the statement (`23503`, `23514`) — and is valid once it
    /// passes. PostgreSQL 18 cannot alter a `CHECK`'s enforcement (`42809`).
    // [spec:pgorm:req:sql.ddl.enforcement]
    Enforced,
    /// `NOT ENFORCED`: a foreign key, or from PostgreSQL 19 a `CHECK`, is
    /// left unchecked and not valid.
    // [spec:pgorm:req:sql.ddl.enforcement]
    NotEnforced,
}

impl ConstraintChange {
    /// The words this change is written with, after the constraint's name.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub(crate) fn clause(self) -> &'static str {
        match self {
            Self::Inherit => "INHERIT",
            Self::NoInherit => "NO INHERIT",
            Self::Enforced => "ENFORCED",
            Self::NotEnforced => "NOT ENFORCED",
        }
    }
}

/// What a drop does to the objects that depend on what it drops: `RESTRICT`,
/// refusing the drop while any does (`2BP01`), which is PostgreSQL's default,
/// or `CASCADE`, dropping them with it.
// [spec:pgorm:req:sql.ddl.alter-table+15]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropBehavior {
    /// `RESTRICT`: refuse the drop while anything depends on it.
    Restrict,
    /// `CASCADE`: drop whatever depends on it too.
    Cascade,
}

impl DropBehavior {
    /// The keyword, as the statement writes it.
    pub(crate) fn keyword(self) -> &'static str {
        match self {
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
        }
    }
}

/// A constraint dropped by name: `DROP CONSTRAINT [IF EXISTS ]"name"[
/// RESTRICT | CASCADE]`, what
/// [`TableAlterStatement::drop_constraint`](crate::TableAlterStatement::drop_constraint)
/// takes.
///
/// A constraint of any kind is dropped the same way — a key, a foreign key, a
/// `CHECK`, PostgreSQL 18's `NOT NULL` — so the drop names the constraint and
/// not its kind; which kind the name holds is the server's knowledge. A name
/// on its own converts into the plain drop ([`IntoConstraintDrop`]).
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// assert_eq!(
///     Table::alter(Glyph::Table)
///         .drop_constraint(Name::runtime("glyph_aspect_check"))
///         .drop_constraint(
///             ConstraintDrop::new(Name::runtime("glyph_pkey"))
///                 .if_exists()
///                 .cascade(),
///         )
///         .to_string(),
///     [
///         r#"ALTER TABLE "glyph" DROP CONSTRAINT "glyph_aspect_check","#,
///         r#"DROP CONSTRAINT IF EXISTS "glyph_pkey" CASCADE"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// The drop is the server's to refuse where it cannot be made: a name the
/// table has no constraint under (`42704`, which `if_exists` turns into a
/// notice), a key another table's foreign key depends on unless the drop
/// cascades to it (`2BP01`), a constraint a table inherited from its parent
/// (`42P16`), and the `NOT NULL` of a primary-key column (`42P16`).
// [spec:pgorm:req:sql.ddl.alter-table+15]
#[derive(Debug, Clone)]
pub struct ConstraintDrop {
    pub(crate) name: Name,
    pub(crate) if_exists: bool,
    pub(crate) behavior: Option<DropBehavior>,
}

impl ConstraintDrop {
    /// Drop the constraint called `name`, refusing a name the table does not
    /// have and anything that depends on it.
    pub fn new<N>(name: N) -> Self
    where
        N: IntoName,
    {
        Self {
            name: name.into_name(),
            if_exists: false,
            behavior: None,
        }
    }

    /// Pass over a name the table has no constraint under, with a notice:
    /// `DROP CONSTRAINT IF EXISTS "name"`.
    #[must_use]
    pub fn if_exists(mut self) -> Self {
        self.if_exists = true;
        self
    }

    /// Drop whatever depends on the constraint too: ` CASCADE`. The last of
    /// `cascade()` and `restrict()` wins.
    #[must_use]
    pub fn cascade(mut self) -> Self {
        self.behavior = Some(DropBehavior::Cascade);
        self
    }

    /// Refuse the drop while anything depends on the constraint, saying so:
    /// ` RESTRICT`, which is also what an unsaid behavior does. The last of
    /// `cascade()` and `restrict()` wins.
    #[must_use]
    pub fn restrict(mut self) -> Self {
        self.behavior = Some(DropBehavior::Restrict);
        self
    }

    /// The name of the constraint dropped.
    pub fn get_name(&self) -> &Name {
        &self.name
    }

    /// Whether a missing name is passed over.
    pub fn is_if_exists(&self) -> bool {
        self.if_exists
    }

    /// What the drop does to dependent objects, if the caller said.
    pub fn get_behavior(&self) -> Option<DropBehavior> {
        self.behavior
    }
}

/// A name, which is the plain drop of the constraint it names, or a
/// [`ConstraintDrop`] already built: what
/// [`TableAlterStatement::drop_constraint`](crate::TableAlterStatement::drop_constraint)
/// takes.
// [spec:pgorm:req:sql.ddl.alter-table+15]
pub trait IntoConstraintDrop {
    /// The drop.
    fn into_constraint_drop(self) -> ConstraintDrop;
}

impl IntoConstraintDrop for ConstraintDrop {
    fn into_constraint_drop(self) -> ConstraintDrop {
        self
    }
}

impl<N> IntoConstraintDrop for N
where
    N: IntoName,
{
    fn into_constraint_drop(self) -> ConstraintDrop {
        ConstraintDrop::new(self)
    }
}

/// A foreign key or `CHECK` added `NOT VALID`: the rows already in the table
/// are left unchecked, and every row written from then on is held to it, until
/// [`validate_constraint`](crate::TableAlterStatement::validate_constraint)
/// checks the rest.
///
/// [`TableForeignKey::not_valid`] and [`Check::not_valid`] make one, and only
/// the `ALTER TABLE ... ADD` actions take it —
/// [`add_foreign_key`](crate::TableAlterStatement::add_foreign_key) and
/// [`add_check`](crate::TableAlterStatement::add_check) — because only there
/// does the clause say anything: a `CREATE TABLE` creates every constraint
/// valid, there being no rows to leave unchecked, so a table-level `NOT VALID`
/// there is taken and ignored, and a column's `CHECK` or `REFERENCES` has no
/// place for it at all (`42601`). So the `CREATE TABLE` positions and a
/// column's `CHECK` take no `NotValid`:
///
/// ```compile_fail,E0277
/// use pgorm_query::{tests_cfg::*, *};
///
/// Table::create(Glyph::Table).check(Check::new(Expr::col(Glyph::Aspect).gt(0)).not_valid());
/// ```
///
/// ```compile_fail,E0277
/// use pgorm_query::{tests_cfg::*, *};
///
/// ColumnDef::new(Glyph::Aspect).check(Check::new(Expr::col(Glyph::Aspect).gt(0)).not_valid());
/// ```
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let mut font = TableForeignKey::new(Char::Table, Char::FontId, Font::Table, Font::Id);
/// font.name(Name::runtime("character_font"));
///
/// assert_eq!(
///     Table::alter(Char::Table)
///         .add_foreign_key(font.not_valid())
///         .add_check(Check::new(Expr::col(Char::SizeW).gt(0)).not_valid())
///         .to_string(),
///     [
///         r#"ALTER TABLE "character" ADD CONSTRAINT "character_font""#,
///         r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id") NOT VALID,"#,
///         r#"ADD CHECK ("size_w" > 0) NOT VALID"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// A `NOT ENFORCED` constraint is never valid, whatever it says, and
/// `VALIDATE CONSTRAINT` refuses it (`55000`).
// [spec:pgorm:req:sql.ddl.alter-table+15]
#[derive(Debug, Clone)]
pub struct NotValid<C> {
    pub(crate) constraint: C,
}

impl<C> NotValid<C> {
    /// The constraint added `NOT VALID`.
    pub fn get_constraint(&self) -> &C {
        &self.constraint
    }
}

impl TableForeignKey {
    /// Add this key `NOT VALID`, leaving the rows already there unchecked:
    /// what [`add_foreign_key`](crate::TableAlterStatement::add_foreign_key)
    /// takes, and nothing that creates a table does ([`NotValid`]).
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn not_valid(self) -> NotValid<Self> {
        NotValid { constraint: self }
    }
}

impl Check {
    /// Add this `CHECK` `NOT VALID`, leaving the rows already there
    /// unchecked: what [`add_check`](crate::TableAlterStatement::add_check)
    /// takes, and nothing that creates a table or a column does
    /// ([`NotValid`]).
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn not_valid(self) -> NotValid<Self> {
        NotValid { constraint: self }
    }
}

/// What an `ALTER TABLE ... ADD` action takes for a constraint of kind `C`:
/// anything that converts into the constraint, which is checked against the
/// rows already there, or the constraint wrapped in [`NotValid`], which is
/// not.
// [spec:pgorm:req:sql.ddl.alter-table+15]
pub trait IntoAddedConstraint<C> {
    /// The constraint, and whether it is added `NOT VALID`.
    fn into_added_constraint(self) -> (C, bool);
}

impl<C> IntoAddedConstraint<C> for NotValid<C> {
    fn into_added_constraint(self) -> (C, bool) {
        (self.constraint, true)
    }
}

impl<F> IntoAddedConstraint<TableForeignKey> for F
where
    F: Into<TableForeignKey>,
{
    fn into_added_constraint(self) -> (TableForeignKey, bool) {
        (self.into(), false)
    }
}

impl<C> IntoAddedConstraint<Check> for C
where
    C: IntoCheck,
{
    fn into_added_constraint(self) -> (Check, bool) {
        (self.into_check(), false)
    }
}
