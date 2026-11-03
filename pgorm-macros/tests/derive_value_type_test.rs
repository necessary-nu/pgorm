//! Verification for `ValueType`.
//!
//! Note on coverage of the shared Rust-type -> `ColumnType` table: the `char`
//! and `u64` rows cannot be reached through this derive (nor through
//! the `Entity` derive), because the generated `TryGetable` impl delegates to
//! `<inner as TryGetable>` and neither type implements it.

#![allow(dead_code)]

use pgorm::pgorm_query::{ArrayType, ColumnType, StringLen, ValueType};
use pgorm::{Value, ValueType};

#[test]
fn when_user_import_nothing_macro_still_works_test() {
    #[derive(pgorm::ValueType)]
    struct MyString(String);
}

#[test]
fn when_user_alias_result_macro_still_works_test() {
    #[allow(dead_code)]
    type Result<T> = std::result::Result<T, ()>;
    #[derive(pgorm::ValueType)]
    struct MyString(String);
}

#[derive(ValueType, Debug, PartialEq)]
struct MyString(String);

#[derive(ValueType, Debug, PartialEq)]
struct MyInt(i32);

#[derive(ValueType, Debug, PartialEq)]
struct MyBig(i64);

/// `Option<T>` is unwrapped to `T` before the table lookup.
#[derive(ValueType, Debug, PartialEq)]
struct MyOptional(Option<i64>);

/// Both overrides at once.
#[derive(ValueType, Debug, PartialEq)]
#[pgorm(column_type = ColumnType::Text, array_type = ArrayType::String)]
struct MyText(String);

/// A `#[pgorm(...)]` key the derive does not read is a spanned compile error;
/// see `tests/compile-fail/value_type_unknown_attribute.rs`. With no attribute
/// at all the inference stands on its own.
#[derive(ValueType, Debug, PartialEq)]
struct MyPlain(i32);

#[derive(Debug, Clone, PartialEq, Eq, pgorm::ActiveEnum)]
#[pgorm(rs_type = String, db_type = ColumnType::String(StringLen::N(1)))]
enum Tea {
    #[pgorm(string_value = "E")]
    Everyday,
}

/// A type outside the table falls through to `<T as ValueType>::column_type()`
/// and `array_type()`.
#[derive(ValueType, Debug, PartialEq)]
struct MyTea(Tea);

// [spec:pgorm:sem:macros.derive.value-type+6/test]    the inferred ColumnType / ArrayType
#[test]
fn column_and_array_types_inferred_from_inner_type() {
    assert_eq!(MyString::column_type(), ColumnType::string(None));
    assert_eq!(MyString::array_type(), ArrayType::String);

    assert_eq!(MyInt::column_type(), ColumnType::Integer);
    assert_eq!(MyInt::array_type(), ArrayType::Int);

    assert_eq!(MyBig::column_type(), ColumnType::BigInteger);
    assert_eq!(MyBig::array_type(), ArrayType::BigInt);

    // `Option<i64>` resolves as `i64`.
    assert_eq!(MyOptional::column_type(), ColumnType::BigInteger);
    assert_eq!(MyOptional::array_type(), ArrayType::BigInt);

    // Anything not in the table delegates to the inner `ValueType`.
    assert_eq!(MyTea::column_type(), <Tea as ValueType>::column_type());
    assert_eq!(MyTea::column_type(), ColumnType::String(StringLen::N(1)));
    assert_eq!(MyTea::array_type(), <Tea as ValueType>::array_type());
}

// [spec:pgorm:sem:macros.derive.value-type+6/test]    the attribute overrides
#[test]
fn column_type_and_array_type_attributes_override() {
    // `String` would infer `string(None)` / `ArrayType::String`.
    assert_eq!(MyText::column_type(), ColumnType::Text);
    assert_eq!(MyText::array_type(), ArrayType::String);

    // Without an override, the inference stands.
    assert_eq!(MyPlain::column_type(), ColumnType::Integer);
    assert_eq!(MyPlain::array_type(), ArrayType::Int);
}

// [spec:pgorm:sem:macros.derive.value-type+6/test]    From<T> for Value, ValueType, TryGetable
#[test]
fn the_three_generated_impls() {
    // `From<T> for Value` goes through `self.0`.
    assert_eq!(Value::from(MyInt(5)), Value::Int(Some(5)));
    assert_eq!(
        Value::from(MyString("hi".to_owned())),
        Value::String(Some(Box::new("hi".to_owned())))
    );

    // `ValueType` delegates to the inner type...
    assert_eq!(
        <MyInt as ValueType>::try_from(Value::Int(Some(5))).unwrap(),
        MyInt(5)
    );
    assert!(<MyInt as ValueType>::try_from(Value::Bool(Some(true))).is_err());
    // ...and reports the *newtype's* name.
    assert_eq!(MyInt::type_name(), "MyInt");
    assert_eq!(MyString::type_name(), "MyString");
    assert_eq!(MyText::type_name(), "MyText");

    // `TryGetable` is generated too.
    fn assert_try_getable<T: pgorm::TryGetable>() {}
    assert_try_getable::<MyInt>();
    assert_try_getable::<MyString>();
    assert_try_getable::<MyTea>();
}
