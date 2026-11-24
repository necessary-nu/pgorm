//! `ALTER TYPE` on a composite: attributes added, dropped, retyped and
//! renamed.

use super::{CompositeAttribute, PendingTypeAlter, TypeRef};
use crate::{Collation, ColumnType, DropBehavior, IntoCollation, IntoName, Name, QueryBuilder};

/// One change to a composite's attributes, as `ALTER TYPE` chains them.
// [spec:pgorm:req:sql.ddl.type-composite+3]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum AttributeChange {
    /// `ADD ATTRIBUTE "name" <type>[ COLLATE <collation>]`.
    Add(CompositeAttribute),
    /// `DROP ATTRIBUTE [IF EXISTS ]"name"`.
    Drop { name: Name, if_exists: bool },
    /// `ALTER ATTRIBUTE "name" TYPE <type>[ COLLATE <collation>]`: the
    /// attribute's new type, and the collation it takes, written as an added
    /// attribute's are.
    Retype(CompositeAttribute),
}

/// `ALTER TYPE <composite> <change>[ <behavior>], ...`: one or more changes to
/// a composite's attributes, each an [`AttributeChange`].
///
/// [`PendingTypeAlter`]'s attribute methods start it with its first change,
/// and the same methods here append the rest, which PostgreSQL applies in
/// order in one statement; a composite alteration with no change has no
/// constructor. Renaming an attribute is a statement of its own,
/// [`AttributeRenameStatement`], because PostgreSQL takes `RENAME` alone
/// (`42601` beside another change).
///
/// ```
/// use pgorm_query::{extension::Type, *};
///
/// assert_eq!(
///     Type::alter(Name::runtime("address"))
///         .add_attribute(Name::runtime("zip"), ColumnType::Text)
///         .drop_attribute_if_exists(Name::runtime("no"))
///         .alter_attribute(Name::runtime("street"), ColumnType::string(Some(200)))
///         .cascade()
///         .to_string(),
///     [
///         r#"ALTER TYPE "address" ADD ATTRIBUTE "zip" text CASCADE,"#,
///         r#"DROP ATTRIBUTE IF EXISTS "no" CASCADE,"#,
///         r#"ALTER ATTRIBUTE "street" TYPE varchar(200) CASCADE"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// The behavior is the statement's, written after each change, since each
/// takes one: a composite that is the type of a typed table (`CREATE TABLE
/// ... OF`) is altered only `CASCADE`, which carries every change into the
/// table (`2BP01` without it), and a column of the type in another table
/// refuses a retype even then (`0A000`). The rest is the server's to refuse:
/// an attribute added twice (`42701`) and one dropped or retyped that the type
/// does not have (`42703`, which `drop_attribute_if_exists` turns into a
/// notice for a drop).
// [spec:pgorm:req:sql.ddl.type-composite+3]
#[derive(Debug, Clone)]
pub struct CompositeAlterStatement {
    pub(crate) name: TypeRef,
    pub(crate) first: AttributeChange,
    pub(crate) rest: Vec<AttributeChange>,
    pub(crate) behavior: Option<DropBehavior>,
}

/// `ALTER TYPE <composite> RENAME ATTRIBUTE "from" TO "to"[ <behavior>]`.
///
/// A statement of its own, as a table's column rename is: PostgreSQL takes
/// `RENAME` as an `ALTER TYPE`'s sole action (`42601` beside another), so it
/// does not chain with a [`CompositeAlterStatement`]'s changes:
///
/// ```compile_fail,E0599
/// use pgorm_query::{extension::Type, *};
///
/// Type::alter(Name::runtime("address"))
///     .rename_attribute(Name::runtime("no"), Name::runtime("number"))
///     .add_attribute(Name::runtime("zip"), ColumnType::Text);
/// ```
///
/// ```
/// use pgorm_query::{extension::Type, *};
///
/// assert_eq!(
///     Type::alter(Name::runtime("address"))
///         .rename_attribute(Name::runtime("no"), Name::runtime("number"))
///         .cascade()
///         .to_string(),
///     r#"ALTER TYPE "address" RENAME ATTRIBUTE "no" TO "number" CASCADE"#
/// );
/// ```
// [spec:pgorm:req:sql.ddl.type-composite+3]
#[derive(Debug, Clone)]
pub struct AttributeRenameStatement {
    pub(crate) name: TypeRef,
    pub(crate) from: Name,
    pub(crate) to: Name,
    pub(crate) behavior: Option<DropBehavior>,
}

