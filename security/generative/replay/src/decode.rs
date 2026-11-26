//! A result column, decoded into a tagged Rust value.
//!
//! The dispatch is keyed off the column's declared PostgreSQL type rather than
//! off the bytes, so `text` and an enum label — identical on the wire — reach
//! different tags, and an array's element type is recovered from `Kind::Array`
//! instead of guessed from its first element.

use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use pgorm::pgorm_query::{ArrayType, Interval, Value, Vector};
use pgorm_driver::{
    Row,
    types::{FromSql, Kind, Type},
};
use uuid::Uuid;

use crate::{
    FormatError,
    codecs::{CheckedArray, EnumLabel, ExactDecimal, ExactJson, Inet, Mac},
    wire::{Tagged, TypeName},
};

fn read<T: for<'a> FromSql<'a>>(row: &Row, index: usize) -> Result<Option<T>, FormatError> {
    row.try_get(index).map_err(|_| {
        FormatError::new(format!(
            "column {index} cannot be decoded without loss; its PostgreSQL type or value is unsupported"
        ))
    })
}

fn typed<T: for<'a> FromSql<'a>>(
    row: &Row,
    index: usize,
    array: bool,
    kind: ArrayType,
    convert: impl Fn(Option<T>) -> Value,
) -> Result<Value, FormatError> {
    if array {
        let values = read::<CheckedArray<T>>(row, index)?
            .map(|values| Box::new(values.0.into_iter().map(convert).collect()));
        Ok(Value::Array(kind, values))
    } else {
        read::<T>(row, index).map(convert)
    }
}

/// Decode one column of a row into its tagged value.
///
/// # Errors
///
/// Returns [`FormatError`] when the column's PostgreSQL type has no lossless
/// Rust representation, or when its value cannot survive the round trip.
pub(crate) fn value(row: &Row, index: usize) -> Result<Tagged, FormatError> {
    let declared = row.columns()[index].type_();
    let (ty, array) = match declared.kind() {
        Kind::Array(member) => (member, true),
        _ => (declared, false),
    };
    macro_rules! scalar {
        ($rust:ty, $variant:ident) => {
            typed::<$rust>(row, index, array, ArrayType::$variant, Value::$variant)
        };
    }
    macro_rules! boxed {
        ($rust:ty, $variant:ident) => {
            typed::<$rust>(row, index, array, ArrayType::$variant, |v| {
                Value::$variant(v.map(Box::new))
            })
        };
    }
    if matches!(ty.kind(), Kind::Enum(_)) {
        let inner = typed::<EnumLabel>(row, index, array, ArrayType::String, |v| {
            Value::String(v.map(|v| Box::new(v.0)))
        })?;
        return Ok(Tagged::from_enum(
            inner,
            TypeName::new(ty.name()).in_schema(ty.schema()),
            array,
        ));
    }
    let inner = match *ty {
        Type::BOOL => scalar!(bool, Bool),
        Type::CHAR => scalar!(i8, TinyInt),
        Type::INT2 => scalar!(i16, SmallInt),
        Type::INT4 => scalar!(i32, Int),
        Type::INT8 => scalar!(i64, BigInt),
        Type::OID => scalar!(u32, Unsigned),
        Type::FLOAT4 => scalar!(f32, Float),
        Type::FLOAT8 => scalar!(f64, Double),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => boxed!(String, String),
        Type::BYTEA => boxed!(Vec<u8>, Bytes),
        Type::JSON | Type::JSONB => typed::<ExactJson>(row, index, array, ArrayType::Json, |v| {
            Value::Json(v.map(|v| Box::new(v.0)))
        }),
        Type::UUID => boxed!(Uuid, Uuid),
        Type::DATE => boxed!(Date, Date),
        Type::TIME => boxed!(Time, Time),
        Type::TIMESTAMP => boxed!(DateTime, DateTime),
        Type::TIMESTAMPTZ => boxed!(Timestamp, DateTimeWithTimeZone),
        Type::INTERVAL => boxed!(Interval, Interval),
        Type::NUMERIC => typed::<ExactDecimal>(row, index, array, ArrayType::Decimal, |v| {
            Value::Decimal(v.map(|v| Box::new(v.0)))
        }),
        Type::INET | Type::CIDR => typed::<Inet>(row, index, array, ArrayType::IpNetwork, |v| {
            Value::IpNetwork(v.map(|v| Box::new(v.0)))
        }),
        Type::MACADDR => typed::<Mac>(row, index, array, ArrayType::MacAddress, |v| {
            Value::MacAddress(v.map(|v| Box::new(v.0)))
        }),
        _ if ty.name() == "vector" && matches!(ty.kind(), Kind::Simple) => boxed!(Vector, Vector),
        _ => {
            return Err(FormatError::new(format!(
                "column {index} has an unsupported PostgreSQL type"
            )));
        }
    }?;
    Ok(Tagged::from_value(inner))
}
