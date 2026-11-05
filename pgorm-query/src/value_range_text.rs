//! A range's text form, `[1.5,2.5)`: what PostgreSQL's range input function
//! reads, and so the one spelling a value of a range type a schema created can
//! travel as, cast to that type by name.
//!
//! The built-in ranges are written as calls of their constructors
//! (`int4range(1, 5, '[)')`), each bound rendered as the literal of its own
//! type. A range type a schema created has a constructor too, but no variant
//! in [`RangeType`](crate::RangeType) to carry its name, and there is no cast
//! between two range types, so a value of one is written as its text and cast
//! from `text` to the type the caller names. Each bound is rendered as its
//! subtype's text and quoted where the range parser needs it, so a bound is
//! data inside the literal and the literal is a value — bound as a parameter
//! or escaped as a string — never SQL.

use std::{fmt, str::FromStr};

use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{Multirange, Nullable, Range, Value, ValueType, ValueTypeError};

pub(crate) mod sealed {
    /// The half of [`RangeSubtype`](super::RangeSubtype) a caller cannot
    /// implement: reading one bound back from its text.
    pub trait Sealed: Sized {
        /// The subtype's value a bound's text holds, or `None` when it holds
        /// none.
        fn parse_bound(text: &str) -> Option<Self>;
    }
}

/// A Rust type a range ranges over, in a built-in range type or in one a
/// schema created: `i16`, `i32`, `i64`, `f32`, `f64`, `Decimal`, `String`,
/// `civil::Date`, `civil::Time`, `civil::DateTime`, `jiff::Timestamp` and
/// `Uuid`.
///
/// Each is a scalar PostgreSQL can range over — its type has a default b-tree
/// operator class — whose text form the type's input function reads back as
/// the same value and whose wire form pgorm decodes, so a range over it can
/// be written as text and read either way. Sealed, because the set is what
/// those three facts were checked for.
// [spec:pgorm:def:sql.value.created-range+6]
pub trait RangeSubtype: ValueType + Nullable + Into<Value> + Clone + sealed::Sealed {}

macro_rules! range_subtype {
    ( $type: ty, |$text: ident| $parse: expr ) => {
        impl sealed::Sealed for $type {
            fn parse_bound($text: &str) -> Option<Self> {
                $parse
            }
        }

        impl RangeSubtype for $type {}
    };
}

range_subtype!(i16, |text| text.trim().parse().ok());
range_subtype!(i32, |text| text.trim().parse().ok());
range_subtype!(i64, |text| text.trim().parse().ok());
range_subtype!(f32, |text| text.trim().parse().ok());
range_subtype!(f64, |text| text.trim().parse().ok());
range_subtype!(Decimal, |text| Decimal::from_str(text.trim()).ok());
range_subtype!(String, |text| Some(text.to_owned()));
range_subtype!(Date, |text| text.trim().parse().ok());
range_subtype!(Time, |text| text.trim().parse().ok());
range_subtype!(DateTime, |text| text.trim().parse().ok());
range_subtype!(Timestamp, |text| text.trim().parse().ok());
range_subtype!(Uuid, |text| Uuid::parse_str(text.trim()).ok());

/// The bytes PostgreSQL's range parser treats as whitespace (C's `isspace`).
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0B' | '\x0C' | '\r')
}

/// Truncation to the microsecond PostgreSQL stores, as the literal rendering
/// does it.
macro_rules! microseconds {
    ($rounder:path) => {
        <$rounder>::new()
            .smallest(jiff::Unit::Microsecond)
            .mode(jiff::RoundMode::Trunc)
    };
}

