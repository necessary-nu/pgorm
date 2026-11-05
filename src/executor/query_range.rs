//! Decoding a range or a multirange column into [`Range`] / [`Multirange`].
//!
//! pgorm-query owns the two types but not the wire crates, so it has no
//! `FromSql` for them, and the orphan rule forbids writing one here. Decoding
//! routes through local newtypes instead, as `InetSql` does for an address,
//! and each bound is decoded by its subtype's own `FromSql`.

use std::ops::Bound;

use pgorm_query::{Multirange, Range};
use postgres_protocol::types::{self as wire, RangeBound};
use tokio_postgres::{
    row::RowIndex,
    types::{FromSql, Kind, Type},
};

use super::{QueryResult, TryFromU64, TryGetError, TryGetable};

type WireResult<T> = Result<T, Box<dyn std::error::Error + Sync + Send>>;

/// A [`Range`] read from the wire, over any subtype a `FromSql` reads: the
/// `FromSql` a range column decodes through, which the language bindings
/// read a built-in range with.
// [spec:pgorm:def:exec.decode.range+4]
#[derive(Debug)]
pub struct RangeSql<T>(pub Range<T>);

// [spec:pgorm:def:exec.decode.range+4]
impl<'a, T> FromSql<'a> for RangeSql<T>
where
    T: FromSql<'a>,
{
    fn from_sql(ty: &Type, raw: &'a [u8]) -> WireResult<Self> {
        match ty.kind() {
            Kind::Range(subtype) => range_from_sql(subtype, raw).map(Self),
            _ => Err(format!("`{ty}` is not a range type").into()),
        }
    }

    fn accepts(ty: &Type) -> bool {
        matches!(ty.kind(), Kind::Range(subtype) if T::accepts(subtype))
    }
}

/// A [`Multirange`] read from the wire: a count of ranges, each
/// length-prefixed in a range's own encoding.
// [spec:pgorm:def:exec.decode.range+4]
#[derive(Debug)]
pub struct MultirangeSql<T>(pub Multirange<T>);

// [spec:pgorm:def:exec.decode.range+4]
impl<'a, T> FromSql<'a> for MultirangeSql<T>
where
    T: FromSql<'a>,
{
    fn from_sql(ty: &Type, raw: &'a [u8]) -> WireResult<Self> {
        let Kind::Multirange(subtype) = ty.kind() else {
            return Err(format!("`{ty}` is not a multirange type").into());
        };
        multirange_from_sql(subtype, raw).map(Self)
    }

    fn accepts(ty: &Type) -> bool {
        matches!(ty.kind(), Kind::Multirange(subtype) if T::accepts(subtype))
    }
}

/// A multirange over `subtype` from its binary form, each bound read by
/// `T`'s `FromSql` for `subtype`.
// [spec:pgorm:def:exec.decode.range+4]
pub fn multirange_from_sql<'a, T>(subtype: &Type, raw: &'a [u8]) -> WireResult<Multirange<T>>
where
    T: FromSql<'a>,
{
    multirange_from_sql_with(raw, |raw| T::from_sql(subtype, raw))
}

/// A range over `subtype` from its binary form, each bound read by `T`'s
/// `FromSql` for `subtype`.
// [spec:pgorm:def:exec.decode.range+4]
pub fn range_from_sql<'a, T>(subtype: &Type, raw: &'a [u8]) -> WireResult<Range<T>>
where
    T: FromSql<'a>,
{
    range_from_sql_with(raw, |raw| T::from_sql(subtype, raw))
}

/// A multirange from its binary form: a count of ranges, each
/// length-prefixed in a range's own encoding ([`range_from_sql_with`]), and
/// nothing after the last.
// [spec:pgorm:def:exec.decode.range+4]
pub fn multirange_from_sql_with<'a, B>(
    raw: &'a [u8],
    mut bound: impl FnMut(&'a [u8]) -> WireResult<B>,
) -> WireResult<Multirange<B>> {
    let mut raw = raw;
    let count = usize::try_from(read_i32(&mut raw)?)?;
    let ranges = (0..count)
        .map(|_| {
            let len = usize::try_from(read_i32(&mut raw)?)?;
            let (range, rest) = raw.split_at_checked(len).ok_or("invalid message size")?;
            raw = rest;
            range_from_sql_with(range, &mut bound)
        })
        .collect::<WireResult<Multirange<B>>>()?;
    if !raw.is_empty() {
        return Err("invalid message size".into());
    }
    Ok(ranges)
}

