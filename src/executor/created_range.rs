//! A range or multirange type a schema created, between the binary form it
//! travels in and the text form its [`Value`] is: the `Value::String` the
//! [`CreatedRange`](crate::CreatedRange) newtype naming it converts into.
//!
//! tokio-postgres reports such a range as a range over its subtype and knows
//! nothing else of it, so its bounds are read and written through the Rust
//! type that subtype reads as, one of [`RangeSubtype`]'s, whose `Display` and
//! `FromStr` are the text form. It reports such a multirange as a simple type
//! with no subtype at all, so a multirange is read this way only where its
//! kind has been completed from the catalogue ([`completed_multiranges`]). A
//! built-in range or multirange has a `Value` of its own and is not read here.

use std::{future::Future, ops::Bound, pin::Pin};

use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use pgorm_pool::GenericClient;
use pgorm_query::{ArrayType, Multirange, Range, RangeSubtype, Value, base_type};
use rust_decimal::Decimal;
use tokio_postgres::types::{FromSql, Kind, Oid, Type};

use super::{
    exact_decimal::ExactDecimal,
    query::{multirange_from_sql, range_from_sql},
};
use crate::Error;

type WireError = Box<dyn std::error::Error + Sync + Send>;

/// A range's or a multirange's binary form, read against its subtype, as its
/// text form.
type ToText = fn(&Type, &[u8], bool) -> Result<String, WireError>;

/// The text form of ranges whose bounds are the subtype's values: one range,
/// or the multirange of them; `None` when a bound is no value of the subtype.
type Written = fn(Vec<Range<Value>>, bool) -> Option<String>;

/// The ranges a text form spells, one or a multirange's, each bound the
/// `Value` of the subtype's Rust type; `None` for text spelling none.
type Read = fn(&str, bool) -> Option<Vec<Range<Value>>>;

/// Whether `ty` is a range type a schema created: a range, and none of the
/// six built in, each of which has a `RangeType` and so a `Value::Range`.
// [spec:pgorm:def:sql.value.created-range+9]
pub(crate) fn is_created_range(ty: &Type) -> bool {
    matches!(ty.kind(), Kind::Range(_))
        && !matches!(
            *ty,
            Type::INT4_RANGE
                | Type::INT8_RANGE
                | Type::NUM_RANGE
                | Type::DATE_RANGE
                | Type::TS_RANGE
                | Type::TSTZ_RANGE
        )
}

/// Whether `ty` is a multirange type a schema created whose kind names its
/// subtype, as [`completed_multiranges`] makes it: a multirange, and none of
/// the six built in, each of which has a `RangeType` and so a
/// `Value::Multirange`.
// [spec:pgorm:def:sql.value.created-range+9]
pub(crate) fn is_created_multirange(ty: &Type) -> bool {
    matches!(ty.kind(), Kind::Multirange(_))
        && !matches!(
            *ty,
            Type::INT4MULTI_RANGE
                | Type::INT8MULTI_RANGE
                | Type::NUMMULTI_RANGE
                | Type::DATEMULTI_RANGE
                | Type::TSMULTI_RANGE
                | Type::TSTZMULTI_RANGE
        )
}

/// The format of a range or multirange type a schema created, over one of
/// the subtypes [`RangeSubtype`] admits, named by the value kind its bounds
/// are: its binary form read through the subtype's codec, a `numeric` bound
/// held to [`ExactDecimal`]'s exactness, and its text form written and read
/// through the subtype's Rust type, as `Range<T>`'s `Display` and `FromStr`
/// write and read it.
///
/// It is the one implementation of that format: pgorm's binary COPY and
/// cursor bindings read and write such a range through it, and so do the
/// language bindings, which hold its value as its text form and its bounds
/// as `Value`s of the subtype's kind.
// [spec:pgorm:def:sql.value.created-range+9]
#[derive(Clone, Debug)]
pub struct CreatedRangeCodec {
    subtype: ArrayType,
    multirange: bool,
    decoded: ToText,
    written: Written,
    read: Read,
}

