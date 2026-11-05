#![allow(unused_imports, dead_code)]

pub mod common;

use pgorm::{IntoActiveValue, TryGetable, Value};

/*

When supporting a new type in pgorm we should implement the following traits for it:
  - `IntoActiveValue`, given that it implemented `Into<Value>` already
  - `TryGetable`

Also, we need to update `impl FromQueryResult for JsonValue` at `src/query/json.rs`
to correctly serialize the type as `serde_json::Value`.

*/

pub fn it_impl_into_active_value<T: IntoActiveValue<V>, V: Into<Value>>() {}

pub fn it_impl_try_getable<T: TryGetable>() {}

#[allow(unused_macros)]
macro_rules! it_impl_traits {
    ( $ty: ty ) => {
        it_impl_into_active_value::<$ty, $ty>();
        it_impl_into_active_value::<Option<$ty>, Option<$ty>>();
        it_impl_into_active_value::<Option<Option<$ty>>, Option<$ty>>();

        it_impl_try_getable::<$ty>();
        it_impl_try_getable::<Option<$ty>>();
    };
}

#[pgorm_macros::test]
fn main() {
    it_impl_traits!(i8);
    it_impl_traits!(i16);
    it_impl_traits!(i32);
    it_impl_traits!(i64);
    it_impl_traits!(u32);
    it_impl_traits!(bool);
    it_impl_traits!(f32);
    it_impl_traits!(f64);
    it_impl_traits!(Vec<u8>);
    it_impl_traits!(String);
    it_impl_traits!(serde_json::Value);
    it_impl_traits!(jiff::civil::Date);
    it_impl_traits!(jiff::civil::Time);
    it_impl_traits!(jiff::civil::DateTime);
    it_impl_traits!(jiff::Timestamp);
    it_impl_traits!(rust_decimal::Decimal);
    it_impl_traits!(uuid::Uuid);
}