fn read_i32(raw: &mut &[u8]) -> WireResult<i32> {
    let (head, rest) = raw.split_first_chunk::<4>().ok_or("invalid message size")?;
    *raw = rest;
    Ok(i32::from_be_bytes(*head))
}

/// A range from its binary form, each present bound's bytes read by
/// `bound`: the one reader of the format, which a `FromSql` subtype, pgorm's
/// COPY rows and the language bindings' `Value`s each read bounds through.
/// The empty flag decodes to [`Range::Empty`] and nothing else: a range with
/// no bounds is the range with no values, never the range of every value.
/// The server never sends a NULL bound, so one is a malformed message rather
/// than something to read as either.
// [spec:pgorm:def:exec.decode.range+4]
pub fn range_from_sql_with<'a, B>(
    raw: &'a [u8],
    mut bound: impl FnMut(&'a [u8]) -> WireResult<B>,
) -> WireResult<Range<B>> {
    let mut side = |side: RangeBound<Option<&'a [u8]>>| -> WireResult<Bound<B>> {
        Ok(match side {
            RangeBound::Inclusive(Some(raw)) => Bound::Included(bound(raw)?),
            RangeBound::Exclusive(Some(raw)) => Bound::Excluded(bound(raw)?),
            RangeBound::Unbounded => Bound::Unbounded,
            RangeBound::Inclusive(None) | RangeBound::Exclusive(None) => {
                return Err("a range bound is NULL".into());
            }
        })
    };
    Ok(match wire::range_from_sql(raw)? {
        wire::Range::Empty => Range::Empty,
        wire::Range::Nonempty(lower, upper) => Range::Bounds {
            lower: side(lower)?,
            upper: side(upper)?,
        },
    })
}

/// A range is no counter, so no `u64` stands for one, as one stands for a
/// text key's string; a temporal key holds one as its period.
// [spec:pgorm:def:exec.decode.from-u64+3]
impl<T> TryFromU64 for Range<T> {
    fn try_from_u64(_: u64) -> Result<Self, crate::Error> {
        Err(crate::Error::ConvertFromU64("Range"))
    }
}

/// A multirange, which a temporal key may hold as its period, is no counter
/// either.
// [spec:pgorm:def:exec.decode.from-u64+3]
impl<T> TryFromU64 for Multirange<T> {
    fn try_from_u64(_: u64) -> Result<Self, crate::Error> {
        Err(crate::Error::ConvertFromU64("Multirange"))
    }
}

/// A range over a subtype, read whatever range type the server reports over
/// it: a built-in one, or one a schema created, which tokio-postgres reports
/// as a range over its subtype all the same.
// [spec:pgorm:def:exec.decode.range+4]
macro_rules! try_getable_range {
    ( $type: ty ) => {
        impl TryGetable for Range<$type> {
            fn try_get_by<I: RowIndex + std::fmt::Display>(
                res: &QueryResult,
                idx: I,
            ) -> Result<Self, TryGetError> {
                let result: RangeSql<$type> =
                    res.row.try_get(idx).map_err(TryGetError::postgres)?;
                Ok(result.0)
            }

            // [spec:pgorm:sem:exec.verify.accepts+1]    the newtype that reads the wire format
            fn accepts(ty: &Type) -> bool {
                <RangeSql<$type> as FromSql>::accepts(ty)
            }
        }
    };
}

