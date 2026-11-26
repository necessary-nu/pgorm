//! Scopes: conditions an application holds a table's statements to, such as
//! `company_id = $me.company`, which no statement on the table may be built
//! without.

use crate::entity::column_def::{compared_column, compared_value};
use crate::pipeline::{Binder, Expr as PipelineExpr};
use crate::{ActiveModel, ActiveValue, Column, Entity, EntityName, StaticName, Value};
use pgorm_query::{
    AliasName, Condition, ConditionalStatement, Expr, Expression, FromItem, IntoFromItem, Name,
    NamedTable, TableName, TableSample,
};
use std::any::{Any, TypeId};
use std::fmt;
use std::marker::PhantomData;

/// An application type a scoped entity's statements are held to, such as a
/// `CompanyId` or a `UserId`: `#[derive(Scope)]` implements it, and the type
/// is also the value type of the columns it scopes.
///
/// A scope value is a context of its own, holding itself:
/// `company::Entity::find_for(&CompanyId(7))` reads company 7's row, the
/// company entity being scoped by `CompanyId` alone.
// [spec:pgorm:def:scope.declare+1]
pub trait Scope: Clone + 'static {}

/// How a [`ScopeContext`] holds one scope: one value, which a statement under
/// it is held to, or every value, an [`Every`], which holds it to none.
// [spec:pgorm:def:scope.context]
pub enum Held<'a, S: ?Sized> {
    /// One value: a statement's rows are those of this value.
    One(&'a S),
    /// Every value, lifted on purpose: the scope puts no condition on a
    /// statement, and an insert takes its column from the row.
    Every,
}

impl<S: ?Sized> Clone for Held<'_, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: ?Sized> Copy for Held<'_, S> {}

impl<S: ?Sized> fmt::Debug for Held<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::One(_) => f.write_str("One(..)"),
            Self::Every => f.write_str("Every"),
        }
    }
}

/// What a statement on a scoped entity is built under: the scopes it holds,
/// each one value or every value.
///
/// One scope value is a context, and so is an [`Every`], holding its scope's
/// every value. `#[derive(ScopeContext)]` makes a struct of them one, each
/// field a scope or an `Every` of one, and implements [`Provides`] for each
/// scope it holds, which is what the compiler checks a statement's context
/// against. [`NoContext`] holds nothing, and is the context of every form that
/// takes none.
// [spec:pgorm:def:scope.context]
pub trait ScopeContext {
    /// How this context holds the scope whose [`TypeId`] is `scope`, or
    /// `None` when it does not hold it.
    fn held(&self, scope: TypeId) -> Option<Held<'_, dyn Any>>;
}

/// A [`ScopeContext`] that holds the scope `S`: what the bounds of a scoped
/// entity's statements ask of their context. `#[derive(ScopeContext)]`
/// implements it for each scope a struct holds.
// [spec:pgorm:def:scope.context]
pub trait Provides<S: Scope>: ScopeContext {}

// [spec:pgorm:def:scope.context]
impl<S: Scope> ScopeContext for S {
    fn held(&self, scope: TypeId) -> Option<Held<'_, dyn Any>> {
        (scope == TypeId::of::<S>()).then_some(Held::One(self as &dyn Any))
    }
}

// [spec:pgorm:def:scope.context]
impl<S: Scope> Provides<S> for S {}

/// Every value of the scope `S`, lifted on purpose: a context holding it
/// satisfies `S` and puts no condition on a statement for it, and an insert
/// under it fills nothing, so the row sets `S`'s column itself.
///
/// It is how an administrator over every user of one company is written:
///
/// ```
/// use pgorm::entity::prelude::*;
/// use pgorm::tests_cfg::scoped::{note, CompanyId, UserId};
///
/// #[derive(Clone, Copy, Debug, ScopeContext)]
/// pub struct CompanyAdmin {
///     pub company: CompanyId,
///     pub users: Every<UserId>,
/// }
///
/// let admin = CompanyAdmin { company: CompanyId(7), users: Every::new() };
/// assert_eq!(
///     note::Entity::find_for(&admin).build().0,
///     [
///         r#"SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body""#,
///         r#"FROM "note" WHERE "note"."company_id" = $1"#,
///     ]
///     .join(" ")
/// );
/// ```
// [spec:pgorm:def:scope.every]
pub struct Every<S>(PhantomData<fn() -> S>);

impl<S> Every<S> {
    /// Every value of `S`.
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<S> Default for Every<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Clone for Every<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Every<S> {}

impl<S> fmt::Debug for Every<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Every<{}>", scope_name::<S>())
    }
}

