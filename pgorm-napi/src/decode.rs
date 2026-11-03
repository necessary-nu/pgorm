//! Result rows into the binding's values, on the runtime thread.
//!
//! Each column is read in its binary form and decoded through the driver's
//! codec for its type, the codec pgorm's own `TryGetable` uses, after looking
//! through any domain to the type it is built over. A value the binding's
//! JavaScript type for its kind cannot hold exactly is a decode failure, never
//! a rounded value, a NULL or a missing column.

use std::{collections::HashSet, error::Error, ops::Bound};

use bytes::BytesMut;
use fallible_iterator::FallibleIterator;
use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use pgorm::{
    CopyColumn,
    pgorm_query::{
        ArrayType, Interval, IpNetwork, MacAddress, Multirange, Range, RangeType, Value, Vector,
        base_type,
    },
};
use postgres_protocol::types::{self as wire, RangeBound};
use rust_decimal::Decimal;
use tokio_postgres::{
    Row,
    types::{FromSql, Kind, ToSql, Type},
};
use uuid::Uuid;

use crate::{
    errors::Failure,
    values::{CreatedKind, Tag, Tagged, TypeName, created, json::ExactJson},
};

type CodecError = Box<dyn Error + Send + Sync>;

/// A column's bytes, or `None` for SQL NULL, whatever its type.
struct Raw<'a>(Option<&'a [u8]>);

impl<'a> FromSql<'a> for Raw<'a> {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, CodecError> {
        Ok(Self(Some(raw)))
    }

    fn from_sql_null(_: &Type) -> Result<Self, CodecError> {
        Ok(Self(None))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

/// A result's column names, which become the keys of its row objects: two
/// columns of one name would leave one object key for both, and are refused.
// [spec:pgorm:req:bindings.rows]
pub(crate) fn names(row: &Row) -> Result<Vec<String>, Failure> {
    let names: Vec<String> = row
        .columns()
        .iter()
        .map(|column| column.name().to_owned())
        .collect();
    let mut seen = HashSet::with_capacity(names.len());
    for name in &names {
        if !seen.insert(name.as_str()) {
            return Err(Failure::Decode(format!(
                "the result has more than one column named {name:?}: alias them apart"
            )));
        }
    }
    Ok(names)
}

/// One row's values, in column order.
// [spec:pgorm:req:bindings.rows]
pub(crate) fn row(row: &Row) -> Result<Vec<Tagged>, Failure> {
    (0..row.len()).map(|index| column(row, index)).collect()
}

fn column(row: &Row, index: usize) -> Result<Tagged, Failure> {
    let column = &row.columns()[index];
    let raw = row
        .try_get::<_, Raw>(index)
        .map_err(|error| Failure::Decode(error.to_string()))?;
    decode(column.type_(), raw.0).map_err(|error| {
        Failure::Decode(format!(
            "column {:?} of type `{}` cannot be decoded: {error}",
            column.name(),
            column.type_()
        ))
    })
}

/// One field of a COPY of rows, read as a result column of its type is. A
/// `money` field is the count of its currency's minor units, which pgorm
/// reads as a `Decimal` at the places the COPY's description learned; a
/// query's `money` column has no places beside it and is refused, as a query
/// reads it cast to `numeric`.
// [spec:pgorm:req:bindings.copy-rows+1]
pub(crate) fn field(column: &CopyColumn, raw: Option<&[u8]>) -> Result<Tagged, Failure> {
    if column.money_places().is_some() {
        return match column.decode(raw) {
            Ok(value) => Ok(Tagged::value(value)),
            Err(pgorm::Error::Type(message)) => Err(Failure::Decode(message)),
            Err(error) => Err(Failure::Decode(error.to_string())),
        };
    }
    decode(column.type_(), raw).map_err(|error| {
        Failure::Decode(format!(
            "COPY column {:?} of type `{}` cannot be decoded: {error}",
            column.name(),
            column.type_()
        ))
    })
}

/// A value of `ty`, an array of a supported type included.
// [spec:pgorm:req:bindings.values]
fn decode(ty: &Type, raw: Option<&[u8]>) -> Result<Tagged, CodecError> {
    let ty = base_type(ty);
    let Kind::Array(member) = ty.kind() else {
        return scalar(ty, raw);
    };
    let member = base_type(member);
    let element = tag(member)?;
    if matches!(element, Tag::Created(_)) {
        return Err("arrays of a created range type are not supported".into());
    }
    let tag = Tag::Array(Box::new(element.clone()));
    let Some(raw) = raw else {
        return Ok(Tagged::null(tag));
    };
    let array = wire::array_from_sql(raw)?;
    let dimensions = array.dimensions().collect::<Vec<_>>()?;
    if dimensions.len() > 1 || dimensions.first().is_some_and(|d| d.lower_bound != 1) {
        return Err("only one-dimensional arrays with lower bound 1 are supported".into());
    }
    let items = array.values().collect::<Vec<_>>()?;
    let kind = match &element {
        Tag::Scalar(kind) => kind.clone(),
        _ => ArrayType::String,
    };
    let values = items
        .into_iter()
        .map(|item| scalar(member, item).map(|item| item.datum))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Tagged {
        datum: Value::Array(kind, Some(Box::new(values))),
        tag,
    })
}

