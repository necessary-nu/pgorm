//! Range and multirange values: PostgreSQL's six built-in range types and the
//! multiranges built over them.

use std::ops::{self, Bound};

use jiff::{
    Timestamp,
    civil::{Date, DateTime},
};
use rust_decimal::Decimal;

use crate::{
    ArrayType, ColumnType, Nullable, RangeSubtype, Value, ValueType, ValueTypeError,
    value::with_array::NotU8,
};

/// Which of PostgreSQL's built-in range types a range is, named by the subtype
/// it ranges over.
///
/// The set is closed because PostgreSQL's is: these are the range types every
/// database has, each with a multirange of its own. A range type created with
/// `CREATE TYPE ... AS RANGE` has a name only its schema knows, so it has no
/// variant here; a value of one over one of these subtypes still binds and
/// decodes, because the wire format of a range is its subtype's.
// [spec:pgorm:def:sql.value.range+5]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RangeType {
    /// `int4range` / `int4multirange`, over `integer` (`i32`).
    Int4,
    /// `int8range` / `int8multirange`, over `bigint` (`i64`).
    Int8,
    /// `numrange` / `nummultirange`, over `numeric` (`Decimal`).
    Numeric,
    /// `daterange` / `datemultirange`, over `date` (`civil::Date`).
    Date,
    /// `tsrange` / `tsmultirange`, over `timestamp` (`civil::DateTime`).
    Timestamp,
    /// `tstzrange` / `tstzmultirange`, over `timestamptz` (`jiff::Timestamp`).
    TimestampTz,
}

impl RangeType {
    /// The catalogue name of the range type: `int4range`, `tstzrange`, ...
    // [spec:pgorm:def:sql.value.range+5]
    pub fn range_type_name(self) -> &'static str {
        match self {
            Self::Int4 => "int4range",
            Self::Int8 => "int8range",
            Self::Numeric => "numrange",
            Self::Date => "daterange",
            Self::Timestamp => "tsrange",
            Self::TimestampTz => "tstzrange",
        }
    }

    /// The catalogue name of the multirange over the range type:
    /// `int4multirange`, `tstzmultirange`, ...
    // [spec:pgorm:def:sql.value.range+5]
    pub fn multirange_type_name(self) -> &'static str {
        match self {
            Self::Int4 => "int4multirange",
            Self::Int8 => "int8multirange",
            Self::Numeric => "nummultirange",
            Self::Date => "datemultirange",
            Self::Timestamp => "tsmultirange",
            Self::TimestampTz => "tstzmultirange",
        }
    }
}

/// A range of values: the empty range, or every value between two bounds.
///
/// The empty range is a value of its own and not a pair of bounds, because no
/// pair of bounds is equal to it: `(,)` is every value, and the server writes
/// `[5,5)` back as `empty` while still refusing `[5,1)`. A bound is
/// [`Bound::Included`], [`Bound::Excluded`] or [`Bound::Unbounded`], and an
/// unbounded side is not the same thing as a side bounded by a subtype's own
/// `infinity`: `[2020-01-01,infinity]` contains `infinity`, and
/// `[2020-01-01,)` is past it.
///
/// Equality is structural, not PostgreSQL's. The server canonicalises a
/// discrete range when it stores one — `int4range` `[1,5]` comes back as
/// `[1,6)` — so a value read back can differ from the value written while
/// meaning the same set; nothing here evaluates the bounds, and an inverted
/// pair is the server's to refuse (`22000`).
///
/// The standard library's ranges convert to the bounds they spell:
///
/// ```
/// use std::ops::Bound;
/// use pgorm_query::Range;
///
/// assert_eq!(
///     Range::from(1..5),
///     Range::Bounds { lower: Bound::Included(1), upper: Bound::Excluded(5) },
/// );
/// assert_eq!(
///     Range::from(..=5),
///     Range::Bounds { lower: Bound::Unbounded, upper: Bound::Included(5) },
/// );
/// assert!(Range::<i32>::Empty.is_empty());
/// ```
///
/// Under the `serde` feature it serializes in serde's own enum form, each
/// bound as serde writes any `Bound`: `"Empty"`, or
/// `{"Bounds":{"lower":{"Included":1},"upper":"Unbounded"}}` in JSON. A
/// present bound is always an object holding its value, so no value the
/// subtype serializes to — `null` included — can be read as no bound.
// [spec:pgorm:def:sql.value.range+5]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub enum Range<T> {
    /// The range containing no value.
    Empty,
    /// The values between `lower` and `upper`, either of which may be
    /// unbounded.
    Bounds {
        /// The lower bound.
        lower: Bound<T>,
        /// The upper bound.
        upper: Bound<T>,
    },
}