// [spec:pgorm:def:scope.every]
impl<S: Scope> ScopeContext for Every<S> {
    fn held(&self, scope: TypeId) -> Option<Held<'_, dyn Any>> {
        (scope == TypeId::of::<S>()).then_some(Held::Every)
    }
}

// [spec:pgorm:def:scope.every]
impl<S: Scope> Provides<S> for Every<S> {}

/// The context that holds no scope: the one the forms taking no context build
/// under. An unscoped entity's statements run under it; a scoped entity's
/// cannot, so its plain forms are compile errors naming the entity and its
/// scopes.
// [spec:pgorm:def:scope.context]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoContext;

impl ScopeContext for NoContext {
    fn held(&self, _scope: TypeId) -> Option<Held<'_, dyn Any>> {
        None
    }
}

/// An entity whose statements may be built under the context `Cx`: every
/// entity under every context holding the scopes it declares, and an
/// unscoped one under any. The `Entity` derive implements it, through a trait
/// of the entity's own whose message names the entity and its scopes when a
/// context does not hold them.
///
/// The plain statement forms bound their entity by `ScopedIn<NoContext>`, so
/// they are a scoped entity's compile errors, and each `_for` form by
/// `ScopedIn<Cx>` for the context it is given.
// [spec:pgorm:req:scope.compile]
pub trait ScopedIn<Cx: ScopeContext + ?Sized>: Entity {}

/// What [`UpdateMany::from`](crate::UpdateMany::from) and
/// [`DeleteMany::using`](crate::DeleteMany::using) add to a statement with no
/// context: an entity bounded by [`ScopedIn<NoContext>`](ScopedIn), so an
/// unscoped one, and pgorm-query's own items, a [`FromItem`], [`NamedTable`],
/// [`TableName`], [`Name`], [`TableSample`] or [`AliasName`], which name no
/// entity to pgorm and are added as they are.
///
/// A scoped entity is added by [`from_for`](crate::UpdateMany::from_for) or
/// [`using_for`](crate::DeleteMany::using_for), given a context holding its
/// scopes, so naming one in `from` or `using` is its compile error:
///
/// ```compile_fail,E0277
/// use pgorm::{entity::*, query::*, tests_cfg::scoped::{company, note, Member, CompanyId, UserId}};
///
/// let member = Member { company: CompanyId(7), user: UserId(3) };
/// let _ = Update::many_for(&member, company::Entity).from(note::Entity);
/// ```
// [spec:pgorm:sem:scope.from-using]
pub trait UnscopedFromItem: IntoFromItem {}

// [spec:pgorm:sem:scope.from-using]
impl<E: ScopedIn<NoContext>> UnscopedFromItem for E {}

impl UnscopedFromItem for FromItem {}

impl UnscopedFromItem for NamedTable {}

impl UnscopedFromItem for TableName {}

impl UnscopedFromItem for Name {}

impl UnscopedFromItem for TableSample {}

impl UnscopedFromItem for AliasName {}

/// What an entity declares about scopes apart from any context: the scopes,
/// and how a context's values become the condition on its rows and the
/// columns of a row it inserts. The `Entity` derive implements it for every
/// entity, an unscoped one with the defaults.
// [spec:pgorm:def:scope.declare+1]
pub trait EntityScopes {
    /// The scopes, or `None` for an unscoped entity.
    fn scopes() -> Option<Scopes> {
        None
    }

    /// Fill `row`'s scope columns from `cx`, or refuse a row that names a
    /// value `cx` does not hold, for an insert under `cx`.
    fn fill_scopes(
        cx: &dyn ScopeContext,
        row: &mut <Self as Entity>::ActiveModel,
    ) -> Result<(), ScopeError>
    where
        Self: Entity,
    {
        let _ = (cx, row);
        Ok(())
    }
}

/// The scopes an entity declares, as data a statement carries: their names,
/// for a refusal, and how a context's values become the condition on a row of
/// the entity read under a given name. A [`RelationDef`](crate::RelationDef)
/// carries its sides' scopes, so a join into a scoped table is scoped by the
/// context of the form that adds it, or refused.
// [spec:pgorm:def:scope.declare+1]
#[derive(Clone, Copy)]
pub struct Scopes {
    names: &'static [&'static str],
    condition: ScopeCondition,
    piped: PipedScope,
}

/// How a context's values become the condition on a row read under a name.
type ScopeCondition = fn(&dyn ScopeContext, Name) -> Result<Option<Condition>, ScopeError>;

/// How a context's values become a pipeline's filter on rows read under a
/// name, each value bound through the pipeline's binder.
type PipedScope = for<'brand> fn(
    &dyn ScopeContext,
    String,
    &mut Binder<'brand>,
) -> Result<Option<PipelineExpr<'brand>>, ScopeError>;

