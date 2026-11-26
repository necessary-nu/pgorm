//! What converts into an `ActiveModel`, and into one of its values.

use crate::{ActiveModel, ActiveValue, NotSet, Value};
use pgorm_query::Nullable;

/// What an `ActiveModel` `A` is made from: `A` itself, and a form struct in
/// any module or crate that `#[derive(IntoActiveModel)]` implements it on.
// [spec:pgorm:req:entity.active-model.into+4]
pub trait IntoActiveModel<A>
where
    A: ActiveModel,
{
    /// The `ActiveModel` this value sets, a field it leaves out `NotSet`.
    fn into_active_model(self) -> A;
}

impl<A> IntoActiveModel<A> for A
where
    A: ActiveModel,
{
    fn into_active_model(self) -> A {
        self
    }
}

/// What a form's field is, as the [`ActiveValue`] of the column type `V` it
/// sets: the built-in column types and the value types the derives make, each
/// setting its column, and an `Option` of one, whose `None` leaves the column
/// `NotSet`.
// [spec:pgorm:req:entity.active-model.into+4]
pub trait IntoActiveValue<V>
where
    V: Into<Value>,
{
    /// The column's value: `Set`, or `NotSet` for a `None` the form left out.
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