impl CreatedRangeCodec {
    /// The codec of a range type a schema created over a subtype of value
    /// kind `subtype`, or of the multirange beside it when `multirange`;
    /// `None` for a kind no `RangeSubtype` is.
    pub fn new(subtype: &ArrayType, multirange: bool) -> Option<Self> {
        let codec: fn(ArrayType, bool) -> Self = match subtype {
            ArrayType::SmallInt => halves::<i16, i16>,
            ArrayType::Int => halves::<i32, i32>,
            ArrayType::BigInt => halves::<i64, i64>,
            ArrayType::Float => halves::<f32, f32>,
            ArrayType::Double => halves::<f64, f64>,
            ArrayType::Decimal => halves::<Decimal, ExactDecimal>,
            ArrayType::String => halves::<String, String>,
            ArrayType::Date => halves::<Date, Date>,
            ArrayType::Time => halves::<Time, Time>,
            ArrayType::DateTime => halves::<DateTime, DateTime>,
            ArrayType::DateTimeWithTimeZone => halves::<Timestamp, Timestamp>,
            #[cfg(feature = "with-uuid")]
            ArrayType::Uuid => halves::<uuid::Uuid, uuid::Uuid>,
            _ => return None,
        };
        Some(codec(subtype.clone(), multirange))
    }

    /// The value kind the bounds of a range over `subtype`, a PostgreSQL
    /// type, are, a domain read as the type it is over; `None` when no
    /// `RangeSubtype` reads `subtype`.
    pub fn subtype_kind(subtype: &Type) -> Option<ArrayType> {
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
            #[cfg(feature = "with-uuid")]
            Type::UUID => ArrayType::Uuid,
            _ => return None,
        })
    }

    /// The codec of `ty`, a range or multirange type a schema created; an
    /// error naming its subtype when no `RangeSubtype` reads that.
    pub fn of(ty: &Type) -> Result<Self, WireError> {
        let (subtype, multirange) = match ty.kind() {
            Kind::Range(subtype) => (subtype, false),
            Kind::Multirange(subtype) => (subtype, true),
            _ => return Err(format!("`{ty}` is not a range or multirange type").into()),
        };
        Self::subtype_kind(subtype)
            .and_then(|kind| Self::new(&kind, multirange))
            .ok_or_else(|| {
                let what = if multirange { "multirange" } else { "range" };
                format!(
                    "a {what} type a schema created over `{subtype}` has no value: \
                     no `RangeSubtype` reads `{subtype}`"
                )
                .into()
            })
    }

    /// The value kind of the subtype's values.
    pub fn subtype(&self) -> &ArrayType {
        &self.subtype
    }

    /// Whether this is the codec of a multirange.
    pub fn is_multirange(&self) -> bool {
        self.multirange
    }

    /// The text form of `ranges`, each bound a value of the subtype's kind:
    /// the first range for a range type, the multirange of them for a
    /// multirange type; `None` when a bound is no value of the subtype or a
    /// range type is given none.
    pub fn text(&self, ranges: Vec<Range<Value>>) -> Option<String> {
        (self.written)(ranges, self.multirange)
    }

    /// The ranges `text` spells, read with the subtype's own parsing as
    /// PostgreSQL's range input reads it — one for a range type, a
    /// multirange's for a multirange type — each bound the `Value` of the
    /// subtype's Rust type; `None` for text spelling none.
    pub fn ranges(&self, text: &str) -> Option<Vec<Range<Value>>> {
        (self.read)(text, self.multirange)
    }

    /// A value's binary form, each bound read by the codec of `subtype`, the
    /// PostgreSQL type the bounds are, as its text form.
    pub fn text_from_sql(&self, subtype: &Type, raw: &[u8]) -> Result<String, WireError> {
        (self.decoded)(subtype, raw, self.multirange)
    }
}

/// A result or COPY field of a range or multirange type a schema created, as
/// its text form, read through its [`CreatedRangeCodec`]: the `Value::String`
/// a `CreatedRange` newtype converts into.
// [spec:pgorm:def:sql.value.created-range+9]
#[derive(Debug)]
pub struct CreatedRangeText(pub String);

impl<'a> FromSql<'a> for CreatedRangeText {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, WireError> {
        let (Kind::Range(subtype) | Kind::Multirange(subtype)) = ty.kind() else {
            return Err(format!("`{ty}` is not a range or multirange type").into());
        };
        CreatedRangeCodec::of(ty)?
            .text_from_sql(subtype, raw)
            .map(Self)
    }

    fn accepts(ty: &Type) -> bool {
        (is_created_range(ty) || is_created_multirange(ty)) && CreatedRangeCodec::of(ty).is_ok()
    }
}

