//! The columns a lookup keys on, and the value shape that lines up with them.

use crate::{Name, SqlName, StaticName, Value, ValueTuple, value::IntoValueTuple};
use std::fmt;

/// The columns a lookup keys on, in declared order.
///
/// One arity-agnostic representation, whether the key is a single column or a
/// composite: every consumer walks the columns rather than dispatching on how
/// many there are, and a column set of a given width has exactly one spelling.
// [spec:pgorm:def:entity.relation.def+12]
#[derive(Debug, Clone)]
pub struct Key(Vec<Name>);

impl Key {
    /// The number of columns.
    // [spec:pgorm:def:entity.relation.def+12]
    pub fn arity(&self) -> usize {
        self.0.len()
    }

    /// Iterate the columns in declared order.
    // [spec:pgorm:def:entity.relation.def+12]
    pub fn iter(&self) -> impl Iterator<Item = &Name> {
        self.0.iter()
    }

    /// The one column of a unary set, or `None` when the set is wider: what a
    /// consumer that can only act on one column asks, instead of dispatching on
    /// arity.
    // [spec:pgorm:def:entity.relation.def+12]
    pub fn single(&self) -> Option<&Name> {
        match self.0.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }
}

// [spec:pgorm:def:entity.relation.def+12]
impl From<Name> for Key {
    fn from(name: Name) -> Self {
        Self(vec![name])
    }
}

// [spec:pgorm:def:entity.relation.def+12]
impl From<Vec<Name>> for Key {
    fn from(names: Vec<Name>) -> Self {
        Self(names)
    }
}

// [spec:pgorm:def:entity.relation.def+12]
impl FromIterator<Name> for Key {
    fn from_iter<I: IntoIterator<Item = Name>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for Key {
    type Item = Name;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl SqlName for Key {
    fn unquoted(&self, s: &mut dyn fmt::Write) {
        for name in self.iter() {
            write!(s, "{}", name.to_string()).expect("write to sql sink");
        }
    }
}

/// Performs a conversion into a [`Key`]
// [spec:pgorm:def:entity.relation.def+12]
pub trait IntoKey {
    /// The shape a boundary value must have to line up with this key: a
    /// tuple of [`Value`] of the same length, so the arity of a column set and
    /// the arity of the values compared against it are one fact rather than
    /// two. [`Key`] itself, whose arity is only known at runtime, maps to
    /// [`ValueTuple`].
    type ValueType: IntoValueTuple;

    /// Method to perform the conversion
    fn into_key(self) -> Key;
}

/// A value tuple whose arity matches the order-key shape `K`.
///
/// `K` is the [`IntoKey::ValueType`] of a column set, so a tuple of the
/// wrong length has no implementation here and is rejected at compile time.
/// The exception is `K = ValueTuple`, the shape of a runtime-built
/// [`Key`], which accepts any tuple and leaves the arity to be checked
/// when the query runs.
// [spec:pgorm:def:entity.relation.def+12]
pub trait IntoBoundary<K>: IntoValueTuple {}

impl<T> IntoBoundary<ValueTuple> for T where T: IntoValueTuple {}

impl IntoKey for Key {
    type ValueType = ValueTuple;

    fn into_key(self) -> Key {
        self
    }
}

impl<V> IntoBoundary<Value> for V where V: Into<Value> {}

/// A name minted at run time keys a lookup the same as one the program
/// spells. There is deliberately no `impl` for `&str` or `String`: text
/// becomes a name at [`Name::runtime`] and nowhere else, so the conversion
/// stays greppable here too.
impl IntoKey for Name {
    type ValueType = Value;

    fn into_key(self) -> Key {
        Key::from(self)
    }
}

impl<T> IntoKey for T
where
    T: StaticName,
{
    type ValueType = Value;

    fn into_key(self) -> Key {
        Key::from(Name::new(self))
    }
}

/// Expands to [`Value`] once per type parameter of a tuple impl, so the
/// boundary shape is built from the same repetition as the tuple itself.
macro_rules! boundary_element {
    ( $T:ident ) => {
        Value
    };
}

macro_rules! impl_into_key {
    ( $($T:ident : $N:tt),+ $(,)? ) => {
        impl< $($T),+ > IntoKey for ( $($T),+ )
        where
            $($T: StaticName),+
        {
            type ValueType = ( $(boundary_element!($T)),+ );

            fn into_key(self) -> Key {
                Key::from(vec![
                    $(Name::new(self.$N)),+
                ])
            }
        }

        impl< $($T),+ > IntoBoundary<( $(boundary_element!($T)),+ )> for ( $($T),+ )
        where
            $($T: Into<Value>),+
        {
        }
    };
}

#[rustfmt::skip]
mod impl_into_key {
    use super::*;

    impl_into_key!(T0:0, T1:1);
    impl_into_key!(T0:0, T1:1, T2:2);
    impl_into_key!(T0:0, T1:1, T2:2, T3:3);
    impl_into_key!(T0:0, T1:1, T2:2, T3:3, T4:4);
    impl_into_key!(T0:0, T1:1, T2:2, T3:3, T4:4, T5:5);
    impl_into_key!(T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6);
    impl_into_key!(T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7);
    impl_into_key!(T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8);
    impl_into_key!(T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8, T9:9);
    impl_into_key!(T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8, T9:9, T10:10);
    impl_into_key!(T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8, T9:9, T10:10, T11:11);
}