/// Whether `ty` is one of the six built-in range types or their multiranges,
/// rather than a type a schema created.
fn built_in(ty: &Type) -> Option<RangeType> {
    Some(match *ty {
        Type::INT4_RANGE | Type::INT4MULTI_RANGE => RangeType::Int4,
        Type::INT8_RANGE | Type::INT8MULTI_RANGE => RangeType::Int8,
        Type::NUM_RANGE | Type::NUMMULTI_RANGE => RangeType::Numeric,
        Type::DATE_RANGE | Type::DATEMULTI_RANGE => RangeType::Date,
        Type::TS_RANGE | Type::TSMULTI_RANGE => RangeType::Timestamp,
        Type::TSTZ_RANGE | Type::TSTZMULTI_RANGE => RangeType::TimestampTz,
        _ => return None,
    })
}

/// The value kind of a created range's subtype.
fn subtype_kind(subtype: &Type) -> Option<ArrayType> {
    Some(match *base_type(subtype) {
        Type::INT2 => ArrayType::SmallInt,
        Type::INT4 => ArrayType::Int,
        Type::INT8 => ArrayType::BigInt,
        Type::FLOAT4 => ArrayType::Float,
        Type::FLOAT8 => ArrayType::Double,
        Type::NUMERIC => ArrayType::Decimal,
        Type::TEXT | Type::VARCHAR | Type::BPCHAR => ArrayType::String,
        Type::DATE => ArrayType::Date,
        Type::TIME => ArrayType::Time,
        Type::TIMESTAMP => ArrayType::DateTime,
        Type::TIMESTAMPTZ => ArrayType::DateTimeWithTimeZone,
        Type::UUID => ArrayType::Uuid,
        _ => return None,
    })
}

/// The kind a column of `ty` decodes as.
// [spec:pgorm:req:bindings.value-tags]
// [spec:pgorm:req:napi.temporal+1]
fn tag(ty: &Type) -> Result<Tag, CodecError> {
    let ty = base_type(ty);
    match ty.kind() {
        Kind::Enum(_) => {
            return Ok(Tag::Enum(TypeName {
                name: ty.name().to_owned(),
                schema: Some(ty.schema().to_owned()),
            }));
        }
        Kind::Range(subtype) | Kind::Multirange(subtype) => {
            let multirange = matches!(ty.kind(), Kind::Multirange(_));
            if let Some(range) = built_in(ty) {
                return Ok(Tag::Scalar(if multirange {
                    ArrayType::Multirange(range)
                } else {
                    ArrayType::Range(range)
                }));
            }
            let Some(subtype) = subtype_kind(subtype) else {
                return Err(format!(
                    "the range type ranges over `{subtype}`, which no created range kind does"
                )
                .into());
            };
            return Ok(Tag::Created(CreatedKind {
                name: TypeName {
                    name: ty.name().to_owned(),
                    schema: Some(ty.schema().to_owned()),
                },
                subtype,
                multirange,
            }));
        }
        Kind::Array(_) => return Err("nested arrays are not supported".into()),
        _ => {}
    }
    let kind = match *ty {
        Type::BOOL => ArrayType::Bool,
        Type::CHAR => ArrayType::TinyInt,
        Type::INT2 => ArrayType::SmallInt,
        Type::INT4 => ArrayType::Int,
        Type::INT8 => ArrayType::BigInt,
        Type::OID => ArrayType::Unsigned,
        Type::FLOAT4 => ArrayType::Float,
        Type::FLOAT8 => ArrayType::Double,
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => ArrayType::String,
        Type::BYTEA => ArrayType::Bytes,
        Type::JSON | Type::JSONB => ArrayType::Json,
        Type::UUID => ArrayType::Uuid,
        Type::DATE => ArrayType::Date,
        Type::TIME => ArrayType::Time,
        Type::TIMESTAMP => ArrayType::DateTime,
        Type::TIMESTAMPTZ => ArrayType::DateTimeWithTimeZone,
        Type::NUMERIC => ArrayType::Decimal,
        Type::INET | Type::CIDR => ArrayType::IpNetwork,
        Type::MACADDR => ArrayType::MacAddress,
        Type::INTERVAL => ArrayType::Interval,
        Type::TIMETZ => {
            return Err(
                "timetz is not supported: no Temporal type holds a time of day at a \
                        fixed offset; select it cast to time, or to text"
                    .into(),
            );
        }
        _ if ty.name() == "vector" && matches!(ty.kind(), Kind::Simple) => ArrayType::Vector,
        _ => return Err("the type is not supported".into()),
    };
    Ok(Tag::Scalar(kind))
}

