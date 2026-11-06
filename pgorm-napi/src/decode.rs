//! Result rows into the binding's values, on the runtime thread.
//!
//! Each column is read in its binary form and decoded through the driver's
//! codec for its type, the codec pgorm's own `TryGetable` uses, after looking
//! through any domain to the type it is built over. A value the binding's
//! JavaScript type for its kind cannot hold exactly is a decode failure, never
//! a rounded value, a NULL or a missing column.

use std::{collections::HashSet, error::Error};

use fallible_iterator::FallibleIterator;
use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use pgorm::{
    CopyColumn, CreatedRangeCodec, CreatedRangeText, ExactDecimal,
    pgorm_query::{
        ArrayType, Interval, IpNetwork, MacAddress, Multirange, Range, RangeType, Value, Vector,
        base_type,
    },
};
use postgres_protocol::types as wire;
use tokio_postgres::{
    Row,
    types::{FromSql, Kind, Type},
};
use uuid::Uuid;

use crate::{
    errors::Failure,
    values::{CreatedKind, Tag, Tagged, TypeName, json::ExactJson},
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
// [spec:pgorm:req:bindings.copy-rows+2]
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

/// The kind a column of `ty` decodes as.
// [spec:pgorm:req:bindings.value-tags+1]
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
            let Some(subtype) = CreatedRangeCodec::subtype_kind(subtype) else {
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
        Tag::Created(_) => Value::String(Some(Box::new(CreatedRangeText::from_sql(ty, raw)?.0))),
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
                Some(Box::new(read_multirange(subtype, raw, &element)?)),
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

/// A range, each bound decoded as a value of `element` by the codec of
/// `subtype`, through pgorm's one range reader.
// [spec:pgorm:def:exec.decode.range+4]
fn read_range(subtype: &Type, raw: &[u8], element: &ArrayType) -> Result<Range<Value>, CodecError> {
    let subtype = base_type(subtype);
    pgorm::range_from_sql_with(raw, |raw| value(subtype, element, raw))
}

/// A multirange, each range's bounds read as [`read_range`] reads them.
// [spec:pgorm:def:exec.decode.range+4]
fn read_multirange(
    subtype: &Type,
    raw: &[u8],
    element: &ArrayType,
) -> Result<Multirange<Value>, CodecError> {
    let subtype = base_type(subtype);
    pgorm::multirange_from_sql_with(raw, |raw| value(subtype, element, raw))
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use pgorm::pgorm_query::tests_cfg::Variants;
    use postgres_protocol::types::RangeBound;

    use super::*;

    // [spec:pgorm:req:bindings.created-ranges+5/test]
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
        let read = scalar(&ty, Some(&raw))?;
        assert_eq!(read.tag, Tag::Created(kind));
        assert_eq!(
            read.datum,
            Value::String(Some(Box::new("[1.5,2.5)".into())))
        );
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