fn attribute<N>(
    name: N,
    column_type: ColumnType,
    collation: Option<Collation>,
) -> CompositeAttribute
where
    N: IntoName,
{
    CompositeAttribute {
        name: name.into_name(),
        column_type,
        collation,
    }
}

/// An attribute under a named collation.
fn collated<N, C>(name: N, column_type: ColumnType, collation: C) -> CompositeAttribute
where
    N: IntoName,
    C: IntoCollation,
{
    attribute(name, column_type, Some(collation.into_collation()))
}

impl AttributeChange {
    fn drop<N>(name: N, if_exists: bool) -> Self
    where
        N: IntoName,
    {
        Self::Drop {
            name: name.into_name(),
            if_exists,
        }
    }
}

impl PendingTypeAlter {
    fn change(self, first: AttributeChange) -> CompositeAlterStatement {
        CompositeAlterStatement {
            name: self.name,
            first,
            rest: Vec::new(),
            behavior: None,
        }
    }

    /// Add an attribute to the composite: `ADD ATTRIBUTE "name" <type>`,
    /// written as [`TypeCreateStatement::attribute`](super::TypeCreateStatement::attribute)
    /// writes one.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub fn add_attribute<N>(self, name: N, column_type: ColumnType) -> CompositeAlterStatement
    where
        N: IntoName,
    {
        self.change(AttributeChange::Add(attribute(name, column_type, None)))
    }

    /// Add an attribute under a named collation: `ADD ATTRIBUTE "name" <type>
    /// COLLATE "collation"`.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub fn add_attribute_collated<N, C>(
        self,
        name: N,
        ty: ColumnType,
        collation: C,
    ) -> CompositeAlterStatement
    where
        N: IntoName,
        C: IntoCollation,
    {
        self.change(AttributeChange::Add(collated(name, ty, collation)))
    }

    /// Drop an attribute: `DROP ATTRIBUTE "name"`.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub fn drop_attribute<N>(self, name: N) -> CompositeAlterStatement
    where
        N: IntoName,
    {
        self.change(AttributeChange::drop(name, false))
    }

    /// Drop an attribute, passing over one the type does not have with a
    /// notice: `DROP ATTRIBUTE IF EXISTS "name"`.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub fn drop_attribute_if_exists<N>(self, name: N) -> CompositeAlterStatement
    where
        N: IntoName,
    {
        self.change(AttributeChange::drop(name, true))
    }

    /// Give an attribute a new type: `ALTER ATTRIBUTE "name" TYPE <type>`.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub fn alter_attribute<N>(self, name: N, column_type: ColumnType) -> CompositeAlterStatement
    where
        N: IntoName,
    {
        self.change(AttributeChange::Retype(attribute(name, column_type, None)))
    }

    /// Give an attribute a new type under a named collation: `ALTER ATTRIBUTE
    /// "name" TYPE <type> COLLATE "collation"`.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub fn alter_attribute_collated<N, C>(
        self,
        name: N,
        ty: ColumnType,
        collation: C,
    ) -> CompositeAlterStatement
    where
        N: IntoName,
        C: IntoCollation,
    {
        self.change(AttributeChange::Retype(collated(name, ty, collation)))
    }

    /// Rename an attribute: `RENAME ATTRIBUTE "from" TO "to"`, a statement of
    /// its own ([`AttributeRenameStatement`]).
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub fn rename_attribute<F, T>(self, from: F, to: T) -> AttributeRenameStatement
    where
        F: IntoName,
        T: IntoName,
    {
        AttributeRenameStatement {
            name: self.name,
            from: from.into_name(),
            to: to.into_name(),
            behavior: None,
        }
    }
}