fn scalar(ty: &Type, raw: Option<&[u8]>) -> Result<Tagged, CodecError> {
    let ty = base_type(ty);
    let tag = tag(ty)?;
    let Some(raw) = raw else {
        return Ok(Tagged::null(tag));
    };
    let datum = match &tag {
        // The server sends only the type's labels, and the client's copy of
        // the label list can be older than an `ALTER TYPE ... ADD VALUE`, so
        // the label is not checked against it.
        Tag::Enum(_) => Value::String(Some(Box::new(String::from_sql(&Type::TEXT, raw)?))),
        Tag::Created(kind) => Value::String(Some(Box::new(created_text(ty, kind, raw)?))),
        Tag::Scalar(kind) => value(ty, kind, raw)?,
        Tag::Array(_) => return Err("nested arrays are not supported".into()),
    };
    Ok(Tagged { datum, tag })
}

/// A non-NULL value of `kind`, from its binary form.
fn value(ty: &Type, kind: &ArrayType, raw: &[u8]) -> Result<Value, CodecError> {
    Ok(match kind {
        ArrayType::Bool => Value::Bool(Some(bool::from_sql(ty, raw)?)),
        ArrayType::TinyInt => Value::TinyInt(Some(i8::from_sql(ty, raw)?)),
        ArrayType::SmallInt => Value::SmallInt(Some(i16::from_sql(ty, raw)?)),
        ArrayType::Int => Value::Int(Some(i32::from_sql(ty, raw)?)),
        ArrayType::BigInt => Value::BigInt(Some(i64::from_sql(ty, raw)?)),
        ArrayType::Unsigned => Value::Unsigned(Some(u32::from_sql(ty, raw)?)),
        ArrayType::Float => Value::Float(Some(f32::from_sql(ty, raw)?)),
        ArrayType::Double => Value::Double(Some(f64::from_sql(ty, raw)?)),
        ArrayType::String => Value::String(Some(Box::new(String::from_sql(ty, raw)?))),
        ArrayType::Bytes => Value::Bytes(Some(Box::new(Vec::<u8>::from_sql(ty, raw)?))),
        ArrayType::Json => Value::Json(Some(Box::new(ExactJson::from_sql(ty, raw)?.0))),
        ArrayType::Uuid => Value::Uuid(Some(Box::new(Uuid::from_sql(ty, raw)?))),
        ArrayType::Date => Value::Date(Some(Box::new(Date::from_sql(ty, raw)?))),
        // The driver's civil-time codec refuses 24:00:00, which no Temporal
        // time holds either.
        ArrayType::Time => Value::Time(Some(Box::new(Time::from_sql(ty, raw)?))),
        ArrayType::DateTime => Value::DateTime(Some(Box::new(DateTime::from_sql(ty, raw)?))),
        ArrayType::DateTimeWithTimeZone => {
            Value::DateTimeWithTimeZone(Some(Box::new(Timestamp::from_sql(ty, raw)?)))
        }
        ArrayType::Decimal => Value::Decimal(Some(Box::new(ExactDecimal::from_sql(ty, raw)?.0))),
        ArrayType::IpNetwork => {
            let inet = wire::inet_from_sql(raw)?;
            Value::IpNetwork(Some(Box::new(IpNetwork::new(inet.addr(), inet.netmask())?)))
        }
        ArrayType::MacAddress => Value::MacAddress(Some(Box::new(MacAddress::new(
            wire::macaddr_from_sql(raw)?,
        )))),
        ArrayType::Vector => Value::Vector(Some(Box::new(Vector::from_sql(ty, raw)?))),
        ArrayType::Range(range) => {
            let Kind::Range(subtype) = ty.kind() else {
                return Err("expected a range".into());
            };
            let element = range.element_type();
            Value::Range(*range, Some(Box::new(read_range(subtype, raw, &element)?)))
        }
        ArrayType::Multirange(range) => {
            let Kind::Multirange(subtype) = ty.kind() else {
                return Err("expected a multirange".into());
            };
            let element = range.element_type();
            Value::Multirange(
                *range,
                Some(Box::new(Multirange::from(read_multirange(
                    subtype, raw, &element,
                )?))),
            )
        }
        ArrayType::Interval => Value::from(Interval::from_sql(ty, raw)?),
        // `tag` answers neither of these, which no column decodes as.
        ArrayType::BigUnsigned | ArrayType::Char => {
            return Err("no PostgreSQL type decodes as this kind".into());
        }
        _ => return Err(crate::values::unrepresented().message().into()),
    })
}

