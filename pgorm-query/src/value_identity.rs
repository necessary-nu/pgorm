//! Identity for [`Value`]: `PartialEq`, `Eq` and `Hash` agree bit-for-bit.
//!
//! Split from `value.rs` so the identity contract reads as one unit: the
//! float, double and vector variants compare and hash by IEEE bit pattern,
//! which is what makes the blanket `Eq` lawful.

use crate::value::Value;
use std::hash::Hash;

fn eq_f32(a: Option<f32>, b: Option<f32>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.to_bits() == b.to_bits(),
        (None, None) => true,
        _ => false,
    }
}

fn eq_f64(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.to_bits() == b.to_bits(),
        (None, None) => true,
        _ => false,
    }
}

fn eq_vector(a: Option<&pgvector::Vector>, b: Option<&pgvector::Vector>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (a, b) = (a.as_slice(), b.as_slice());
            a.len() == b.len() && std::iter::zip(a, b).all(|(a, b)| a.to_bits() == b.to_bits())
        }
        (None, None) => true,
        _ => false,
    }
}

// [spec:pgorm:def:sql.value+4]
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::TinyInt(a), Self::TinyInt(b)) => a == b,
            (Self::SmallInt(a), Self::SmallInt(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::BigInt(a), Self::BigInt(b)) => a == b,
            (Self::Unsigned(a), Self::Unsigned(b)) => a == b,
            (Self::BigUnsigned(a), Self::BigUnsigned(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => eq_f32(*a, *b),
            (Self::Double(a), Self::Double(b)) => eq_f64(*a, *b),
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Char(a), Self::Char(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::Json(a), Self::Json(b)) => a == b,
            (Self::Date(a), Self::Date(b)) => a == b,
            (Self::Time(a), Self::Time(b)) => a == b,
            (Self::DateTime(a), Self::DateTime(b)) => a == b,
            (Self::DateTimeWithTimeZone(a), Self::DateTimeWithTimeZone(b)) => a == b,
            (Self::Uuid(a), Self::Uuid(b)) => a == b,
            (Self::Decimal(a), Self::Decimal(b)) => a == b,
            // Elements compare through this same impl, so a float inside an
            // array is compared bitwise too.
            (Self::Array(a_ty, a), Self::Array(b_ty, b)) => a_ty == b_ty && a == b,
            (Self::Vector(a), Self::Vector(b)) => eq_vector(a.as_deref(), b.as_deref()),
            (Self::IpNetwork(a), Self::IpNetwork(b)) => a == b,
            (Self::MacAddress(a), Self::MacAddress(b)) => a == b,
            (Self::Interval(a), Self::Interval(b)) => a == b,
            // Bounds compare through this same impl, as array elements do.
            (Self::Range(a_ty, a), Self::Range(b_ty, b)) => a_ty == b_ty && a == b,
            (Self::Multirange(a_ty, a), Self::Multirange(b_ty, b)) => a_ty == b_ty && a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

fn hash_f32<H: std::hash::Hasher>(value: Option<f32>, state: &mut H) {
    match value {
        Some(value) => value.to_bits().hash(state),
        None => state.write_u8(0),
    }
}

fn hash_f64<H: std::hash::Hasher>(value: Option<f64>, state: &mut H) {
    match value {
        Some(value) => value.to_bits().hash(state),
        None => state.write_u8(0),
    }
}

fn hash_vector<H: std::hash::Hasher>(value: Option<&pgvector::Vector>, state: &mut H) {
    match value {
        Some(value) => {
            let slice = value.as_slice();
            slice.len().hash(state);
            for element in slice {
                element.to_bits().hash(state);
            }
        }
        None => state.write_u8(0),
    }
}

// [spec:pgorm:def:sql.value+4]
impl Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Value::Bool(value) => value.hash(state),
            Value::TinyInt(value) => value.hash(state),
            Value::SmallInt(value) => value.hash(state),
            Value::Int(value) => value.hash(state),
            Value::BigInt(value) => value.hash(state),
            Value::Unsigned(value) => value.hash(state),
            Value::BigUnsigned(value) => value.hash(state),
            Value::Float(value) => hash_f32(*value, state),
            Value::Double(value) => hash_f64(*value, state),
            Value::String(value) => value.hash(state),
            Value::Char(value) => value.hash(state),
            Value::Bytes(value) => value.hash(state),
            Value::Json(value) => value.hash(state),
            Value::Date(value) => value.hash(state),
            Value::Time(value) => value.hash(state),
            Value::DateTime(value) => value.hash(state),
            Value::DateTimeWithTimeZone(value) => value.hash(state),
            Value::Uuid(value) => value.hash(state),
            Value::Decimal(value) => value.hash(state),
            Value::Array(_, value) => value.hash(state),
            Value::Vector(value) => hash_vector(value.as_deref(), state),
            Value::IpNetwork(value) => value.hash(state),
            Value::MacAddress(value) => value.hash(state),
            Value::Interval(value) => value.hash(state),
            Value::Range(_, value) => value.hash(state),
            Value::Multirange(_, value) => value.hash(state),
        }
    }
}
