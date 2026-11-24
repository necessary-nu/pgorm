//! What converts into an `ActiveModel`, and into one of its values.

use crate::{ActiveModelTrait, ActiveValue, NotSet, Value};
use pgorm_query::Nullable;

/// A Trait for any type that can be converted into an ActiveModel
// [spec:pgorm:req:entity.active-model.into+3]
pub trait IntoActiveModel<A>
where
    A: ActiveModelTrait,
{
    /// Method to call to perform the conversion
    fn into_active_model(self) -> A;
}

impl<A> IntoActiveModel<A> for A
where
    A: ActiveModelTrait,
{
    fn into_active_model(self) -> A {
        self
    }
}

/// Any type that can be converted into an [ActiveValue]
// [spec:pgorm:req:entity.active-model.into+3]
pub trait IntoActiveValue<V>
where
    V: Into<Value>,
{
    /// Method to perform the conversion
    fn into_active_value(self) -> ActiveValue<V>;
}

impl<V> IntoActiveValue<Option<V>> for Option<V>
where
    V: IntoActiveValue<V> + Into<Value> + Nullable,
{
    fn into_active_value(self) -> ActiveValue<Option<V>> {
        match self {
            Some(value) => ActiveValue::Set(Some(value)),
            None => NotSet,
        }
    }
}

impl<V> IntoActiveValue<Option<V>> for Option<Option<V>>
where
    V: IntoActiveValue<V> + Into<Value> + Nullable,
{
    fn into_active_value(self) -> ActiveValue<Option<V>> {
        match self {
            Some(value) => ActiveValue::Set(value),
            None => NotSet,
        }
    }
}