impl<T> Range<T> {
    /// The values between two bounds.
    pub fn new(lower: Bound<T>, upper: Bound<T>) -> Self {
        Self::Bounds { lower, upper }
    }

    /// Whether this is the empty range. A range of bounds is not empty here
    /// even when it contains no value: that is the server's to decide.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// The lower bound, or `None` for the empty range.
    pub fn lower(&self) -> Option<&Bound<T>> {
        match self {
            Self::Empty => None,
            Self::Bounds { lower, .. } => Some(lower),
        }
    }

    /// The upper bound, or `None` for the empty range.
    pub fn upper(&self) -> Option<&Bound<T>> {
        match self {
            Self::Empty => None,
            Self::Bounds { upper, .. } => Some(upper),
        }
    }

    /// Convert each bound's value, keeping its inclusivity.
    pub fn map<U, F>(self, mut f: F) -> Range<U>
    where
        F: FnMut(T) -> U,
    {
        match self {
            Self::Empty => Range::Empty,
            Self::Bounds { lower, upper } => Range::Bounds {
                lower: lower.map(&mut f),
                upper: upper.map(&mut f),
            },
        }
    }
}

/// `a..b` is `[a,b)`.
impl<T> From<ops::Range<T>> for Range<T> {
    fn from(range: ops::Range<T>) -> Self {
        Self::new(Bound::Included(range.start), Bound::Excluded(range.end))
    }
}

/// `a..=b` is `[a,b]`.
impl<T> From<ops::RangeInclusive<T>> for Range<T> {
    fn from(range: ops::RangeInclusive<T>) -> Self {
        let (start, end) = range.into_inner();
        Self::new(Bound::Included(start), Bound::Included(end))
    }
}

/// `a..` is `[a,)`.
impl<T> From<ops::RangeFrom<T>> for Range<T> {
    fn from(range: ops::RangeFrom<T>) -> Self {
        Self::new(Bound::Included(range.start), Bound::Unbounded)
    }
}

/// `..b` is `(,b)`.
impl<T> From<ops::RangeTo<T>> for Range<T> {
    fn from(range: ops::RangeTo<T>) -> Self {
        Self::new(Bound::Unbounded, Bound::Excluded(range.end))
    }
}

/// `..=b` is `(,b]`.
impl<T> From<ops::RangeToInclusive<T>> for Range<T> {
    fn from(range: ops::RangeToInclusive<T>) -> Self {
        Self::new(Bound::Unbounded, Bound::Included(range.end))
    }
}

/// `..` is `(,)`, every value.
impl<T> From<ops::RangeFull> for Range<T> {
    fn from(_: ops::RangeFull) -> Self {
        Self::new(Bound::Unbounded, Bound::Unbounded)
    }
}

/// A multirange: a set of ranges of one subtype, written as a list.
///
/// It is its own type rather than a `Vec<Range<T>>` because PostgreSQL has
/// both, and they are different types: a `Vec` of ranges is an array,
/// `int4range[]`, while this is `int4multirange`. The server stores a
/// multirange sorted, with overlapping and adjacent ranges merged and empty
/// ones dropped, so `{[5,8), [1,3), [2,4), empty}` reads back as
/// `{[1,4),[5,8)}`; the list here is the one written, in the order written.
///
/// Under the `serde` feature it serializes as that list of ranges, a JSON
/// array, in the order written and with any empty range kept.
// [spec:pgorm:def:sql.value.range+5]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Multirange<T>(Vec<Range<T>>);

impl<T> Multirange<T> {
    /// The ranges, in order.
    pub fn iter(&self) -> impl Iterator<Item = &Range<T>> {
        self.0.iter()
    }

    /// Convert each range's bounds, keeping their inclusivity.
    pub fn map<U, F>(self, mut f: F) -> Multirange<U>
    where
        F: FnMut(T) -> U,
    {
        self.0.into_iter().map(|range| range.map(&mut f)).collect()
    }
}

