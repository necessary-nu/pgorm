//! Range and multirange columns: the wire format read through
//! `postgres_protocol`, each bound by the same codec a scalar of the subtype
//! is read with, so a numeric bound is held to the same exactness.

use std::ops::Bound;

use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use pgorm::pgorm_query::{ArrayType, Multirange, Range, RangeType, Value, base_type};
use postgres_protocol::types::{self as wire, RangeBound};
use pyo3::PyResult;
use tokio_postgres::types::{FromSql, Kind, Type};

use uuid::Uuid;

use super::{
    codecs::ExactDecimal,
    decode::{Field, read, typed},
};
use crate::{
    errors::DecodeError,
    values::{CreatedKind, PyTypeName, PyValue},
};

type CodecError = Box<dyn std::error::Error + Send + Sync>;

/// A range over a subtype `T` reads: a built-in range, or a range type a
/// schema created over the same subtype.
#[derive(Debug)]
struct RangeCodec<T>(Range<T>);

impl<'a, T: FromSql<'a>> FromSql<'a> for RangeCodec<T> {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, CodecError> {
        match ty.kind() {
            Kind::Range(subtype) => read_range(base_type(subtype), raw).map(Self),
            _ => Err("expected a PostgreSQL range".into()),
        }
    }

    fn accepts(ty: &Type) -> bool {
        matches!(ty.kind(), Kind::Range(subtype) if T::accepts(base_type(subtype)))
    }
}

/// A multirange: a count, then each range length-prefixed.
#[derive(Debug)]
struct MultirangeCodec<T>(Multirange<T>);

impl<'a, T: FromSql<'a>> FromSql<'a> for MultirangeCodec<T> {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, CodecError> {
        let Kind::Multirange(subtype) = ty.kind() else {
            return Err("expected a PostgreSQL multirange".into());
        };
        let subtype = base_type(subtype);
        let mut raw = raw;
        let count = usize::try_from(read_i32(&mut raw)?)?;
        let ranges = (0..count)
            .map(|_| {
                let len = usize::try_from(read_i32(&mut raw)?)?;
                let (range, rest) = raw.split_at_checked(len).ok_or("invalid message size")?;
                raw = rest;
                read_range(subtype, range)
            })
            .collect::<Result<Multirange<T>, CodecError>>()?;
        if !raw.is_empty() {
            return Err("invalid message size".into());
        }
        Ok(Self(ranges))
    }

    fn accepts(ty: &Type) -> bool {
        matches!(ty.kind(), Kind::Multirange(subtype) if T::accepts(base_type(subtype)))
    }
}

fn read_i32(raw: &mut &[u8]) -> Result<i32, CodecError> {
    let (head, rest) = raw.split_first_chunk::<4>().ok_or("invalid message size")?;
    *raw = rest;
    Ok(i32::from_be_bytes(*head))
}

/// The empty flag is the empty range and nothing else; a NULL bound is a
/// malformed message, never no bound.
fn read_range<'a, T: FromSql<'a>>(subtype: &Type, raw: &'a [u8]) -> Result<Range<T>, CodecError> {
    let bound = |bound: RangeBound<Option<&'a [u8]>>| -> Result<Bound<T>, CodecError> {
        Ok(match bound {
            RangeBound::Inclusive(Some(raw)) => Bound::Included(T::from_sql(subtype, raw)?),
            RangeBound::Exclusive(Some(raw)) => Bound::Excluded(T::from_sql(subtype, raw)?),
            RangeBound::Unbounded => Bound::Unbounded,
            RangeBound::Inclusive(None) | RangeBound::Exclusive(None) => {
                return Err("a range bound is NULL".into());
            }
        })
    };
    Ok(match wire::range_from_sql(raw)? {
        wire::Range::Empty => Range::Empty,
        wire::Range::Nonempty(lower, upper) => Range::new(bound(lower)?, bound(upper)?),
    })
}

/// The built-in range type over `subtype`, which a range type a schema
/// created over the same subtype reads as.
fn range_type(subtype: &Type) -> Option<RangeType> {
    Some(match *subtype {
        Type::INT4 => RangeType::Int4,
        Type::INT8 => RangeType::Int8,
        Type::NUMERIC => RangeType::Numeric,
        Type::DATE => RangeType::Date,
        Type::TIMESTAMP => RangeType::Timestamp,
        Type::TIMESTAMPTZ => RangeType::TimestampTz,
        _ => return None,
    })
}

/// Whether `ty` is one of the six built-in range types or their multiranges,
/// rather than a type a schema created.
fn built_in(ty: &Type) -> bool {
    matches!(
        *ty,
        Type::INT4_RANGE
            | Type::INT8_RANGE
            | Type::NUM_RANGE
            | Type::DATE_RANGE
            | Type::TS_RANGE
            | Type::TSTZ_RANGE
            | Type::INT4MULTI_RANGE
            | Type::INT8MULTI_RANGE
            | Type::NUMMULTI_RANGE
            | Type::DATEMULTI_RANGE
            | Type::TSMULTI_RANGE
            | Type::TSTZMULTI_RANGE
    )
}