/// The text a value's type reads back as that value, or `None` for `NULL`.
///
/// Every scalar is written as its type's input function reads it, the
/// temporal ones as the literal rendering writes them and truncated to the
/// microsecond for the same reason; an array, a range and a multirange are
/// written in their own text forms, so a hand-built range whose bound is one
/// still has a spelling.
// [spec:pgorm:def:sql.value.created-range+6]
pub(crate) fn value_text(value: &Value) -> Option<String> {
    Some(match value {
        Value::Bool(v) => if (*v)? { "t" } else { "f" }.to_owned(),
        Value::TinyInt(v) => (*v)?.to_string(),
        Value::SmallInt(v) => (*v)?.to_string(),
        Value::Int(v) => (*v)?.to_string(),
        Value::BigInt(v) => (*v)?.to_string(),
        Value::Unsigned(v) => (*v)?.to_string(),
        Value::BigUnsigned(v) => (*v)?.to_string(),
        Value::Float(v) => (*v)?.to_string(),
        Value::Double(v) => (*v)?.to_string(),
        Value::String(v) => v.as_deref()?.clone(),
        Value::Char(v) => (*v)?.to_string(),
        Value::Bytes(v) => v
            .as_deref()?
            .iter()
            .fold("\\x".to_owned(), |mut hex, byte| {
                hex.push_str(&format!("{byte:02x}"));
                hex
            }),
        Value::Json(v) => v.as_deref()?.to_string(),
        Value::Date(v) => v.as_deref()?.strftime("%Y-%m-%d").to_string(),
        Value::Time(v) => {
            let v = v.as_deref()?;
            let v = v.round(microseconds!(jiff::civil::TimeRound)).unwrap_or(*v);
            v.strftime("%H:%M:%S%.f").to_string()
        }
        Value::DateTime(v) => {
            let v = v.as_deref()?;
            let v = v
                .round(microseconds!(jiff::civil::DateTimeRound))
                .unwrap_or(*v);
            v.strftime("%Y-%m-%d %H:%M:%S%.f").to_string()
        }
        Value::DateTimeWithTimeZone(v) => {
            let v = v.as_deref()?;
            let v = v.round(microseconds!(jiff::TimestampRound)).unwrap_or(*v);
            v.strftime("%Y-%m-%d %H:%M:%S%.f%:z").to_string()
        }
        Value::Uuid(v) => v.as_deref()?.to_string(),
        Value::Decimal(v) => v.as_deref()?.to_string(),
        Value::IpNetwork(v) => v.as_deref()?.to_string(),
        Value::MacAddress(v) => v.as_deref()?.to_string(),
        Value::Interval(v) => v.as_deref()?.to_string(),
        Value::Vector(v) => {
            let elements: Vec<String> = v
                .as_deref()?
                .as_slice()
                .iter()
                .map(f32::to_string)
                .collect();
            format!("[{}]", elements.join(","))
        }
        Value::Array(_, v) => array_text(v.as_deref()?),
        Value::Range(_, v) => range_text(v.as_deref()?),
        Value::Multirange(_, v) => multirange_text(v.as_deref()?),
    })
}

/// An array's text form, `{"a","b",NULL}`: every element quoted except a
/// `NULL` and a nested array, which is a dimension rather than an element.
fn array_text(elements: &[Value]) -> String {
    let elements: Vec<String> = elements
        .iter()
        .map(|element| match (element, value_text(element)) {
            (_, None) => "NULL".to_owned(),
            (Value::Array(..), Some(text)) => text,
            (_, Some(text)) => format!("\"{}\"", escape(&text, '\\')),
        })
        .collect();
    format!("{{{}}}", elements.join(","))
}

/// Each `"` and `\` preceded by `by`: `\` for an array element, and for a
/// range bound `"`, doubling the quote as the range writer does.
fn escape(text: &str, by: char) -> String {
    text.chars()
        .fold(String::with_capacity(text.len()), |mut out, c| {
            if c == '"' || c == '\\' {
                out.push(if c == '\\' { '\\' } else { by });
            }
            out.push(c);
            out
        })
}