impl fmt::Debug for Scopes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scopes")
            .field("names", &self.names)
            .finish_non_exhaustive()
    }
}

impl Scopes {
    /// The scopes `names` lists, whose condition `condition` writes and whose
    /// pipeline filter `piped` writes: what the `Entity` derive writes, a
    /// program declaring its scopes there.
    #[doc(hidden)]
    pub const fn new(
        names: &'static [&'static str],
        condition: ScopeCondition,
        piped: PipedScope,
    ) -> Self {
        Self {
            names,
            condition,
            piped,
        }
    }

    /// The scopes' names, as their types are written.
    pub fn names(&self) -> &'static [&'static str] {
        self.names
    }

    /// The condition `cx`'s scopes put on a row read under `table`, `None`
    /// where every scope is lifted, or the refusal of a context that does
    /// not hold one of them.
    // [spec:pgorm:sem:scope.read]
    pub(crate) fn condition(
        &self,
        cx: &dyn ScopeContext,
        table: Name,
    ) -> Result<Option<Condition>, ScopeError> {
        (self.condition)(cx, table)
    }

    /// The filter `cx`'s scopes put on a pipeline's rows read under
    /// `qualifier`, each value bound through `binder`.
    // [spec:pgorm:sem:scope.read]
    pub(crate) fn piped<'brand>(
        &self,
        cx: &dyn ScopeContext,
        qualifier: String,
        binder: &mut Binder<'brand>,
    ) -> Result<Option<PipelineExpr<'brand>>, ScopeError> {
        (self.piped)(cx, qualifier, binder)
    }

    /// The scopes' names as a refusal lists them: `` `A` `` or
    /// `` `A` and `B` ``.
    pub(crate) fn listed(&self) -> String {
        let names: Vec<String> = self.names.iter().map(|name| format!("`{name}`")).collect();
        match names.split_last() {
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            None => String::new(),
        }
    }
}

/// A scope written by hand: the condition a scope value puts on an entity's
/// rows, where that is not one column equal to the value, and how an insert
/// under it fills a row. An entity declares it, `#[pgorm(scope(UserId))]`,
/// and the program implements this for the scope's type.
///
/// A message, declared `#[pgorm(table_name = "message", scope(UserId))]`, is
/// its sender's and its recipient's to read, which `UserId`'s
/// `ScopeOn<message::Entity>` writes as the condition
/// `sender_id = u OR recipient_id = u`, its `insert` filling `sender_id`:
///
/// ```
/// use pgorm::{entity::*, query::*, tests_cfg::scoped::{message, UserId}};
///
/// assert_eq!(
///     message::Entity::find_for(&UserId(3)).as_query().to_string(),
///     [
///         r#"SELECT "message"."id", "message"."sender_id", "message"."recipient_id", "message"."body""#,
///         r#"FROM "message" WHERE "message"."sender_id" = 3 OR "message"."recipient_id" = 3"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// The condition is given the name the row is read under, which is the
/// table's own or the alias a join reads it under, and qualifies its columns
/// by it. Under an [`Every`] of the scope neither is called.
// [spec:pgorm:def:scope.written]
pub trait ScopeOn<E: Entity>: Scope {
    /// The condition this value puts on a row of `E` read under `table`.
    fn condition(&self, table: Name) -> Condition;

    /// Fill or check `row`, a row of `E` an insert under this value writes,
    /// refusing one it may not write.
    fn insert(&self, row: &mut E::ActiveModel) -> Result<(), ScopeError>;
}

