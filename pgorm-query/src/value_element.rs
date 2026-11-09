//! What an array parameter's element type is read from.

use crate::{ArrayType, Value, ValueType};

/// A Rust type whose values make the elements of one array parameter: what
/// [`Value::array`] reads the array's element type from, so an empty list is
/// still a typed array.
///
/// Every [`ValueType`] is one, and so are the borrowed spellings that convert
/// into a [`Value`] but cannot be read back out of one: `&str` and `&String`
/// are elements of a `text[]`, and `&[u8]` of a `bytea[]`. That is what lets
/// `eq_any(["a", "b"])` bind its list as `eq("a")` binds its value.
///
/// The method is named apart from [`ValueType::array_type`], which every
/// `ValueType` element also has, so that the two traits in scope together
/// leave neither call ambiguous.
// [spec:pgorm:def:sql.value.array+7]
pub trait ArrayElement: Into<Value> {
    /// The tag an array of these values carries, answered with no value in
    /// hand, which is what an empty list has to go on.
    fn element_type() -> ArrayType;
}

impl<T> ArrayElement for T
where
    T: Into<Value> + ValueType,
{
    fn element_type() -> ArrayType {
        T::array_type()
    }
}

/// `ArrayElement` for a borrowed spelling of an owned element type.
macro_rules! borrowed_element {
    ( $type: ty, $tag: ident ) => {
        impl ArrayElement for $type {
            fn element_type() -> ArrayType {
                ArrayType::$tag
            }
        }
    };
}

borrowed_element!(&str, String);
borrowed_element!(&String, String);
borrowed_element!(&[u8], Bytes);