/// A range's text form, as PostgreSQL's range output writes it: `empty`, or
/// a bracket, each bound, and a bracket. An unbounded side, and a `NULL`
/// bound — which is no bound, as everywhere else — is written as nothing.
// [spec:pgorm:def:sql.value.created-range+6]
pub(crate) fn range_text(range: &Range<Value>) -> String {
    use std::ops::Bound::{Excluded, Included, Unbounded};

    let Range::Bounds { lower, upper } = range else {
        return "empty".to_owned();
    };
    let (lower, open) = match lower {
        Included(value) => (value_text(value), '['),
        Excluded(value) => (value_text(value), '('),
        Unbounded => (None, '('),
    };
    let (upper, close) = match upper {
        Included(value) => (value_text(value), ']'),
        Excluded(value) => (value_text(value), ')'),
        Unbounded => (None, ')'),
    };
    let open = if lower.is_none() { '(' } else { open };
    let close = if upper.is_none() { ')' } else { close };
    format!(
        "{open}{},{}{close}",
        lower.as_deref().map(range_bound).unwrap_or_default(),
        upper.as_deref().map(range_bound).unwrap_or_default(),
    )
}

/// One bound inside a range literal: as written when the range parser would
/// read it back unchanged, otherwise in double quotes with each `"` and `\`
/// doubled — the empty string, and any text holding a quote, a backslash, a
/// bracket, a comma or whitespace, which the parser would otherwise read as
/// syntax or keep around the value.
fn range_bound(text: &str) -> String {
    let plain = !text.is_empty()
        && !text
            .chars()
            .any(|c| matches!(c, '"' | '\\' | '(' | ')' | '[' | ']' | ',') || is_space(c));
    if plain {
        text.to_owned()
    } else {
        format!("\"{}\"", escape(text, '"'))
    }
}

/// A multirange's text form, `{[1,3),[5,8)}`: its ranges in the order
/// written.
// [spec:pgorm:def:sql.value.created-range+6]
pub(crate) fn multirange_text(multirange: &Multirange<Value>) -> String {
    let ranges: Vec<String> = multirange.iter().map(range_text).collect();
    format!("{{{}}}", ranges.join(","))
}

/// A range's text form, each bound in its subtype's text form: what
/// PostgreSQL's range input reads back as this range, and what its output
/// writes for a continuous range whose bounds print as these do.
///
/// ```
/// use pgorm_query::Range;
///
/// assert_eq!(Range::from(1.5..2.5).to_string(), "[1.5,2.5)");
/// assert_eq!(Range::<f64>::from(..).to_string(), "(,)");
/// assert_eq!(Range::<f64>::Empty.to_string(), "empty");
/// assert_eq!(
///     Range::from("a,b".to_owned()..="say \"hi\"".to_owned()).to_string(),
///     r#"["a,b","say ""hi"""]"#,
/// );
/// ```
// [spec:pgorm:def:sql.value.created-range+6]
impl<T> fmt::Display for Range<T>
where
    T: RangeSubtype,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&range_text(&self.clone().map(Into::into)))
    }
}

/// A multirange's text form, `{[1,3),[5,8)}`.
// [spec:pgorm:def:sql.value.created-range+6]
impl<T> fmt::Display for Multirange<T>
where
    T: RangeSubtype,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&multirange_text(&self.clone().map(Into::into)))
    }
}

/// Reads a range's text form as PostgreSQL's range input does, and each bound
/// with the subtype's own text parsing: surrounding whitespace, the
/// case-insensitive `empty`, an empty side as no bound, double quotes around
/// a bound and a backslash or doubled quote inside one.
///
/// ```
/// use std::ops::Bound;
/// use pgorm_query::Range;
///
/// assert_eq!("[1.5,2.5)".parse().ok(), Some(Range::from(1.5..2.5)));
/// assert_eq!(" EMPTY ".parse().ok(), Some(Range::<f64>::Empty));
/// assert_eq!(
///     r#"("a""b",)"#.parse().ok(),
///     Some(Range::new(Bound::Excluded("a\"b".to_owned()), Bound::Unbounded)),
/// );
/// assert!("[1.5,2.5".parse::<Range<f64>>().is_err());
/// ```
// [spec:pgorm:def:sql.value.created-range+6]
impl<T> FromStr for Range<T>
where
    T: RangeSubtype,
{
    type Err = ValueTypeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut parser = Parser(text.trim_start_matches(is_space));
        let range = parser.range().ok_or(ValueTypeError)?;
        if parser.0.trim_start_matches(is_space).is_empty() {
            Ok(range)
        } else {
            Err(ValueTypeError)
        }
    }
}