impl CompositeAlterStatement {
    fn and(mut self, change: AttributeChange) -> Self {
        self.rest.push(change);
        self
    }

    /// Add a further attribute, after the changes already listed.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    #[must_use]
    pub fn add_attribute<N>(self, name: N, column_type: ColumnType) -> Self
    where
        N: IntoName,
    {
        self.and(AttributeChange::Add(attribute(name, column_type, None)))
    }

    /// Add a further attribute under a named collation.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    #[must_use]
    pub fn add_attribute_collated<N, C>(self, name: N, ty: ColumnType, collation: C) -> Self
    where
        N: IntoName,
        C: IntoCollation,
    {
        self.and(AttributeChange::Add(collated(name, ty, collation)))
    }

    /// Drop a further attribute.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    #[must_use]
    pub fn drop_attribute<N>(self, name: N) -> Self
    where
        N: IntoName,
    {
        self.and(AttributeChange::drop(name, false))
    }

    /// Drop a further attribute if the type has it.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    #[must_use]
    pub fn drop_attribute_if_exists<N>(self, name: N) -> Self
    where
        N: IntoName,
    {
        self.and(AttributeChange::drop(name, true))
    }

    /// Retype a further attribute.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    #[must_use]
    pub fn alter_attribute<N>(self, name: N, column_type: ColumnType) -> Self
    where
        N: IntoName,
    {
        self.and(AttributeChange::Retype(attribute(name, column_type, None)))
    }

    /// Retype a further attribute under a named collation.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    #[must_use]
    pub fn alter_attribute_collated<N, C>(self, name: N, ty: ColumnType, collation: C) -> Self
    where
        N: IntoName,
        C: IntoCollation,
    {
        self.and(AttributeChange::Retype(collated(name, ty, collation)))
    }

    /// Carry every change into the typed tables and their dependents:
    /// ` CASCADE` after each. The last of `cascade()` and `restrict()` wins.
    #[must_use]
    pub fn cascade(mut self) -> Self {
        self.behavior = Some(DropBehavior::Cascade);
        self
    }

    /// Refuse every change a typed table depends on, saying so: ` RESTRICT`
    /// after each, which is also what an unsaid behavior does.
    #[must_use]
    pub fn restrict(mut self) -> Self {
        self.behavior = Some(DropBehavior::Restrict);
        self
    }

    /// The changes, in the order they apply; never empty.
    pub fn changes(&self) -> impl Iterator<Item = &AttributeChange> {
        std::iter::once(&self.first).chain(self.rest.iter())
    }

    /// The behavior each change carries, if the caller said.
    pub fn get_behavior(&self) -> Option<DropBehavior> {
        self.behavior
    }
}

impl AttributeRenameStatement {
    /// Carry the rename into the typed tables: ` CASCADE`. The last of
    /// `cascade()` and `restrict()` wins.
    #[must_use]
    pub fn cascade(mut self) -> Self {
        self.behavior = Some(DropBehavior::Cascade);
        self
    }

    /// Refuse the rename while a typed table depends on the type, saying so:
    /// ` RESTRICT`.
    #[must_use]
    pub fn restrict(mut self) -> Self {
        self.behavior = Some(DropBehavior::Restrict);
        self
    }
}

/// Renders the statement with every value inlined as an escaped SQL literal.
/// This is its only rendering: it exposes no placeholder-emitting build, so
/// nothing here is left to bind.
// [spec:pgorm:req:sql.ddl+12]
impl std::fmt::Display for CompositeAlterStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_composite_alter_statement(self, &mut sql);
        f.write_str(&sql)
    }
}

/// Renders the statement with every value inlined as an escaped SQL literal.
/// This is its only rendering: it exposes no placeholder-emitting build, so
/// nothing here is left to bind.
// [spec:pgorm:req:sql.ddl+12]
impl std::fmt::Display for AttributeRenameStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_attribute_rename_statement(self, &mut sql);
        f.write_str(&sql)
    }
}
