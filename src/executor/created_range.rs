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

use std::{future::Future, pin::Pin};

use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use pgorm_pool::GenericClient;
use pgorm_query::{Multirange, Range, RangeSubtype, Value, base_type};
use rust_decimal::Decimal;
use tokio_postgres::types::{FromSql, Kind, Oid, Type};

use super::query::{multirange_from_sql, range_from_sql};
use crate::Error;

type WireError = Box<dyn std::error::Error + Sync + Send>;

/// A range's or a multirange's binary form, read against its subtype, as its
/// text form.
type ToText = fn(&Type, &[u8]) -> Result<String, WireError>;

/// A range's text form as the range it spells, `None` for text spelling none.
type FromText = fn(&str) -> Option<Range<Value>>;

/// A multirange's text form as the multirange it spells, `None` for text
/// spelling none.
type FromMultiText = fn(&str) -> Option<Multirange<Value>>;

/// Whether `ty` is a range type a schema created: a range, and none of the
/// six built in, each of which has a `RangeType` and so a `Value::Range`.
// [spec:pgorm:def:sql.value.created-range+7]
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
// [spec:pgorm:def:sql.value.created-range+7]
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

/// A range or multirange type a schema created over a subtype a
/// `RangeSubtype` reads: its subtype, and the halves of the text form of a
/// range and a multirange of it through that Rust type.
#[derive(Clone, Copy)]
pub(crate) struct CreatedRangeCodec<'a> {
    subtype: &'a Type,
    range_text: ToText,
    multirange_text: ToText,
    bounds: FromText,
    ranges: FromMultiText,
}

impl<'a> CreatedRangeCodec<'a> {
    /// The codec of `ty`, a range or multirange type a schema created; an
    /// error naming its subtype when no `RangeSubtype` reads that, a domain
    /// being read as the type it is over.
    // [spec:pgorm:def:sql.value.created-range+7]
    pub(crate) fn of(ty: &'a Type) -> Result<Self, WireError> {
        let (Kind::Range(subtype) | Kind::Multirange(subtype)) = ty.kind() else {
            return Err(format!("`{ty}` is not a range or multirange type").into());
        };
        let base = base_type(subtype);
        let codec: fn(&Type) -> CreatedRangeCodec<'_> = match *base {
            Type::INT2 => halves::<i16>,
            Type::INT4 => halves::<i32>,
            Type::INT8 => halves::<i64>,
            Type::FLOAT4 => halves::<f32>,
            Type::FLOAT8 => halves::<f64>,
            Type::NUMERIC => halves::<Decimal>,
            Type::TEXT | Type::VARCHAR | Type::BPCHAR => halves::<String>,
            Type::DATE => halves::<Date>,
            Type::TIME => halves::<Time>,
            Type::TIMESTAMP => halves::<DateTime>,
            Type::TIMESTAMPTZ => halves::<Timestamp>,
            #[cfg(feature = "with-uuid")]
            Type::UUID => halves::<uuid::Uuid>,
            _ => {
                let what = match ty.kind() {
                    Kind::Multirange(_) => "multirange",
                    _ => "range",
                };
                return Err(format!(
                    "a {what} type a schema created over `{subtype}` has no value: \
                     no `RangeSubtype` reads `{subtype}`"
                )
                .into());
            }
        };
        Ok(codec(base))
    }
}

/// A field of `ty`, a range or multirange type a schema created, as its text
/// form: the binary form read through the subtype's codec, then written as
/// `Display` writes the range or multirange.
// [spec:pgorm:req:conn.copy.rows+4]
pub(crate) fn created_range_text(ty: &Type, raw: &[u8]) -> Result<String, WireError> {
    let codec = CreatedRangeCodec::of(ty)?;
    match ty.kind() {
        Kind::Multirange(_) => (codec.multirange_text)(codec.subtype, raw),
        _ => (codec.range_text)(codec.subtype, raw),
    }
}

/// The range `text` spells in `ty`, a range type a schema created, each bound
/// the `Value` of the subtype's Rust type; an error for text that spells no
/// range of that subtype.
// [spec:pgorm:req:exec.cursor.binding-range+3]
pub(crate) fn created_range_bounds(ty: &Type, text: &str) -> Result<Range<Value>, WireError> {
    let codec = CreatedRangeCodec::of(ty)?;
    (codec.bounds)(text)
        .ok_or_else(|| format!("`{text}` is the text of no range of `{}`", codec.subtype).into())
}

/// The multirange `text` spells in `ty`, a multirange type a schema created,
/// each bound the `Value` of the subtype's Rust type; an error for text that
/// spells no multirange of that subtype.
// [spec:pgorm:req:exec.cursor.binding-range+3]
pub(crate) fn created_multirange_ranges(
    ty: &Type,
    text: &str,
) -> Result<Multirange<Value>, WireError> {
    let codec = CreatedRangeCodec::of(ty)?;
    (codec.ranges)(text).ok_or_else(|| {
        format!(
            "`{text}` is the text of no multirange of `{}`",
            codec.subtype
        )
        .into()
    })
}

/// The codec over `subtype` for ranges and multiranges over `T`.
fn halves<T>(subtype: &Type) -> CreatedRangeCodec<'_>
where
    T: RangeSubtype + for<'a> FromSql<'a>,
{
    CreatedRangeCodec {
        subtype,
        range_text: |subtype, raw| Ok(range_from_sql::<T>(subtype, raw)?.to_string()),
        multirange_text: |subtype, raw| Ok(multirange_from_sql::<T>(subtype, raw)?.to_string()),
        bounds: |text| Some(text.parse::<Range<T>>().ok()?.map(Into::into)),
        ranges: |text| Some(text.parse::<Multirange<T>>().ok()?.map(Into::into)),
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
// [spec:pgorm:req:conn.copy.rows+4]
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
// [spec:pgorm:req:conn.copy.rows+4]
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