fn read_i32(raw: &mut &[u8]) -> Result<i32, CodecError> {
    let (head, rest) = raw.split_first_chunk::<4>().ok_or("invalid message size")?;
    *raw = rest;
    Ok(i32::from_be_bytes(*head))
}

/// A range in its binary form, each bound decoded as a value of `element` by
/// the codec of `subtype`. The empty flag is the empty range and nothing
/// else; a NULL bound is a malformed message, never no bound.
fn read_range(subtype: &Type, raw: &[u8], element: &ArrayType) -> Result<Range<Value>, CodecError> {
    let subtype = base_type(subtype);
    let bound = |bound: RangeBound<Option<&[u8]>>| -> Result<Bound<Value>, CodecError> {
        Ok(match bound {
            RangeBound::Inclusive(Some(raw)) => Bound::Included(value(subtype, element, raw)?),
            RangeBound::Exclusive(Some(raw)) => Bound::Excluded(value(subtype, element, raw)?),
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

/// A multirange: a count, then each range length-prefixed.
fn read_multirange(
    subtype: &Type,
    raw: &[u8],
    element: &ArrayType,
) -> Result<Vec<Range<Value>>, CodecError> {
    let mut raw = raw;
    let count = usize::try_from(read_i32(&mut raw)?)?;
    let mut ranges = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let len = usize::try_from(read_i32(&mut raw)?)?;
        let (range, rest) = raw.split_at_checked(len).ok_or("invalid message size")?;
        raw = rest;
        ranges.push(read_range(subtype, range, element)?);
    }
    if !raw.is_empty() {
        return Err("invalid message size".into());
    }
    Ok(ranges)
}

/// A column of a range type a schema created: read in binary through the
/// subtype's codec, and held as the text form that type's values travel as.
// [spec:pgorm:req:bindings.created-ranges+1]
fn created_text(ty: &Type, kind: &CreatedKind, raw: &[u8]) -> Result<String, CodecError> {
    let (Kind::Range(subtype) | Kind::Multirange(subtype)) = ty.kind() else {
        return Err("expected a range".into());
    };
    let ranges = if kind.multirange {
        read_multirange(subtype, raw, &kind.subtype)?
    } else {
        vec![read_range(subtype, raw, &kind.subtype)?]
    };
    created::text(&kind.subtype, ranges, kind.multirange)
        .ok_or_else(|| "a bound does not convert to the range's subtype".into())
}

/// A `numeric` that rust_decimal, and so pgorm, holds exactly: its upstream
/// decoder can round, so the value it reads is written back and compared.
// [spec:pgorm:req:bindings.values]
// [spec:pgorm:req:napi.values+1]
#[derive(Debug)]
pub(crate) struct ExactDecimal(pub(crate) Decimal);

impl<'a> FromSql<'a> for ExactDecimal {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, CodecError> {
        let input = Numeric::read(raw)?;
        if input.scale > 28 {
            return Err("the numeric has more than 28 fractional digits, a decimal's most".into());
        }
        let value = Decimal::from_sql(ty, raw)?;
        let mut encoded = BytesMut::new();
        value.to_sql(ty, &mut encoded)?;
        let output = Numeric::read(&encoded)?;
        if value.scale() != u32::from(input.scale) || input.canonical() != output.canonical() {
            return Err("the numeric has no exact decimal: its coefficient exceeds 96 bits".into());
        }
        Ok(Self(value))
    }

    fn accepts(ty: &Type) -> bool {
        <Decimal as FromSql>::accepts(ty)
    }
}

struct Numeric {
    weight: i16,
    sign: u16,
    scale: u16,
    digits: Vec<u16>,
}

impl Numeric {
    fn read(raw: &[u8]) -> Result<Self, CodecError> {
        if raw.len() < 8 || !raw.len().is_multiple_of(2) {
            return Err("invalid numeric binary representation".into());
        }
        let word = |i: usize| u16::from_be_bytes([raw[i], raw[i + 1]]);
        if usize::from(word(0)) * 2 + 8 != raw.len() {
            return Err("invalid numeric digit count".into());
        }
        let digits: Vec<_> = (8..raw.len()).step_by(2).map(word).collect();
        if !matches!(word(4), 0 | 0x4000) {
            return Err("the numeric is NaN or infinite, which a decimal cannot hold".into());
        }
        if digits.iter().any(|digit| *digit >= 10_000) {
            return Err("invalid numeric digit".into());
        }
        Ok(Self {
            weight: i16::from_be_bytes([raw[2], raw[3]]),
            sign: word(4),
            scale: word(6),
            digits,
        })
    }

    fn canonical(&self) -> (i32, u16, &[u16]) {
        let start = self.digits.iter().position(|digit| *digit != 0);
        let end = self.digits.iter().rposition(|digit| *digit != 0);
        match (start, end) {
            (Some(start), Some(end)) => (
                i32::from(self.weight) - i32::try_from(start).unwrap_or(i32::MAX),
                self.sign,
                &self.digits[start..=end],
            ),
            _ => (0, 0, &[]),
        }
    }
}

#[cfg(test)]
mod tests {
    use pgorm::pgorm_query::tests_cfg::Variants;

    use super::*;

    fn numeric(text: &str) -> Vec<u8> {
        let mut out = BytesMut::new();
        match Decimal::from_str_exact(text) {
            Ok(value) => match value.to_sql(&Type::NUMERIC, &mut out) {
                Ok(_) => out.to_vec(),
                Err(error) => panic!("{error}"),
            },
            Err(error) => panic!("{error}"),
        }
    }

    // [spec:pgorm:req:bindings.values/test]
    #[test]
    fn numerics_decode_exactly_or_not_at_all() -> Result<(), CodecError> {
        assert_eq!(
            ExactDecimal::from_sql(&Type::NUMERIC, &numeric("19.9900"))?
                .0
                .to_string(),
            "19.9900"
        );
        // NaN: no digits, the NaN sign word.
        let nan = [0, 0, 0, 0, 0xC0, 0, 0, 0];
        assert!(ExactDecimal::from_sql(&Type::NUMERIC, &nan).is_err());
        // Scale 29, one digit group: more fractional digits than a decimal keeps.
        let fine = [0, 1, 0xFF, 0xF8, 0, 0, 0, 29, 0, 1];
        assert!(ExactDecimal::from_sql(&Type::NUMERIC, &fine).is_err());
        Ok(())
    }

    // [spec:pgorm:req:bindings.created-ranges+1/test]
    #[test]
    fn created_range_columns_read_as_text_form() -> Result<(), CodecError> {
        let kind = CreatedKind {
            name: TypeName {
                name: "floatrange".into(),
                schema: Some("public".into()),
            },
            subtype: ArrayType::Double,
            multirange: false,
        };
        let ty = Type::new(
            "floatrange".into(),
            90_000,
            Kind::Range(Type::FLOAT8),
            "public".into(),
        );
        let mut raw = BytesMut::new();
        wire::range_to_sql(
            |out| {
                wire::float8_to_sql(1.5, out);
                Ok(RangeBound::Inclusive(postgres_protocol::IsNull::No))
            },
            |out| {
                wire::float8_to_sql(2.5, out);
                Ok(RangeBound::Exclusive(postgres_protocol::IsNull::No))
            },
            &mut raw,
        )?;
        assert_eq!(created_text(&ty, &kind, &raw)?, "[1.5,2.5)");
        Ok(())
    }

    // [spec:pgorm:req:bindings.values/test]
    #[test]
    fn no_value_kind_reaches_the_decoders_wildcard_arm() {
        let unrepresented = crate::values::unrepresented();
        // Long enough that no codec indexes past it; most refuse it.
        let raw = [0; 16];
        for kind in ArrayType::variants().into_iter().chain(ArrayType::kinds()) {
            if let Err(error) = value(&Type::TEXT, &kind, &raw) {
                assert_ne!(error.to_string(), unrepresented.message(), "{kind:?}");
            }
        }
    }
}