/// Reads a multirange's text form, `{[1,3),empty,[5,8)}`, as PostgreSQL's
/// multirange input does.
///
/// ```
/// use pgorm_query::{Multirange, Range};
///
/// assert_eq!(
///     "{[1,3), empty ,[5,8)}".parse().ok(),
///     Some(Multirange::from(vec![Range::from(1..3), Range::Empty, Range::from(5..8)])),
/// );
/// assert_eq!("{}".parse().ok(), Some(Multirange::<i32>::default()));
/// ```
// [spec:pgorm:def:sql.value.created-range+6]
impl<T> FromStr for Multirange<T>
where
    T: RangeSubtype,
{
    type Err = ValueTypeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Parser(text.trim_matches(is_space))
            .multirange()
            .ok_or(ValueTypeError)
    }
}

/// The text still to read.
struct Parser<'a>(&'a str);

impl Parser<'_> {
    fn eat(&mut self, c: char) -> bool {
        match self.0.strip_prefix(c) {
            Some(rest) => {
                self.0 = rest;
                true
            }
            None => false,
        }
    }

    fn skip_space(&mut self) {
        self.0 = self.0.trim_start_matches(is_space);
    }

    /// `{` ranges separated by commas `}`, nothing after.
    fn multirange<T: RangeSubtype>(mut self) -> Option<Multirange<T>> {
        if !self.eat('{') {
            return None;
        }
        self.skip_space();
        let mut ranges = Vec::new();
        if !self.eat('}') {
            loop {
                self.skip_space();
                ranges.push(self.range()?);
                self.skip_space();
                if self.eat('}') {
                    break;
                }
                if !self.eat(',') {
                    return None;
                }
            }
        }
        self.0.is_empty().then(|| ranges.into())
    }

    /// `empty`, or a bracket, two bounds and a bracket.
    fn range<T: RangeSubtype>(&mut self) -> Option<Range<T>> {
        let text = self.0;
        if text
            .get(..5)
            .is_some_and(|word| word.eq_ignore_ascii_case("empty"))
        {
            self.0 = &text[5..];
            return Some(Range::Empty);
        }
        let lower_inclusive = if self.eat('[') {
            true
        } else if self.eat('(') {
            false
        } else {
            return None;
        };
        let lower = self.bound()?;
        if !self.eat(',') {
            return None;
        }
        let upper = self.bound()?;
        let upper_inclusive = if self.eat(']') {
            true
        } else if self.eat(')') {
            false
        } else {
            return None;
        };
        Some(Range::new(
            bound(lower, lower_inclusive)?,
            bound(upper, upper_inclusive)?,
        ))
    }

    /// One bound's text, `None` inside when the side is empty (no bound), or
    /// `None` outside when the text ends before the bound does.
    fn bound(&mut self) -> Option<Option<String>> {
        if self.0.starts_with([',', ')', ']']) {
            return Some(None);
        }
        let rest = self.0;
        let mut text = String::new();
        let mut quoted = false;
        let mut chars = rest.char_indices().peekable();
        while let Some((at, c)) = chars.next() {
            match c {
                ',' | ')' | ']' if !quoted => {
                    self.0 = &rest[at..];
                    return Some(Some(text));
                }
                '\\' => text.push(chars.next()?.1),
                '"' if quoted && chars.peek().is_some_and(|(_, next)| *next == '"') => {
                    text.push('"');
                    chars.next();
                }
                '"' => quoted = !quoted,
                c => text.push(c),
            }
        }
        None
    }
}

