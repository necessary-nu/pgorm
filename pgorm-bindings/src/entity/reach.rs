//! How a registration reaches its entity's statements: through the `_in`
//! forms of the instance it registered, or, for an entity registered by its
//! type, through the `_for` forms under a [`Scoping`], so a scoped entity's
//! statements carry its scopes as Rust's do.

use std::fmt;

use futures_util::future::BoxFuture;
use pgorm::{
    ActiveModel, ActiveModelBehavior, Entity, Error, Insert, IntoActiveModel, ManyRows, NoContext,
    OneRow, ScopedIn, Update, UpdateMany, UpdateOne, pipeline,
};

use super::Write;
use crate::{Database, scope::Scoping};

/// What an ActiveModel's write gave: the model written, or a delete's count.
pub(crate) enum Wrote<M> {
    /// The model an insert or update wrote.
    Model(M),
    /// How many rows a delete removed.
    Count(u64),
}

/// The statements of one registration's entity `E`, each built as the
/// registration reaches it.
// [spec:pgorm:req:bindings.scopes-registered]
pub(crate) trait Reach<E: Entity>: fmt::Debug + Send + Sync + 'static {
    /// The entity value whose names the registration's schema reads.
    fn entity(&self) -> E;
    fn find(&self, cx: &Scoping) -> pgorm::Select<E>;
    fn insert_one(&self, cx: &Scoping, model: E::ActiveModel) -> Insert<E::ActiveModel, OneRow>;
    fn insert_many(
        &self,
        cx: &Scoping,
        models: Vec<E::ActiveModel>,
    ) -> Insert<E::ActiveModel, ManyRows>;
    fn update_one(
        &self,
        cx: &Scoping,
        model: E::ActiveModel,
    ) -> Result<UpdateOne<E::ActiveModel>, Error>;
    fn update_many(&self, cx: &Scoping) -> UpdateMany<E>;
    /// The pipeline reading the entity's rows: `Pipeline::from_for`, its
    /// scopes a filter stage, or the instance's table.
    fn pipeline(&self, cx: &Scoping) -> pipeline::Pipeline;
    /// The entity as another pipeline's source: the instance's table, or a
    /// scoped entity's pipeline embedded, as Rust joins one.
    fn source(&self, cx: &Scoping) -> pipeline::Source;
    /// The ActiveModel's write `write` names through its hooks:
    /// `insert_for`, `update_for` and `delete_for`, or their `_in` forms.
    fn write<'a>(
        &'a self,
        cx: &'a Scoping,
        db: Database<'a>,
        write: Write,
        active: E::ActiveModel,
    ) -> BoxFuture<'a, Result<Wrote<E::Model>, Error>>;
}

/// An instance whose names may be computed at run time, reached through the
/// `_in` forms, which take no context: an unscoped entity's alone.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Instance<E>(pub(crate) E);

impl<E> Reach<E> for Instance<E>
where
    E: Entity + ScopedIn<NoContext> + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync,
    E::ActiveModel: Send + Sync,
{
    fn entity(&self) -> E {
        self.0
    }

    fn find(&self, _: &Scoping) -> pgorm::Select<E> {
        E::find_in(self.0)
    }

    fn insert_one(&self, _: &Scoping, model: E::ActiveModel) -> Insert<E::ActiveModel, OneRow> {
        Insert::one_in(self.0, model)
    }

    fn insert_many(
        &self,
        _: &Scoping,
        models: Vec<E::ActiveModel>,
    ) -> Insert<E::ActiveModel, ManyRows> {
        Insert::many_in(self.0, models)
    }

    fn update_one(
        &self,
        _: &Scoping,
        model: E::ActiveModel,
    ) -> Result<UpdateOne<E::ActiveModel>, Error> {
        Update::one_in(self.0, model)
    }

    fn update_many(&self, _: &Scoping) -> UpdateMany<E> {
        Update::many(self.0)
    }

    fn pipeline(&self, _: &Scoping) -> pipeline::Pipeline {
        pipeline::Pipeline::from(self.0)
    }

    fn source(&self, _: &Scoping) -> pipeline::Source {
        pipeline::IntoSource::into_source(self.0)
    }

    fn write<'a>(
        &'a self,
        _: &'a Scoping,
        db: Database<'a>,
        write: Write,
        active: E::ActiveModel,
    ) -> BoxFuture<'a, Result<Wrote<E::Model>, Error>> {
        let entity = self.0;
        Box::pin(async move {
            Ok(match write {
                Write::Insert => Wrote::Model(active.insert_in(entity, &db).await?),
                Write::Update => Wrote::Model(active.update_in(entity, &db).await?),
                Write::Delete => Wrote::Count(active.delete_in(entity, &db).await?),
            })
        })
    }
}

/// An entity registered by its type, its `Default` names read, reached
/// through the `_for` forms under the context its statement was given,
/// which holds no scope for a statement given none.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Typed;

impl<E> Reach<E> for Typed
where
    E: Entity + ScopedIn<Scoping> + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync,
    E::ActiveModel: ActiveModelBehavior + Send + Sync,
{
    fn entity(&self) -> E {
        E::default()
    }

    fn find(&self, cx: &Scoping) -> pgorm::Select<E> {
        E::find_for(cx)
    }

    fn insert_one(&self, cx: &Scoping, model: E::ActiveModel) -> Insert<E::ActiveModel, OneRow> {
        Insert::one_for(cx, model)
    }

    fn insert_many(
        &self,
        cx: &Scoping,
        models: Vec<E::ActiveModel>,
    ) -> Insert<E::ActiveModel, ManyRows> {
        Insert::many_for(cx, models)
    }

    fn update_one(
        &self,
        cx: &Scoping,
        model: E::ActiveModel,
    ) -> Result<UpdateOne<E::ActiveModel>, Error> {
        Update::one_for(cx, model)
    }

    fn update_many(&self, cx: &Scoping) -> UpdateMany<E> {
        Update::many_for(cx, E::default())
    }

    fn pipeline(&self, cx: &Scoping) -> pipeline::Pipeline {
        pipeline::Pipeline::from_for(cx, E::default())
    }

    fn source(&self, cx: &Scoping) -> pipeline::Source {
        pipeline::IntoSource::into_source(Reach::<E>::pipeline(self, cx))
    }

    fn write<'a>(
        &'a self,
        cx: &'a Scoping,
        db: Database<'a>,
        write: Write,
        active: E::ActiveModel,
    ) -> BoxFuture<'a, Result<Wrote<E::Model>, Error>> {
        Box::pin(async move {
            Ok(match write {
                Write::Insert => Wrote::Model(active.insert_for(cx, &db).await?),
                Write::Update => Wrote::Model(active.update_for(cx, &db).await?),
                Write::Delete => Wrote::Count(active.delete_for(cx, &db).await?),
            })
        })
    }
}
