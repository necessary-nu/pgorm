//! A registered entity's model and ActiveModel behind their type-erased
//! handles: the real `E::Model` and `E::ActiveModel`, each operation the
//! Rust API of its meaning.

use std::{any::Any, sync::Arc};

use futures_util::future::BoxFuture;
use pgorm::pgorm_query::Value;
use pgorm::{ActiveModel, ActiveValue, Entity, Error, IntoActiveModel, Model as _};

use super::{
    Active, ActiveBackend, EntityInfo, Model, ModelBackend, Write, Written, adapter::column,
    reach::Wrote,
};
use crate::{Database, Failure, scope::Scoping};

/// `value`, a model of the registration `info` describes, behind its
/// type-erased handle.
pub fn model_of<E>(value: E::Model, info: &Arc<EntityInfo>) -> Model
where
    E: Entity + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    Arc::new(ModelAdapter::<E> {
        value,
        info: Arc::clone(info),
    })
}

/// The compiled ActiveModel a type-erased handle holds, refused if another
/// registration made it.
pub(crate) fn concrete<E>(active: &Active) -> Result<E::ActiveModel, Error>
where
    E: Entity + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    active
        .as_any()
        .downcast_ref::<ActiveAdapter<E>>()
        .map(|adapter| adapter.value.clone())
        .ok_or_else(|| Error::Custom("the ActiveModel belongs to another registration".to_owned()))
}

#[derive(Debug)]
struct ModelAdapter<E: Entity> {
    value: E::Model,
    info: Arc<EntityInfo>,
}

// [spec:pgorm:req:bindings.entity-writes+2]
impl<E> ModelBackend for ModelAdapter<E>
where
    E: Entity + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn info(&self) -> &Arc<EntityInfo> {
        &self.info
    }

    fn get(&self, name: &str) -> Result<Value, Failure> {
        Ok(self.value.get(column::<E>(&self.info, name)?))
    }

    fn set(&self, name: &str, value: Value) -> Result<Model, Failure> {
        let mut model = self.value.clone();
        model
            .set(column::<E>(&self.info, name)?, value)
            .map_err(|error| Failure::Construction(error.to_string()))?;
        Ok(model_of::<E>(model, &self.info))
    }

    fn active(&self) -> Active {
        Arc::new(ActiveAdapter::<E> {
            value: self.value.clone().into_active_model(),
            info: self.info.clone(),
        })
    }
}

#[derive(Debug)]
pub(crate) struct ActiveAdapter<E: Entity> {
    pub(crate) value: E::ActiveModel,
    pub(crate) info: Arc<EntityInfo>,
}

impl<E> ActiveAdapter<E>
where
    E: Entity + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    /// A copy of the ActiveModel with `change` made to its column `name`.
    fn changed(
        &self,
        name: &str,
        change: impl FnOnce(&mut E::ActiveModel, E::Column) -> Result<(), Error>,
    ) -> Result<Active, Failure> {
        let column = column::<E>(&self.info, name)?;
        let mut value = self.value.clone();
        change(&mut value, column).map_err(|error| Failure::Construction(error.to_string()))?;
        Ok(Arc::new(Self {
            value,
            info: self.info.clone(),
        }))
    }
}

// [spec:pgorm:req:bindings.entity-writes+2]
impl<E> ActiveBackend for ActiveAdapter<E>
where
    E: Entity + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn info(&self) -> &Arc<EntityInfo> {
        &self.info
    }

    fn get(&self, name: &str) -> Result<ActiveValue<Value>, Failure> {
        self.value
            .get(column::<E>(&self.info, name)?)
            .map_err(|error| Failure::Construction(error.to_string()))
    }

    fn set(&self, name: &str, value: Value) -> Result<Active, Failure> {
        self.changed(name, |active, column| active.set(column, value))
    }

    fn not_set(&self, name: &str) -> Result<Active, Failure> {
        self.changed(name, |active, column| {
            active.not_set(column);
            Ok(())
        })
    }

    fn reset(&self, name: &str) -> Result<Active, Failure> {
        self.changed(name, |active, column| {
            active.reset(column);
            Ok(())
        })
    }

    fn set_if_not_equals(&self, name: &str, value: Value) -> Result<Active, Failure> {
        self.changed(name, |active, column| {
            active.set_if_not_equals(column, value)
        })
    }

    fn is_changed(&self) -> bool {
        self.value.is_changed()
    }

    fn run<'a>(
        &'a self,
        db: Database<'a>,
        write: Write,
        cx: &'a Scoping,
    ) -> BoxFuture<'a, Result<Written, Error>> {
        Box::pin(async move {
            let reach = self.info.reach::<E>()?;
            Ok(
                match reach.write(cx, db, write, self.value.clone()).await? {
                    Wrote::Model(model) => Written::Model(model_of::<E>(model, &self.info)),
                    Wrote::Count(count) => Written::Count(count),
                },
            )
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