/// A built-in range's subtype: its range, its multirange, and an array of
/// either.
// [spec:pgorm:def:exec.decode.range+4]
macro_rules! try_getable_range_family {
    ( $type: ty ) => {
        try_getable_range!($type);

        impl TryGetable for Multirange<$type> {
            fn try_get_by<I: RowIndex + std::fmt::Display>(
                res: &QueryResult,
                idx: I,
            ) -> Result<Self, TryGetError> {
                let result: MultirangeSql<$type> =
                    res.row.try_get(idx).map_err(TryGetError::postgres)?;
                Ok(result.0)
            }

            // [spec:pgorm:sem:exec.verify.accepts+1]    the newtype that reads the wire format
            fn accepts(ty: &Type) -> bool {
                <MultirangeSql<$type> as FromSql>::accepts(ty)
            }
        }

        // [spec:pgorm:def:exec.decode.range+4]
        #[cfg(feature = "postgres-array")]
        impl TryGetable for Vec<Range<$type>> {
            fn try_get_by<I: RowIndex + std::fmt::Display>(
                res: &QueryResult,
                idx: I,
            ) -> Result<Self, TryGetError> {
                let result: Vec<RangeSql<$type>> =
                    res.row.try_get(idx).map_err(TryGetError::postgres)?;
                Ok(result.into_iter().map(|range| range.0).collect())
            }

            // [spec:pgorm:sem:exec.verify.accepts+1]    an array whose member the newtype reads
            fn accepts(ty: &Type) -> bool {
                <Vec<RangeSql<$type>> as FromSql>::accepts(ty)
            }
        }

        // [spec:pgorm:def:exec.decode.range+4]
        #[cfg(feature = "postgres-array")]
        impl TryGetable for Vec<Multirange<$type>> {
            fn try_get_by<I: RowIndex + std::fmt::Display>(
                res: &QueryResult,
                idx: I,
            ) -> Result<Self, TryGetError> {
                let result: Vec<MultirangeSql<$type>> =
                    res.row.try_get(idx).map_err(TryGetError::postgres)?;
                Ok(result.into_iter().map(|multirange| multirange.0).collect())
            }

            // [spec:pgorm:sem:exec.verify.accepts+1]    an array whose member the newtype reads
            fn accepts(ty: &Type) -> bool {
                <Vec<MultirangeSql<$type>> as FromSql>::accepts(ty)
            }
        }
    };
}

try_getable_range_family!(i32);
try_getable_range_family!(i64);
try_getable_range_family!(rust_decimal::Decimal);

#[cfg(feature = "with-jiff")]
try_getable_range_family!(jiff::civil::Date);

#[cfg(feature = "with-jiff")]
try_getable_range_family!(jiff::civil::DateTime);

#[cfg(feature = "with-jiff")]
try_getable_range_family!(jiff::Timestamp);

/// An array of a range type a schema created, each element decoded through
/// its subtype's codec as `Range<T>` decodes one, and a NULL element refused,
/// as in every array. The `CreatedRange` derive reads an array column of its
/// type through this rather than `Vec<Range<T>>`, which a crate built without
/// `postgres-array` lacks and which only a built-in range's subtype has.
// [spec:pgorm:def:sql.value.created-range+9]
#[doc(hidden)]
pub fn try_get_created_ranges<T, I>(res: &QueryResult, idx: I) -> Result<Vec<Range<T>>, TryGetError>
where
    T: for<'a> FromSql<'a>,
    I: RowIndex + std::fmt::Display,
{
    #[cfg(feature = "postgres-array")]
    {
        let result: Vec<RangeSql<T>> = res.row.try_get(idx).map_err(TryGetError::postgres)?;
        Ok(result.into_iter().map(|range| range.0).collect())
    }
    #[cfg(not(feature = "postgres-array"))]
    {
        let _ = (res, idx);
        Err(TryGetError::Db(crate::Error::Type(
            "reading an array of a created range requires the `postgres-array` feature".to_owned(),
        )))
    }
}

/// An array column read as the texts of its elements: what the
/// `CreatedRange` derive reads an array of a created multirange through, the
/// driver reporting the multirange with no subtype, so the column's
/// `select_as` casts it to `text[]`.
// [spec:pgorm:def:sql.value.created-range+9]
#[doc(hidden)]
pub fn try_get_texts<I>(res: &QueryResult, idx: I) -> Result<Vec<String>, TryGetError>
where
    I: RowIndex + std::fmt::Display,
{
    #[cfg(feature = "postgres-array")]
    {
        <Vec<String> as TryGetable>::try_get_by(res, idx)
    }
    #[cfg(not(feature = "postgres-array"))]
    {
        let _ = (res, idx);
        Err(TryGetError::Db(crate::Error::Type(
            "reading an array of a created multirange requires the `postgres-array` feature"
                .to_owned(),
        )))
    }
}

// The subtypes only a range type a schema created ranges over. Their
// multiranges are reported by tokio-postgres as simple types, and an array of
// their ranges is read by `try_get_created_ranges`, so the range alone
// decodes here.
// [spec:pgorm:def:sql.value.created-range+9]
try_getable_range!(i16);
try_getable_range!(f32);
try_getable_range!(f64);
try_getable_range!(String);

#[cfg(feature = "with-jiff")]
try_getable_range!(jiff::civil::Time);

#[cfg(feature = "with-uuid")]
try_getable_range!(uuid::Uuid);

