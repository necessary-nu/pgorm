//! A scoped entity's scopes as a pipeline filter.

use std::fmt;
use std::marker::PhantomData;

use pgorm_query::Value;

use pgorm_query::{Name, SqlName};

use super::adapter;
use super::binder::Binder;
use super::builder::{Pipeline, Source, table_source};
use super::expr::{Expr, ExprOps, entity_column};
use crate::entity::{default_alias, held_by};
use crate::{ColumnTrait, EntityTrait, Held, Scope, ScopeContext, ScopeError, ScopeOn, ScopedIn};

/// The table `entity` names as a source, held to no scope.
// [spec:pgorm:sem:pipeline.qualify+4]
pub(super) fn entity_source<E: EntityTrait>(entity: &E) -> Source {
    let mut source = table_source(match entity.schema_name() {
        Some(schema) => adapter::ident_in(vec![schema.to_owned()], entity.table_name().to_owned()),
        None => adapter::ident(entity.table_name()),
    });
    source.alias = default_alias(entity).map(|alias| SqlName::to_string(&*alias));
    source
}

impl Pipeline {
    /// [`from`](Pipeline::from) a scoped entity under the context `cx`: the
    /// table `entity` names, filtered by each scope `cx` holds the value of,
    /// as `find_for` reads it. A scope written by hand, a [`ScopeOn`](crate::ScopeOn),
    /// is a pgorm-query condition the pipeline cannot read, so a context
    /// holding a value of one is refused with
    /// [`ScopeError::Unpiped`] by [`into_sql`](Pipeline::into_sql). Another
    /// scoped entity joins the pipeline as a pipeline of its own:
    /// `.join(side, Pipeline::from_for(&cx, other), on)`.
    ///
    /// ```
    /// use pgorm::pipeline::Pipeline;
    /// use pgorm::tests_cfg::scoped::{note, CompanyId, Member, UserId};
    ///
    /// let member = Member { company: CompanyId(7), user: UserId(3) };
    /// let (sql, values) = Pipeline::from_for(&member, note::Entity).into_sql()?;
    /// assert_eq!(
    ///     sql,
    ///     "SELECT * FROM note WHERE company_id = $1 AND author_id = $2"
    /// );
    /// assert_eq!(values.0.len(), 2);
    /// # Ok::<_, pgorm::pipeline::PipelineError>(())
    /// ```
    // [spec:pgorm:req:pipeline.surface+4]
    // [spec:pgorm:sem:scope.read]
    pub fn from_for<Cx, E>(cx: &Cx, entity: E) -> Self
    where
        Cx: ScopeContext,
        E: ScopedIn<Cx>,
    {
        let mut pipeline = Self::from(entity_source(&entity));
        let Some(scopes) = E::scopes() else {
            return pipeline;
        };
        let qualifier = SqlName::to_string(&*Name::new(E::default()));
        match pipeline.bound(|binder| {
            scopes
                .piped(cx, qualifier, binder)
                .map(|filter| filter.map(|expr| expr.node))
        }) {
            Ok(Some(node)) => pipeline.stage(adapter::call("filter", vec![node])),
            Ok(None) => pipeline,
            Err(refusal) => {
                pipeline.refused = Some(refusal);
                pipeline
            }
        }
    }
}

/// The filter a context's scopes put on a pipeline over `E`, gathered one
/// scope at a time: what the `Entity` derive's pipeline filter writes.
#[doc(hidden)]
pub struct ScopeFilters<'a, 'brand, E> {
    cx: &'a dyn ScopeContext,
    qualifier: String,
    binder: &'a mut Binder<'brand>,
    filter: Option<Expr<'brand>>,
    entity: PhantomData<fn() -> E>,
}

impl<E> fmt::Debug for ScopeFilters<'_, '_, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeFilters")
            .field("qualifier", &self.qualifier)
            .finish_non_exhaustive()
    }
}

// [spec:pgorm:sem:scope.read]
impl<'a, 'brand, E: EntityTrait> ScopeFilters<'a, 'brand, E> {
    /// No filter yet, over `E` read under `qualifier`.
    pub fn new(
        cx: &'a dyn ScopeContext,
        qualifier: String,
        binder: &'a mut Binder<'brand>,
    ) -> Self {
        Self {
            cx,
            qualifier,
            binder,
            filter: None,
            entity: PhantomData,
        }
    }

    /// `column == $n`, `$n` bound to the value of `S` the context holds, the
    /// column compared as the entity types it.
    pub fn column<S, C>(mut self, scope: &'static str, column: C) -> Result<Self, ScopeError>
    where
        S: Scope + Into<Value>,
        C: ColumnTrait,
    {
        if let Held::One(value) = held_by::<S, E>(self.cx, scope)? {
            let bound = self.binder.bind(value.clone().into());
            let compared = entity_column(self.qualifier.clone(), &column).eq(bound);
            self.filter = Some(match self.filter.take() {
                Some(filter) => filter.and(compared),
                None => compared,
            });
        }
        Ok(self)
    }

    /// A scope [`ScopeOn`] writes, whose condition is pgorm-query's and has
    /// no pipeline form: refused where the context holds a value of it.
    pub fn written<S: ScopeOn<E>>(self, scope: &'static str) -> Result<Self, ScopeError> {
        match held_by::<S, E>(self.cx, scope)? {
            Held::One(_) => Err(ScopeError::Unpiped {
                table: E::default().table_name().to_owned(),
                scope,
            }),
            Held::Every => Ok(self),
        }
    }

    /// The filter gathered, `None` where every scope is lifted.
    pub fn finish(self) -> Option<Expr<'brand>> {
        self.filter
    }
}