/// Why a statement on a scoped entity was refused before anything was sent.
// [spec:pgorm:req:scope.refusals+1]
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScopeError {
    /// The statement's context does not hold a scope the table is scoped by:
    /// a table `join_for` or a graph's hop joins under a context that does
    /// not cover it, a target of a one-of load whose context, or lack of
    /// one, does not hold its scopes, or a context whose `ScopeContext` does
    /// not hold what its `Provides` say.
    #[error("`{table}` is scoped by `{scope}`, which the statement's context does not hold")]
    NotHeld {
        /// The scoped table.
        table: String,
        /// The scope the context does not hold.
        scope: &'static str,
    },
    /// A row an insert writes sets a scope column to a value other than the
    /// one the context holds.
    #[error("the row sets `{table}`.`{column}` to a value outside the context's `{scope}`")]
    Conflict {
        /// The scoped table.
        table: String,
        /// The column the row sets.
        column: String,
        /// The scope the column holds.
        scope: &'static str,
    },
    /// A row an insert writes leaves a scope column unset under a context
    /// holding every value of its scope, which leaves the row to set it.
    #[error(
        "the context holds every `{scope}`, so the row sets `{table}`.`{column}` itself, and it is not set"
    )]
    Unset {
        /// The scoped table.
        table: String,
        /// The column the row leaves unset.
        column: String,
        /// The scope the column holds.
        scope: &'static str,
    },
    /// A join into a scoped table added by a form taking no context, which
    /// cannot scope it.
    #[error(
        "`{table}` is scoped by {scopes}; join it with a form taking the statement's context, such as `join_for`"
    )]
    Unscoped {
        /// The scoped table.
        table: String,
        /// Its scopes, as a refusal lists them.
        scopes: String,
    },
    /// A pipeline over a table whose scope is written by hand, a
    /// [`ScopeOn`], under a context holding a value of it: the scope's
    /// condition is pgorm-query's, which a pipeline cannot read.
    #[error(
        "`{table}`'s `{scope}` scope is written as a pgorm-query condition, which a pipeline cannot read; read `{table}` with `find_for`"
    )]
    Unpiped {
        /// The scoped table.
        table: String,
        /// The scope written by hand.
        scope: &'static str,
    },
    /// A hand-written scope's refusal of a row, in its own words.
    #[error("{0}")]
    Refused(String),
}

/// `S`'s name as written: the last segment of its path.
fn scope_name<S>() -> &'static str {
    let name = std::any::type_name::<S>();
    let bare = name.split('<').next().unwrap_or(name);
    bare.rsplit("::").next().unwrap_or(bare)
}

/// How `cx` holds `S`, refused for `E`'s table where it does not.
pub(crate) fn held_by<'a, S: Scope, E: EntityName>(
    cx: &'a dyn ScopeContext,
    scope: &'static str,
) -> Result<Held<'a, S>, ScopeError> {
    match cx.held(TypeId::of::<S>()) {
        Some(Held::One(value)) => match value.downcast_ref::<S>() {
            Some(value) => Ok(Held::One(value)),
            None => Err(not_held::<E>(scope)),
        },
        Some(Held::Every) => Ok(Held::Every),
        None => Err(not_held::<E>(scope)),
    }
}

fn not_held<E: EntityName>(scope: &'static str) -> ScopeError {
    ScopeError::NotHeld {
        table: E::default().table_name().to_owned(),
        scope,
    }
}

/// The condition `cx`'s scopes put on a row of `E` written under `E`'s own
/// name, or the refusal of a context that does not hold one of them.
// [spec:pgorm:sem:scope.write]
pub(crate) fn scope_condition<E: Entity>(
    cx: &dyn ScopeContext,
) -> Result<Option<Condition>, ScopeError> {
    match E::scopes() {
        Some(scopes) => scopes.condition(cx, Name::new(E::default())),
        None => Ok(None),
    }
}

/// AND `cx`'s scopes on a row of `E` written under `E`'s own name into
/// `statement`'s `WHERE`, or hold the refusal in `refused` where none is held
/// yet, the first being the one reported: what a bulk update or delete does
/// for its own table and for an entity its `FROM` or `USING` adds.
// [spec:pgorm:sem:scope.write]
// [spec:pgorm:sem:scope.from-using]
pub(crate) fn hold_statement<E: Entity>(
    cx: &dyn ScopeContext,
    statement: &mut impl ConditionalStatement,
    refused: &mut Option<ScopeError>,
) {
    match scope_condition::<E>(cx) {
        Ok(Some(condition)) => {
            statement.cond_where(condition);
        }
        Ok(None) => {}
        Err(refusal) => {
            refused.get_or_insert(refusal);
        }
    }
}

/// The condition a context's scopes put on a row of `E` read under one name,
/// gathered one scope at a time: what the `Entity` derive's condition writes.
#[doc(hidden)]
pub struct ScopeConditions<'a, E> {
    cx: &'a dyn ScopeContext,
    table: Name,
    condition: Option<Condition>,
    entity: PhantomData<fn() -> E>,
}

impl<E> fmt::Debug for ScopeConditions<'_, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeConditions")
            .field("condition", &self.condition)
            .finish_non_exhaustive()
    }
}

// [spec:pgorm:sem:scope.read]
impl<'a, E: Entity> ScopeConditions<'a, E> {
    /// No condition yet, for rows read under `table`.
    pub fn new(cx: &'a dyn ScopeContext, table: Name) -> Self {
        Self {
            cx,
            table,
            condition: None,
            entity: PhantomData,
        }
    }