/// A side of a parsed range: no bound, or the subtype's value its text holds.
fn bound<T: RangeSubtype>(text: Option<String>, inclusive: bool) -> Option<std::ops::Bound<T>> {
    use std::ops::Bound::{Excluded, Included, Unbounded};

    Some(match text {
        None => Unbounded,
        Some(text) if inclusive => Included(T::parse_bound(&text)?),
        Some(text) => Excluded(T::parse_bound(&text)?),
    })
}

#[cfg(test)]
mod tests {
    use std::ops::Bound::{Excluded, Included, Unbounded};

    use super::*;
    use pretty_assertions::assert_eq;

    fn round_trip<T>(range: Range<T>) -> String
    where
        T: RangeSubtype + PartialEq + fmt::Debug,
    {
        let text = range.to_string();
        assert_eq!(text.parse::<Range<T>>().ok(), Some(range), "{text}");
        text
    }

    // [spec:pgorm:def:sql.value.created-range+6/test]
    #[test]
    fn each_subtype_reads_back_its_text() {
        assert_eq!(round_trip(Range::from(1i16..5)), "[1,5)");
        assert_eq!(round_trip(Range::from(-7i32..=5)), "[-7,5]");
        assert_eq!(
            round_trip(Range::from(i64::MIN..i64::MAX)),
            format!("[{},{})", i64::MIN, i64::MAX)
        );
        assert_eq!(round_trip(Range::from(0.1f32..0.3)), "[0.1,0.3)");
        assert_eq!(
            round_trip(Range::new(Excluded(-0.0f64), Included(1e300))),
            format!("(-0,{}]", 1e300)
        );
        assert_eq!(
            round_trip(Range::new(
                Included(f64::NEG_INFINITY),
                Included(f64::INFINITY)
            )),
            "[-inf,inf]"
        );
        assert_eq!(round_trip(Range::from(Decimal::new(150, 2)..)), "[1.50,)");
        assert_eq!(
            round_trip(Range::from(..Date::constant(2024, 2, 29))),
            "(,2024-02-29)"
        );
        assert_eq!(
            round_trip(Range::from(
                Time::constant(9, 0, 0, 0)..Time::constant(17, 30, 0, 500_000_000)
            )),
            "[09:00:00,17:30:00.5)"
        );
        assert_eq!(
            round_trip(Range::from(
                DateTime::constant(2024, 1, 1, 10, 0, 0, 0)
                    ..DateTime::constant(2024, 1, 2, 0, 0, 0, 0)
            )),
            "[\"2024-01-01 10:00:00\",\"2024-01-02 00:00:00\")"
        );
        let instant: Timestamp = "2024-01-01T10:00:00.123456Z".parse().unwrap();
        assert_eq!(
            round_trip(Range::from(instant..=instant)),
            "[\"2024-01-01 10:00:00.123456+00:00\",\"2024-01-01 10:00:00.123456+00:00\"]"
        );
        let id = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        assert_eq!(
            round_trip(Range::from(id..)),
            "[01234567-89ab-cdef-0123-456789abcdef,)"
        );
    }

    // [spec:pgorm:def:sql.value.created-range+6/test]
    #[test]
    fn bounds_are_quoted_where_the_parser_needs() {
        let text = |lower: &str, upper: &str| Range::from(lower.to_owned()..upper.to_owned());
        assert_eq!(round_trip(text("a", "b")), "[a,b)");
        assert_eq!(round_trip(text("", "b")), "[\"\",b)");
        assert_eq!(round_trip(text("a,b", "c)d")), "[\"a,b\",\"c)d\")");
        assert_eq!(
            round_trip(text("say \"hi\"", "a\\b")),
            r#"["say ""hi""","a\\b")"#
        );
        assert_eq!(round_trip(text(" a ", "\tb")), "[\" a \",\"\tb\")");
        assert_eq!(round_trip(text("(", "]")), "[\"(\",\"]\")");
        assert_eq!(round_trip(text("NULL", "empty")), "[NULL,empty)");
        assert_eq!(round_trip(text("雪", "ü")), "[雪,ü)");
        assert_eq!(round_trip(Range::<String>::Empty), "empty");
        assert_eq!(round_trip(Range::<String>::from(..)), "(,)");
    }