/// A range or multirange column, or an array of either, as a tagged value;
/// `None` for a column of any other type. A range type a schema created reads
/// as its own kind, named as the column's type is, whatever its subtype.
// [spec:pgorm:req:bindings.values]
pub(super) fn value(field: Field<'_>, ty: &Type, array: bool) -> PyResult<Option<PyValue>> {
    let index = field.index();
    let (subtype, multi) = match ty.kind() {
        Kind::Range(subtype) => (subtype, false),
        Kind::Multirange(subtype) => (subtype, true),
        _ => return Ok(None),
    };
    if !built_in(ty) {
        if array {
            return Err(DecodeError::new_err(format!(
                "column {index} is an array of the created range type `{ty}`, which Python \
                 does not hold"
            )));
        }
        return created(field, ty, subtype, multi).map(Some);
    }
    let Some(range) = range_type(subtype) else {
        return Err(DecodeError::new_err(format!(
            "column {index} ranges over `{subtype}`, which none of the built-in range types does"
        )));
    };
    macro_rules! decode {
        ($rust:ty, $bound:expr) => {
            if multi {
                typed::<MultirangeCodec<$rust>>(field, array, ArrayType::Multirange(range), |v| {
                    Value::Multirange(range, v.map(|v| Box::new(v.0.map($bound))))
                })
            } else {
                typed::<RangeCodec<$rust>>(field, array, ArrayType::Range(range), |v| {
                    Value::Range(range, v.map(|v| Box::new(v.0.map($bound))))
                })
            }
        };
    }
    let value = match range {
        RangeType::Int4 => decode!(i32, |v| Value::Int(Some(v))),
        RangeType::Int8 => decode!(i64, |v| Value::BigInt(Some(v))),
        RangeType::Numeric => decode!(ExactDecimal, |v: ExactDecimal| Value::Decimal(Some(
            Box::new(v.0)
        ))),
        RangeType::Date => decode!(Date, |v| Value::Date(Some(Box::new(v)))),
        RangeType::Timestamp => decode!(DateTime, |v| Value::DateTime(Some(Box::new(v)))),
        RangeType::TimestampTz => decode!(Timestamp, |v| Value::DateTimeWithTimeZone(Some(
            Box::new(v)
        ))),
        _ => Err(crate::values::unrepresented()),
    }?;
    Ok(Some(PyValue::from_rust(value)))
}

/// A column of a range type a schema created: read in binary as a range of
/// its subtype's Rust type, through the subtype's own codec, and held as the
/// text form that type's values travel as, tagged with the type's name.
// [spec:pgorm:req:bindings.created-ranges+2]
fn created(field: Field<'_>, ty: &Type, subtype: &Type, multi: bool) -> PyResult<PyValue> {
    let index = field.index();
    macro_rules! text {
        ($rust:ty, $kind:ident, $bound:expr) => {
            (
                ArrayType::$kind,
                if multi {
                    read::<MultirangeCodec<$rust>>(field)?.map(|v| v.0.map($bound).to_string())
                } else {
                    read::<RangeCodec<$rust>>(field)?.map(|v| v.0.map($bound).to_string())
                },
            )
        };
    }
    let (kind, text) = match *base_type(subtype) {
        Type::INT2 => text!(i16, SmallInt, |v| v),
        Type::INT4 => text!(i32, Int, |v| v),
        Type::INT8 => text!(i64, BigInt, |v| v),
        Type::FLOAT4 => text!(f32, Float, |v| v),
        Type::FLOAT8 => text!(f64, Double, |v| v),
        Type::NUMERIC => text!(ExactDecimal, Decimal, |v: ExactDecimal| v.0),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR => text!(String, String, |v| v),
        Type::DATE => text!(Date, Date, |v| v),
        Type::TIME => text!(Time, Time, |v| v),
        Type::TIMESTAMP => text!(DateTime, DateTime, |v| v),
        Type::TIMESTAMPTZ => text!(Timestamp, DateTimeWithTimeZone, |v| v),
        Type::UUID => text!(Uuid, Uuid, |v| v),
        _ => {
            return Err(DecodeError::new_err(format!(
                "column {index} ranges over `{subtype}`, which a created range read from \
                 Python cannot"
            )));
        }
    };
    Ok(PyValue::from_created(
        text,
        CreatedKind {
            name: PyTypeName {
                name: ty.name().to_owned(),
                schema: Some(ty.schema().to_owned()),
            },
            subtype: kind,
            multirange: multi,
        },
    ))
}