/// The range `text` spells in `ty`, a range type a schema created, each bound
/// the `Value` of the subtype's Rust type; an error for text that spells no
/// range of that subtype.
// [spec:pgorm:req:exec.cursor.binding-range+3]
pub(crate) fn created_range_bounds(ty: &Type, text: &str) -> Result<Range<Value>, WireError> {
    CreatedRangeCodec::of(ty)?
        .ranges(text)
        .and_then(|ranges| ranges.into_iter().next())
        .ok_or_else(|| format!("`{text}` is the text of no range of `{}`", subtype_of(ty)).into())
}

/// The multirange `text` spells in `ty`, a multirange type a schema created,
/// each bound the `Value` of the subtype's Rust type; an error for text that
/// spells no multirange of that subtype.
// [spec:pgorm:req:exec.cursor.binding-range+3]
pub(crate) fn created_multirange_ranges(
    ty: &Type,
    text: &str,
) -> Result<Multirange<Value>, WireError> {
    CreatedRangeCodec::of(ty)?
        .ranges(text)
        .map(Multirange::from)
        .ok_or_else(|| {
            format!(
                "`{text}` is the text of no multirange of `{}`",
                subtype_of(ty)
            )
            .into()
        })
}

/// The subtype a range or multirange type ranges over, for a message.
fn subtype_of(ty: &Type) -> &Type {
    match ty.kind() {
        Kind::Range(subtype) | Kind::Multirange(subtype) => base_type(subtype),
        _ => ty,
    }
}

/// `range` with each bound a `T`, or `None` when a bound is not one.
fn typed<T: RangeSubtype>(range: Range<Value>) -> Option<Range<T>> {
    let bound = |bound: Bound<Value>| -> Option<Bound<T>> {
        Some(match bound {
            Bound::Included(value) => Bound::Included(T::try_from(value).ok()?),
            Bound::Excluded(value) => Bound::Excluded(T::try_from(value).ok()?),
            Bound::Unbounded => Bound::Unbounded,
        })
    };
    Some(match range {
        Range::Empty => Range::Empty,
        Range::Bounds { lower, upper } => Range::new(bound(lower)?, bound(upper)?),
    })
}

/// The codec of ranges and multiranges over `T`, whose binary form is read
/// as `W`.
fn halves<T, W>(subtype: ArrayType, multirange: bool) -> CreatedRangeCodec
where
    T: RangeSubtype + From<W>,
    W: for<'a> FromSql<'a>,
{
    CreatedRangeCodec {
        subtype,
        multirange,
        decoded: |subtype, raw, multirange| {
            Ok(if multirange {
                multirange_from_sql::<W>(subtype, raw)?
                    .map(T::from)
                    .to_string()
            } else {
                range_from_sql::<W>(subtype, raw)?.map(T::from).to_string()
            })
        },
        written: |ranges, multirange| {
            let mut ranges = ranges.into_iter().map(typed::<T>);
            if multirange {
                Some(ranges.collect::<Option<Multirange<T>>>()?.to_string())
            } else {
                Some(ranges.next()??.to_string())
            }
        },
        read: |text, multirange| {
            let ranges: Vec<Range<T>> = if multirange {
                text.parse::<Multirange<T>>().ok()?.into_iter().collect()
            } else {
                vec![text.parse::<Range<T>>().ok()?]
            };
            Some(
                ranges
                    .into_iter()
                    .map(|range| range.map(Into::into))
                    .collect(),
            )
        },
    }
}

/// A boxed future, for the recursion through arrays and domains.
type Completing<'a> = Pin<Box<dyn Future<Output = Result<Option<Type>, Error>> + Send + 'a>>;

/// `ty` with each multirange type a schema created in it described as a
/// multirange over its subtype, or `None` when it holds none.
///
/// tokio-postgres learns a type's kind from `pg_range` joined on the *range*
/// type, so it reports such a multirange as a simple type with no subtype to
/// read or write its binary form by. The catalogue names the subtype of the
/// range the multirange was created beside, and the driver resolves that
/// subtype, a domain included. A domain's base and an array's member are
/// completed as the type itself is; a type `postgres-types` knows, or one
/// `pg_range` holds no multirange for, is as the driver described it.
// [spec:pgorm:req:conn.copy.rows+5]
pub(crate) fn completed_multiranges<'a, C>(client: &'a C, ty: &'a Type) -> Completing<'a>
where
    C: GenericClient,
{
    Box::pin(async move {
        let rebuilt =
            |kind| Type::new(ty.name().to_owned(), ty.oid(), kind, ty.schema().to_owned());
        Ok(match ty.kind() {
            Kind::Array(member) => completed_multiranges(client, member)
                .await?
                .map(|member| rebuilt(Kind::Array(member))),
            Kind::Domain(base) => completed_multiranges(client, base)
                .await?
                .map(|base| rebuilt(Kind::Domain(base))),
            Kind::Simple if Type::from_oid(ty.oid()).is_none() => {
                multirange_subtype(client, ty.oid())
                    .await?
                    .map(|subtype| rebuilt(Kind::Multirange(subtype)))
            }
            _ => None,
        })
    })
}