/// The empty multirange, `{}`.
impl<T> Default for Multirange<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> From<Vec<Range<T>>> for Multirange<T> {
    fn from(ranges: Vec<Range<T>>) -> Self {
        Self(ranges)
    }
}

impl<T> FromIterator<Range<T>> for Multirange<T> {
    fn from_iter<I: IntoIterator<Item = Range<T>>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<T> IntoIterator for Multirange<T> {
    type Item = Range<T>;
    type IntoIter = std::vec::IntoIter<Range<T>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// A Rust type a built-in range ranges over: `i32`, `i64`, `Decimal`,
/// `civil::Date`, `civil::DateTime` and `jiff::Timestamp`.
///
/// Sealed, as every [`RangeSubtype`] is, because the answer it gives is one of
/// PostgreSQL's built-in range types and the set of those is closed: a type
/// with no built-in range would have to name one it is not.
// [spec:pgorm:def:sql.value.range+5]
pub trait RangeElement: RangeSubtype {
    /// The built-in range type over this subtype.
    fn range_type() -> RangeType;
}

macro_rules! range_element {
    ( $type: ty, $range: ident ) => {
        impl RangeElement for $type {
            fn range_type() -> RangeType {
                RangeType::$range
            }
        }
    };
}

range_element!(i32, Int4);
range_element!(i64, Int8);
range_element!(Decimal, Numeric);
range_element!(Date, Date);
range_element!(DateTime, Timestamp);
range_element!(Timestamp, TimestampTz);

/// One bound's value out of a [`Value`]: a NULL of the element's own variant
/// is no bound at all, as PostgreSQL's range constructors read a NULL
/// argument.
fn bound_value<T>(bound: Bound<Value>) -> Result<Bound<T>, ValueTypeError>
where
    T: RangeElement,
{
    match bound {
        Bound::Included(value) | Bound::Excluded(value) if value == T::null() => {
            Ok(Bound::Unbounded)
        }
        Bound::Included(value) => T::from_value(value).map(Bound::Included),
        Bound::Excluded(value) => T::from_value(value).map(Bound::Excluded),
        Bound::Unbounded => Ok(Bound::Unbounded),
    }
}

fn range_value<T>(range: Range<Value>) -> Result<Range<T>, ValueTypeError>
where
    T: RangeElement,
{
    match range {
        Range::Empty => Ok(Range::Empty),
        Range::Bounds { lower, upper } => Ok(Range::Bounds {
            lower: bound_value(lower)?,
            upper: bound_value(upper)?,
        }),
    }
}

// [spec:pgorm:def:sql.value.range+5]
impl<T> From<Range<T>> for Value
where
    T: RangeElement,
{
    fn from(range: Range<T>) -> Value {
        Value::Range(T::range_type(), Some(Box::new(range.map(Into::into))))
    }
}

impl<T> Nullable for Range<T>
where
    T: RangeElement,
{
    fn null() -> Value {
        Value::Range(T::range_type(), None)
    }
}

// [spec:pgorm:def:sql.value.range+5]
impl<T> ValueType for Range<T>
where
    T: RangeElement,
{
    fn from_value(v: Value) -> Result<Self, ValueTypeError> {
        match v {
            Value::Range(ty, Some(range)) if ty == T::range_type() => range_value(*range),
            _ => Err(ValueTypeError),
        }
    }

    fn type_name() -> String {
        format!("Range<{}>", T::type_name())
    }

    fn array_type() -> ArrayType {
        ArrayType::Range(T::range_type())
    }

    fn column_type() -> ColumnType {
        ColumnType::Range(T::range_type())
    }
}

// [spec:pgorm:def:sql.value.array+7]
impl<T> NotU8 for Range<T> where T: RangeElement {}

// [spec:pgorm:def:sql.value.array+7]
impl<T> NotU8 for Multirange<T> where T: RangeElement {}

// [spec:pgorm:def:sql.value.range+5]
impl<T> From<Multirange<T>> for Value
where
    T: RangeElement,
{
    fn from(multirange: Multirange<T>) -> Value {
        Value::Multirange(T::range_type(), Some(Box::new(multirange.map(Into::into))))
    }
}

impl<T> Nullable for Multirange<T>
where
    T: RangeElement,
{
    fn null() -> Value {
        Value::Multirange(T::range_type(), None)
    }
}

// [spec:pgorm:def:sql.value.range+5]
impl<T> ValueType for Multirange<T>
where
    T: RangeElement,
{
    fn from_value(v: Value) -> Result<Self, ValueTypeError> {
        match v {
            Value::Multirange(ty, Some(multirange)) if ty == T::range_type() => {
                multirange.0.into_iter().map(range_value).collect()
            }
            _ => Err(ValueTypeError),
        }
    }

    fn type_name() -> String {
        format!("Multirange<{}>", T::type_name())
    }

    fn array_type() -> ArrayType {
        ArrayType::Multirange(T::range_type())
    }

    fn column_type() -> ColumnType {
        ColumnType::Multirange(T::range_type())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // [spec:pgorm:def:sql.value.range+5/test]
    #[test]
    fn std_ranges_spell_their_bounds() {
        use Bound::{Excluded, Included, Unbounded};

        assert_eq!(Range::from(1..5), Range::new(Included(1), Excluded(5)));
        assert_eq!(Range::from(1..=5), Range::new(Included(1), Included(5)));
        assert_eq!(Range::from(1..), Range::new(Included(1), Unbounded));
        assert_eq!(Range::from(..5), Range::new(Unbounded, Excluded(5)));
        assert_eq!(Range::from(..=5), Range::new(Unbounded, Included(5)));
        assert_eq!(Range::<i32>::from(..), Range::new(Unbounded, Unbounded));
    }

    // [spec:pgorm:def:sql.value.range+5/test]
    #[test]
    fn the_empty_range_is_not_every_value() {
        let empty = Range::<i32>::Empty;
        let everything = Range::<i32>::from(..);
        assert_ne!(empty, everything);
        assert!(empty.is_empty());
        assert!(!everything.is_empty());
        assert_eq!(empty.lower(), None);
        assert_eq!(everything.lower(), Some(&Bound::Unbounded));
        assert_ne!(Value::from(empty), Value::from(everything));
    }

    // [spec:pgorm:def:sql.value.range+5/test]
    #[test]
    fn a_range_round_trips_through_value() {
        let range = Range::from(Decimal::new(150, 2)..Decimal::new(3, 0));
        let value = Value::from(range.clone());
        assert_eq!(
            value,
            Value::Range(
                RangeType::Numeric,
                Some(Box::new(Range::new(
                    Bound::Included(Value::Decimal(Some(Box::new(Decimal::new(150, 2))))),
                    Bound::Excluded(Value::Decimal(Some(Box::new(Decimal::new(3, 0))))),
                )))
            )
        );
        assert_eq!(
            <Range<Decimal> as ValueType>::from_value(value).unwrap(),
            range
        );
        assert_eq!(
            <Range<i32> as ValueType>::from_value(Value::from(Range::<i32>::Empty)).unwrap(),
            Range::Empty
        );
    }

    // [spec:pgorm:def:sql.value.range+5/test]
    #[test]
    fn a_range_of_another_subtype_is_a_mismatch() {
        let value = Value::from(Range::from(1i64..2));
        assert!(<Range<i32> as ValueType>::from_value(value.clone()).is_err());
        assert!(<Multirange<i64> as ValueType>::from_value(value).is_err());
        // An empty range has no bound to tell the subtype by, so the tag is
        // what refuses it.
        let empty = Value::from(Range::<i64>::Empty);
        assert!(<Range<i32> as ValueType>::from_value(empty).is_err());
    }

    // [spec:pgorm:def:sql.value.range+5/test]
    #[test]
    fn a_null_bound_is_no_bound() {
        let value = Value::Range(
            RangeType::Int4,
            Some(Box::new(Range::new(
                Bound::Included(Value::Int(None)),
                Bound::Excluded(Value::Int(Some(5))),
            ))),
        );
        assert_eq!(
            <Range<i32> as ValueType>::from_value(value).unwrap(),
            Range::from(..5)
        );
    }

    // [spec:pgorm:def:sql.value.range+5/test]
    #[test]
    fn a_typed_null_keeps_its_range_type() {
        assert_eq!(
            Value::from(None::<Range<i32>>),
            Value::Range(RangeType::Int4, None)
        );
        assert_eq!(
            <Option<Range<i32>> as ValueType>::from_value(Value::Range(RangeType::Int4, None))
                .unwrap(),
            None
        );
        assert!(
            <Option<Range<i32>> as ValueType>::from_value(Value::Range(RangeType::Int8, None))
                .is_err()
        );
        assert_eq!(
            Value::from(None::<Multirange<Date>>),
            Value::Multirange(RangeType::Date, None)
        );
    }

    // [spec:pgorm:def:sql.value.range+5/test]
    #[test]
    fn a_multirange_round_trips_through_value() {
        let multirange: Multirange<i32> = [Range::from(5..8), Range::Empty, Range::from(1..3)]
            .into_iter()
            .collect();
        let value = Value::from(multirange.clone());
        assert!(matches!(value, Value::Multirange(RangeType::Int4, Some(_))));
        assert_eq!(
            <Multirange<i32> as ValueType>::from_value(value).unwrap(),
            multirange
        );
        assert_eq!(
            <Multirange<i32> as ValueType>::from_value(Value::from(Multirange::<i32>::default()))
                .unwrap(),
            Multirange::default()
        );
    }

    // [spec:pgorm:def:sql.value.array+7/test]    a `Vec` of ranges or multiranges is an array of
    // them, tagged with the range type and refusing another's
    #[test]
    fn a_vec_of_ranges_is_an_array() {
        let ranges = vec![Range::from(1i32..3), Range::Empty];
        let value = Value::from(ranges.clone());
        assert_eq!(
            value,
            Value::Array(
                ArrayType::Range(RangeType::Int4),
                Some(Box::new(vec![
                    Range::from(1i32..3).into(),
                    Range::<i32>::Empty.into()
                ]))
            )
        );
        assert_eq!(
            <Vec<Range<i32>> as ValueType>::from_value(value.clone()).unwrap(),
            ranges
        );
        assert!(<Vec<Range<i64>> as ValueType>::from_value(value.clone()).is_err());
        assert!(<Vec<Multirange<i32>> as ValueType>::from_value(value).is_err());
        assert_eq!(
            Value::from(None::<Vec<Range<i32>>>),
            Value::Array(ArrayType::Range(RangeType::Int4), None)
        );
        let multiranges = vec![Multirange::<Date>::default()];
        assert_eq!(
            <Vec<Multirange<Date>> as ValueType>::from_value(Value::from(multiranges.clone()))
                .unwrap(),
            multiranges
        );
        assert_eq!(
            <Vec<Multirange<Date>>>::column_type(),
            ColumnType::Array(std::sync::Arc::new(ColumnType::Multirange(RangeType::Date)))
        );
    }

    // [spec:pgorm:def:sql.value.range+5/test]
    #[test]
    fn each_subtype_names_its_range_type() {
        assert_eq!(
            <Range<i32>>::column_type(),
            ColumnType::Range(RangeType::Int4)
        );
        assert_eq!(
            <Range<i64>>::column_type(),
            ColumnType::Range(RangeType::Int8)
        );
        assert_eq!(
            <Range<Decimal>>::column_type(),
            ColumnType::Range(RangeType::Numeric)
        );
        assert_eq!(
            <Range<Date>>::column_type(),
            ColumnType::Range(RangeType::Date)
        );
        assert_eq!(
            <Range<DateTime>>::column_type(),
            ColumnType::Range(RangeType::Timestamp)
        );
        assert_eq!(
            <Multirange<Timestamp>>::column_type(),
            ColumnType::Multirange(RangeType::TimestampTz)
        );
        assert_ne!(
            ColumnType::Range(RangeType::Int4),
            ColumnType::Range(RangeType::Int8)
        );
        assert_ne!(
            ColumnType::Range(RangeType::Int4),
            ColumnType::Multirange(RangeType::Int4)
        );
        assert_eq!(
            <Range<i32>>::array_type(),
            ArrayType::Range(RangeType::Int4)
        );
        assert_eq!(<Range<i32>>::type_name(), "Range<i32>");
    }

    // [spec:pgorm:def:sql.value.range+5/test]
    #[cfg(feature = "serde")]
    #[test]
    fn serde_writes_a_range_in_enum_form() -> Result<(), serde_json::Error> {
        use Bound::{Excluded, Unbounded};

        for (range, json) in [
            (Range::<i32>::Empty, r#""Empty""#),
            (
                Range::from(1..5),
                r#"{"Bounds":{"lower":{"Included":1},"upper":{"Excluded":5}}}"#,
            ),
            (
                Range::from(1..=5),
                r#"{"Bounds":{"lower":{"Included":1},"upper":{"Included":5}}}"#,
            ),
            (
                Range::new(Excluded(1), Unbounded),
                r#"{"Bounds":{"lower":{"Excluded":1},"upper":"Unbounded"}}"#,
            ),
            (
                Range::from(..=5),
                r#"{"Bounds":{"lower":"Unbounded","upper":{"Included":5}}}"#,
            ),
            (
                Range::from(..),
                r#"{"Bounds":{"lower":"Unbounded","upper":"Unbounded"}}"#,
            ),
        ] {
            assert_eq!(serde_json::to_string(&range)?, json);
            assert_eq!(serde_json::from_str::<Range<i32>>(json)?, range);
        }
        // SQL NULL, the empty range and every value are three values.
        assert_eq!(serde_json::to_string(&None::<Range<i32>>)?, "null");
        assert_eq!(
            serde_json::from_str::<Option<Range<i32>>>(r#""Empty""#)?,
            Some(Range::Empty)
        );
        Ok(())
    }

    // [spec:pgorm:def:sql.value.range+5/test]
    #[cfg(feature = "serde")]
    #[test]
    fn a_bound_value_never_reads_as_unbounded() -> Result<(), serde_json::Error> {
        use Bound::{Excluded, Included, Unbounded};

        // serde_json writes a non-finite float as `null`; the bound around it
        // is still an object, so reading it back fails instead of widening
        // `[0,Infinity)` to `[0,)`.
        let infinite = Range::new(Included(0.0f64), Excluded(f64::INFINITY));
        let text = serde_json::to_string(&infinite)?;
        assert_eq!(
            text,
            r#"{"Bounds":{"lower":{"Included":0.0},"upper":{"Excluded":null}}}"#
        );
        assert!(serde_json::from_str::<Range<f64>>(&text).is_err());

        let nullable: Range<Option<i32>> = Range::new(Included(None), Unbounded);
        let text = serde_json::to_string(&nullable)?;
        assert_eq!(serde_json::from_str::<Range<Option<i32>>>(&text)?, nullable);

        // A text bound holding range syntax is only its value.
        let syntax = Range::new(Included("[a,b)".to_owned()), Excluded(r#"" \"#.to_owned()));
        let text = serde_json::to_string(&syntax)?;
        assert_eq!(serde_json::from_str::<Range<String>>(&text)?, syntax);

        for text in [
            r#""empty""#,
            r#"{"Bounds":{"lower":"Unbounded"}}"#,
            r#"{"Bounds":{"lower":"Unbounded","upper":"Unbounded","x":1}}"#,
            r#"{"Bounds":{"lower":null,"upper":"Unbounded"}}"#,
            r#"{"Bounds":{"lower":1,"upper":"Unbounded"}}"#,
            r#""[1,5)""#,
        ] {
            assert!(serde_json::from_str::<Range<i32>>(text).is_err(), "{text}");
        }
        Ok(())
    }

    // [spec:pgorm:def:sql.value.range+5/test]
    #[cfg(feature = "serde")]
    #[test]
    fn serde_writes_a_multirange_as_its_list() -> Result<(), serde_json::Error> {
        let multirange: Multirange<i32> = [Range::from(5..8), Range::Empty, Range::from(1..3)]
            .into_iter()
            .collect();
        let json = concat!(
            r#"[{"Bounds":{"lower":{"Included":5},"upper":{"Excluded":8}}},"#,
            r#""Empty","#,
            r#"{"Bounds":{"lower":{"Included":1},"upper":{"Excluded":3}}}]"#,
        );
        assert_eq!(serde_json::to_string(&multirange)?, json);
        assert_eq!(serde_json::from_str::<Multirange<i32>>(json)?, multirange);
        assert_eq!(serde_json::to_string(&Multirange::<i32>::default())?, "[]");
        assert_eq!(
            serde_json::from_str::<Multirange<i32>>("[]")?,
            Multirange::default()
        );
        assert!(serde_json::from_str::<Multirange<i32>>(r#""Empty""#).is_err());
        Ok(())
    }
}