    // [spec:pgorm:def:sql.value.created-range+6/test]
    #[test]
    fn the_parser_reads_what_the_server_accepts() {
        let parse = |text: &str| text.parse::<Range<String>>().ok();
        assert_eq!(
            parse("[\\a,b)"),
            Some(Range::from("a".to_owned().."b".to_owned()))
        );
        assert_eq!(
            parse(r#"["c\"d",)"#),
            Some(Range::new(Included("c\"d".to_owned()), Unbounded))
        );
        assert_eq!(parse("  empty\n"), Some(Range::Empty));
        assert_eq!(parse("(,)"), Some(Range::from(..)));
        assert_eq!(
            parse("[ a , b )"),
            Some(Range::from(" a ".to_owned().." b ".to_owned()))
        );
        for malformed in [
            "[a,b", "a,b)", "[a,b,c)", "[a,b) x", "empty x", "[\"a,b)", "{[a,b)}", "[a\\",
        ] {
            assert_eq!(parse(malformed), None, "{malformed}");
        }
        assert!("[x,2)".parse::<Range<f64>>().is_err());
        assert_eq!(
            "[Infinity,NaN]"
                .parse::<Range<f64>>()
                .ok()
                .map(|range| range.to_string()),
            Some("[inf,NaN]".to_owned())
        );
        assert_eq!(
            "[1e+300,)".parse::<Range<f64>>().ok(),
            Some(Range::from(1e300..))
        );
    }

    // [spec:pgorm:def:sql.value.created-range+6/test]
    #[test]
    fn a_null_bound_writes_no_bound() {
        let range = Range::new(Included(Value::Int(None)), Excluded(Value::Int(Some(5))));
        assert_eq!(range_text(&range), "(,5)");
        assert_eq!(
            value_text(&Value::Range(crate::RangeType::Int4, None)),
            None
        );
    }

    // [spec:pgorm:def:sql.value.created-range+6/test]
    #[test]
    fn a_bound_of_any_value_has_a_text() {
        let text = |value: Value| value_text(&value).unwrap();
        assert_eq!(text(true.into()), "t");
        assert_eq!(text(vec![0u8, 255].into()), "\\x00ff");
        assert_eq!(
            text(Value::array(vec![Some("a\"b".to_owned()), None])),
            r#"{"a\"b",NULL}"#
        );
        assert_eq!(
            text(Value::from(Multirange::from(vec![
                Range::from(1..3),
                Range::Empty
            ]))),
            "{[1,3),empty}"
        );
        let nested = Range::new(
            Included(Value::from(Range::from(1..3))),
            Excluded(Value::from(Range::from(4..5))),
        );
        assert_eq!(range_text(&nested), "[\"[1,3)\",\"[4,5)\")");
    }

    // [spec:pgorm:def:sql.value.created-range+6/test]
    #[test]
    fn a_multirange_reads_back_its_text() {
        let multirange = Multirange::from(vec![
            Range::from(5.0f64..8.0),
            Range::Empty,
            Range::new(Unbounded, Included(1.5)),
        ]);
        let text = multirange.to_string();
        assert_eq!(text, "{[5,8),empty,(,1.5]}");
        assert_eq!(text.parse().ok(), Some(multirange));
        assert_eq!(Multirange::<f64>::default().to_string(), "{}");
        for malformed in ["{", "{[1,2)", "{[1,2),}", "[1,2)", "{[1,2)} x"] {
            assert!(malformed.parse::<Multirange<f64>>().is_err(), "{malformed}");
        }
    }
}