/// The subtype of the multirange type `oid`, resolved by the driver, or
/// `None` when `oid` is no multirange's.
///
/// A built-in subtype is known by its OID. Any other is resolved by
/// preparing a statement whose parameter is declared with it: the driver
/// describes a statement's parameters through its own type lookup, so the
/// subtype comes back as every other type the driver reports does.
// [spec:pgorm:req:conn.copy.rows+5]
async fn multirange_subtype<C: GenericClient>(client: &C, oid: Oid) -> Result<Option<Type>, Error> {
    let row = client
        .query_opt(
            "SELECT rngsubtype FROM pg_catalog.pg_range WHERE rngmultitypid = $1",
            &[&oid],
        )
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let subtype: Oid = row.try_get(0)?;
    if let Some(known) = Type::from_oid(subtype) {
        return Ok(Some(known));
    }
    let declared = Type::new(String::new(), subtype, Kind::Simple, String::new());
    let prepared = client.prepare_typed("SELECT $1", &[declared]).await?;
    Ok(prepared.params().first().cloned())
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use postgres_protocol::{IsNull, types as wire};

    use super::*;

    fn numeric_range(lower: &[u16], upper: &[u16]) -> Result<Vec<u8>, WireError> {
        let mut raw = BytesMut::new();
        let bound = |words: &[u16], out: &mut BytesMut| {
            out.extend(words.iter().flat_map(|word| word.to_be_bytes()));
            Ok(wire::RangeBound::Inclusive(IsNull::No))
        };
        wire::range_to_sql(|out| bound(lower, out), |out| bound(upper, out), &mut raw)?;
        Ok(raw.to_vec())
    }

    // [spec:pgorm:def:sql.value.created-range+9/test]    one codec, named by the subtype's
    // value kind, writes and reads the text form from `Value` bounds and reads the binary form,
    // a numeric bound exactly or not at all
    #[test]
    fn the_codec_is_named_by_the_subtypes_kind() -> Result<(), WireError> {
        let floats = CreatedRangeCodec::new(&ArrayType::Double, false).ok_or("no f64 codec")?;
        let range = Range::new(Bound::Included(Value::from(1.5)), Bound::Unbounded);
        assert_eq!(floats.text(vec![range.clone()]).as_deref(), Some("[1.5,)"));
        assert_eq!(floats.ranges("[1.5,)"), Some(vec![range]));
        assert_eq!(
            floats.text(vec![Range::from(Value::from(1)..Value::from(2))]),
            None
        );
        assert_eq!(floats.ranges("[a,b)"), None);
        let spans = CreatedRangeCodec::new(&ArrayType::Int, true).ok_or("no i32 codec")?;
        let ranges = vec![Range::from(Value::from(1)..Value::from(3)), Range::Empty];
        assert_eq!(spans.text(ranges).as_deref(), Some("{[1,3),empty}"));
        assert!(CreatedRangeCodec::new(&ArrayType::Bool, false).is_none());
        assert_eq!(
            CreatedRangeCodec::subtype_kind(&Type::VARCHAR),
            Some(ArrayType::String)
        );
        assert_eq!(CreatedRangeCodec::subtype_kind(&Type::BOOL), None);

        let numbers =
            CreatedRangeCodec::new(&ArrayType::Decimal, false).ok_or("no decimal codec")?;
        let one = [1, 0, 0, 0, 1];
        let raw = numeric_range(&one, &[1, 0, 0, 0, 2])?;
        assert_eq!(numbers.text_from_sql(&Type::NUMERIC, &raw)?, "[1,2]");
        // 1 + 1e-32 in base-10000 groups, which rust_decimal's decoder rounds.
        let rounded = [9, 0, 0, 32, 1, 0, 0, 0, 0, 0, 0, 0, 1];
        let raw = numeric_range(&one, &rounded)?;
        assert!(numbers.text_from_sql(&Type::NUMERIC, &raw).is_err());
        Ok(())
    }
}