    fn and(mut self, condition: Condition) -> Self {
        self.condition = Some(match self.condition.take() {
            Some(all) => all.add(condition),
            None => Condition::all().add(condition),
        });
        self
    }

    /// `column = value` for the value of `S` the context holds, compared
    /// under the column's casts as [`Column::eq`] compares it.
    pub fn column<S, C>(self, scope: &'static str, column: C) -> Result<Self, ScopeError>
    where
        S: Scope + Into<Value>,
        C: Column,
    {
        match held_by::<S, E>(self.cx, scope)? {
            Held::One(value) => {
                let qualified = compared_column(&column, Expr::col((self.table.clone(), column)));
                let compared = compared_value(&column, Expr::val(value.clone().into()));
                Ok(self.and(Condition::all().add(qualified.eq(compared))))
            }
            Held::Every => Ok(self),
        }
    }

    /// The condition [`ScopeOn::condition`] writes for the value of `S` the
    /// context holds.
    pub fn written<S: ScopeOn<E>>(self, scope: &'static str) -> Result<Self, ScopeError> {
        match held_by::<S, E>(self.cx, scope)? {
            Held::One(value) => {
                let condition = value.condition(self.table.clone());
                Ok(self.and(condition))
            }
            Held::Every => Ok(self),
        }
    }

    /// The condition gathered, `None` where every scope is lifted.
    pub fn finish(self) -> Option<Condition> {
        self.condition
    }
}

/// A row an insert under a context writes, its scope columns filled one scope
/// at a time: what the `Entity` derive's fill writes.
#[doc(hidden)]
pub struct ScopeFill<'a, E: Entity> {
    cx: &'a dyn ScopeContext,
    row: &'a mut E::ActiveModel,
}

impl<E: Entity> fmt::Debug for ScopeFill<'_, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeFill")
            .field("row", &self.row)
            .finish_non_exhaustive()
    }
}

// [spec:pgorm:sem:scope.insert]
impl<'a, E: Entity> ScopeFill<'a, E> {
    /// Nothing filled yet.
    pub fn new(cx: &'a dyn ScopeContext, row: &'a mut E::ActiveModel) -> Self {
        Self { cx, row }
    }

    /// Set `column` to the value of `S` the context holds where the row
    /// leaves it unset, and refuse a row setting it to another; under every
    /// value of `S`, refuse a row leaving it unset.
    pub fn column<S>(self, scope: &'static str, column: E::Column) -> Result<Self, ScopeError>
    where
        S: Scope + Into<Value>,
    {
        let table = || E::default().table_name().to_owned();
        let name = || column.as_str().to_owned();
        let current = self
            .row
            .get(column)
            .map_err(|error| ScopeError::Refused(error.to_string()))?;
        match (held_by::<S, E>(self.cx, scope)?, current) {
            (Held::One(value), ActiveValue::NotSet) => {
                self.row
                    .set(column, value.clone().into())
                    .map_err(|error| ScopeError::Refused(error.to_string()))?;
            }
            (Held::One(value), ActiveValue::Set(set) | ActiveValue::Unchanged(set)) => {
                if set != value.clone().into() {
                    return Err(ScopeError::Conflict {
                        table: table(),
                        column: name(),
                        scope,
                    });
                }
            }
            (Held::Every, ActiveValue::NotSet) => {
                return Err(ScopeError::Unset {
                    table: table(),
                    column: name(),
                    scope,
                });
            }
            (Held::Every, ActiveValue::Set(_) | ActiveValue::Unchanged(_)) => {}
        }
        Ok(self)
    }

    /// Fill the row through [`ScopeOn::insert`] for the value of `S` the
    /// context holds; under every value of `S`, leave it as it is.
    pub fn written<S: ScopeOn<E>>(self, scope: &'static str) -> Result<Self, ScopeError> {
        match held_by::<S, E>(self.cx, scope)? {
            Held::One(value) => value.insert(self.row)?,
            Held::Every => {}
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Tenant(i64);
    impl Scope for Tenant {}

    #[test]
    fn a_scope_value_holds_itself_alone() {
        let tenant = Tenant(7);
        assert!(matches!(
            tenant.held(TypeId::of::<Tenant>()),
            Some(Held::One(_))
        ));
        assert!(tenant.held(TypeId::of::<i64>()).is_none());
        assert!(matches!(
            Every::<Tenant>::new().held(TypeId::of::<Tenant>()),
            Some(Held::Every)
        ));
        assert!(NoContext.held(TypeId::of::<Tenant>()).is_none());
        assert_eq!(format!("{:?}", Every::<Tenant>::new()), "Every<Tenant>");
    }
}