#[cfg(test)]
mod tests {
    use super::*;

    fn int4(value: i32) -> Vec<u8> {
        [&4i32.to_be_bytes()[..], &value.to_be_bytes()[..]].concat()
    }

    fn decode(raw: &[u8]) -> WireResult<Range<i32>> {
        RangeSql::<i32>::from_sql(&Type::INT4_RANGE, raw).map(|range| range.0)
    }

    // [spec:pgorm:def:exec.decode.range+4/test]
    #[test]
    fn decodes_each_bound_with_its_inclusivity() {
        assert_eq!(
            decode(&[&[0b0000_0010][..], &int4(1), &int4(6)].concat()).unwrap(),
            Range::from(1..6)
        );
        assert_eq!(
            decode(&[&[0b0000_0100][..], &int4(1), &int4(6)].concat()).unwrap(),
            Range::new(Bound::Excluded(1), Bound::Included(6))
        );
        assert_eq!(
            decode(&[&[0b0001_0010][..], &int4(1)].concat()).unwrap(),
            Range::from(1..)
        );
        assert_eq!(decode(&[0b0001_1000]).unwrap(), Range::from(..));
    }

    // [spec:pgorm:def:exec.decode.range+4/test]
    #[test]
    fn decodes_the_empty_flag_as_the_empty_range() {
        assert_eq!(decode(&[0b0000_0001]).unwrap(), Range::Empty);
        assert_ne!(decode(&[0b0000_0001]).unwrap(), Range::from(..));
    }

    // [spec:pgorm:def:exec.decode.range+4/test]
    #[test]
    fn refuses_a_malformed_range() {
        let null_bound = [&[0b0000_0010][..], &(-1i32).to_be_bytes()[..], &int4(6)].concat();
        assert_eq!(
            decode(&null_bound).unwrap_err().to_string(),
            "a range bound is NULL"
        );
        assert!(decode(&[0b0000_0010, 0, 0, 0, 4, 0]).is_err());
        assert!(decode(&[0b0000_0001, 0]).is_err());
    }

    // [spec:pgorm:def:exec.decode.range+4/test]
    #[test]
    fn decodes_a_multirange_range_by_range() {
        let first = [&[0b0000_0010][..], &int4(1), &int4(4)].concat();
        let second = [&[0b0000_0010][..], &int4(5), &int4(8)].concat();
        let raw = [
            &2i32.to_be_bytes()[..],
            &i32::try_from(first.len()).unwrap().to_be_bytes()[..],
            &first,
            &i32::try_from(second.len()).unwrap().to_be_bytes()[..],
            &second,
        ]
        .concat();
        let decoded = MultirangeSql::<i32>::from_sql(&Type::INT4MULTI_RANGE, &raw)
            .unwrap()
            .0;
        assert_eq!(
            decoded,
            [Range::from(1..4), Range::from(5..8)].into_iter().collect()
        );
        let empty = MultirangeSql::<i32>::from_sql(&Type::INT4MULTI_RANGE, &0i32.to_be_bytes())
            .unwrap()
            .0;
        assert_eq!(empty, Multirange::default());
        assert!(
            MultirangeSql::<i32>::from_sql(&Type::INT4MULTI_RANGE, &raw[..raw.len() - 1]).is_err()
        );
    }

    // [spec:pgorm:sem:exec.verify.accepts+1/test]
    #[test]
    fn accepts_the_range_types_over_its_subtype() {
        let created = Type::new(
            "slot".to_owned(),
            16_384,
            Kind::Range(Type::INT4),
            "public".to_owned(),
        );
        assert!(<Range<i32> as TryGetable>::accepts(&Type::INT4_RANGE));
        assert!(<Range<i32> as TryGetable>::accepts(&created));
        assert!(!<Range<i32> as TryGetable>::accepts(&Type::INT8_RANGE));
        assert!(!<Range<i32> as TryGetable>::accepts(&Type::INT4MULTI_RANGE));
        assert!(!<Range<i32> as TryGetable>::accepts(&Type::INT4));
        assert!(<Multirange<i64> as TryGetable>::accepts(
            &Type::INT8MULTI_RANGE
        ));
        assert!(!<Multirange<i64> as TryGetable>::accepts(&Type::INT8_RANGE));
        assert!(<Range<rust_decimal::Decimal> as TryGetable>::accepts(
            &Type::NUM_RANGE
        ));
    }
}
