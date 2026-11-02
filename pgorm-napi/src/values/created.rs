//! Range and multirange types a schema created. pgorm holds a value of one as
//! its text form, the `Value::String` a `CreatedRange` newtype converts
//! into, because no range tag can carry a name only the schema knows; the
//! subtype converts each bound and reads the text back.

use std::ops::Bound;

use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use pgorm::pgorm_query::{ArrayType, Multirange, Range, RangeSubtype, Value};
use rust_decimal::Decimal;
use uuid::Uuid;

/// Run a generic function at the Rust type of a subtype's value kind.
macro_rules! dispatch {
    ($subtype:expr, $f:ident, $($arg:expr),*) => {
        match $subtype {
            ArrayType::SmallInt => $f::<i16>($($arg),*),
            ArrayType::Int => $f::<i32>($($arg),*),
            ArrayType::BigInt => $f::<i64>($($arg),*),
            ArrayType::Float => $f::<f32>($($arg),*),
            ArrayType::Double => $f::<f64>($($arg),*),
            ArrayType::Decimal => $f::<Decimal>($($arg),*),
            ArrayType::Date => $f::<Date>($($arg),*),
            ArrayType::Time => $f::<Time>($($arg),*),
            ArrayType::DateTime => $f::<DateTime>($($arg),*),
            ArrayType::DateTimeWithTimeZone => $f::<Timestamp>($($arg),*),
            ArrayType::Uuid => $f::<Uuid>($($arg),*),
            _ => $f::<String>($($arg),*),
        }
    };
}

/// A range of subtype values as a range of `T`, or `None` if a bound is not
/// a `T`.
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

fn written<T: RangeSubtype>(ranges: Vec<Range<Value>>, multirange: bool) -> Option<String> {
    let ranges = ranges
        .into_iter()
        .map(typed::<T>)
        .collect::<Option<Vec<_>>>()?;
    if multirange {
        Some(Multirange::from(ranges).to_string())
    } else {
        ranges.into_iter().next().map(|range| range.to_string())
    }
}

fn parsed<T: RangeSubtype>(text: &str, multirange: bool) -> Option<Vec<Range<Value>>> {
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
}

/// The text form Rust's `Display` writes for these ranges, each bound a value
/// of `subtype`: one range, or a multirange of them.
pub(crate) fn text(
    subtype: &ArrayType,
    ranges: Vec<Range<Value>>,
    multirange: bool,
) -> Option<String> {
    dispatch!(subtype, written, ranges, multirange)
}

/// The ranges a text form holds, read with the subtype's own parsing as
/// PostgreSQL's range input reads it; `None` when it holds none.
pub(crate) fn ranges(
    subtype: &ArrayType,
    text: &str,
    multirange: bool,
) -> Option<Vec<Range<Value>>> {
    dispatch!(subtype, parsed, text, multirange)
}
