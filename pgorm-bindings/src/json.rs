//! A `json` or `jsonb` column read so that every number in it reaches the
//! language exactly or not at all.
//!
//! serde_json reads an integer that fits 64 bits as one and anything else as
//! an `f64`. The document's own spelling of each number is compared with what
//! serde_json reads it as, so a number that would change in the reading is
//! refused rather than delivered rounded. One pass over the server's text
//! decides it without building the value, so a binding whose language parses
//! JSON text itself is handed the text, written as serde_json writes the
//! value: each object's keys in order and once each.

use std::{borrow::Cow, error::Error, ops::Range};

use pgorm_driver::types::{FromSql, Type};
use serde_json::{Number, Value};

type CodecError = Box<dyn Error + Send + Sync>;

/// How deep a document may nest.
pub const MAX_DEPTH: usize = 64;

/// The largest integer a 64-bit float holds exactly, 2^53 - 1.
const MAX_FLOAT_INTEGER: u64 = (1 << 53) - 1;

/// A `json` or `jsonb` value whose every number serde_json read exactly.
// [spec:pgorm:req:bindings.values]
// [spec:pgorm:req:napi.values+2]
#[derive(Debug)]
pub struct ExactJson(pub Value);

impl<'a> FromSql<'a> for ExactJson {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, CodecError> {
        let text = ExactJsonText::from_sql(ty, raw)?;
        Ok(Self(serde_json::from_str(text.text)?))
    }

    fn accepts(ty: &Type) -> bool {
        <Value as FromSql>::accepts(ty)
    }
}

/// The text of a `json` or `jsonb` value [`ExactJson`] reads, held to the
/// same rule without the value being built.
// [spec:pgorm:req:bindings.values]
// [spec:pgorm:req:napi.values+2]
#[derive(Debug, Clone, Copy)]
pub struct ExactJsonText<'a> {
    text: &'a str,
    /// Whether each object's keys already stand in order, once each.
    ordered: bool,
    wide_integers: bool,
}

impl ExactJsonText<'_> {
    /// Whether the document holds an integer past 2^53 - 1 in magnitude,
    /// which a 64-bit float does not hold exactly.
    pub fn wide_integers(&self) -> bool {
        self.wide_integers
    }

    /// Append the document to `out` as serde_json writes the value
    /// [`ExactJson`] reads: each object's keys in order and once each, a
    /// repeated key's last value kept, and every scalar as the server spelled
    /// it.
    pub fn push_to(&self, out: &mut String) {
        if self.ordered {
            out.push_str(self.text);
        } else {
            ordered(self.text, whitespace(self.text.as_bytes(), 0), out);
        }
    }
}

impl<'a> FromSql<'a> for ExactJsonText<'a> {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, CodecError> {
        let text = if *ty == Type::JSONB {
            match raw.split_first() {
                Some((1, text)) => text,
                _ => return Err(malformed(ty, raw)),
            }
        } else {
            raw
        };
        let Ok(text) = std::str::from_utf8(text) else {
            return Err(malformed(ty, raw));
        };
        match scan(text) {
            Ok((ordered, wide_integers)) => Ok(Self {
                text,
                ordered,
                wide_integers,
            }),
            Err(Refused::Malformed) => Err(malformed(ty, raw)),
            // A document serde_json cannot read at all is refused as
            // serde_json refuses it, whatever its numbers.
            Err(Refused::Inexact(error)) => match Value::from_sql(ty, raw) {
                Ok(_) => Err(error),
                Err(malformed) => Err(malformed),
            },
        }
    }

    fn accepts(ty: &Type) -> bool {
        <Value as FromSql>::accepts(ty)
    }
}

/// What serde_json, or the driver's framing of JSONB, says of a document the
/// scan refused.
fn malformed(ty: &Type, raw: &[u8]) -> CodecError {
    match Value::from_sql(ty, raw) {
        Err(error) => error,
        Ok(_) => "JSON that serde_json reads and its scan refused".into(),
    }
}

enum Refused {
    /// Text serde_json does not read: a syntax error, a lone surrogate, a
    /// number past `f64`'s range.
    Malformed,
    /// A number serde_json would read as another, or nesting past
    /// [`MAX_DEPTH`].
    Inexact(CodecError),
}

#[derive(Clone, Copy)]
enum Container {
    Array,
    /// An object, and where its last key's text lies.
    Object(Option<(usize, usize)>),
}

/// Walk `text` as one JSON document, holding every number to its reading,
/// and say whether each object's keys stand in order, once each, and whether
/// an integer is past 2^53 - 1 in magnitude.
fn scan(text: &str) -> Result<(bool, bool), Refused> {
    let bytes = text.as_bytes();
    let mut at = whitespace(bytes, 0);
    let mut open: Vec<Container> = Vec::new();
    let mut ordered = true;
    let mut wide = false;
    loop {
        if open.len() > MAX_DEPTH {
            return Err(Refused::Inexact("JSON nests deeper than 64 levels".into()));
        }
        match bytes.get(at) {
            Some(b'{') => {
                at = whitespace(bytes, at + 1);
                if bytes.get(at) == Some(&b'}') {
                    at += 1;
                } else {
                    open.push(Container::Object(None));
                    at = key(bytes, at, &mut open, &mut ordered)?;
                    continue;
                }
            }
            Some(b'[') => {
                at = whitespace(bytes, at + 1);
                if bytes.get(at) == Some(&b']') {
                    at += 1;
                } else {
                    open.push(Container::Array);
                    continue;
                }
            }
            Some(b'"') => at = string(bytes, at)?,
            Some(b'-' | b'0'..=b'9') => {
                let end = number(bytes, at)?;
                wide |= exact(text.get(at..end).unwrap_or_default())?;
                at = end;
            }
            Some(b't') => at = literal(bytes, at, b"true")?,
            Some(b'f') => at = literal(bytes, at, b"false")?,
            Some(b'n') => at = literal(bytes, at, b"null")?,
            _ => return Err(Refused::Malformed),
        }
        loop {
            at = whitespace(bytes, at);
            match (open.last(), bytes.get(at)) {
                (None, None) => return Ok((ordered, wide)),
                (Some(Container::Array), Some(b',')) => {
                    at = whitespace(bytes, at + 1);
                    break;
                }
                (Some(Container::Object(_)), Some(b',')) => {
                    at = key(bytes, whitespace(bytes, at + 1), &mut open, &mut ordered)?;
                    break;
                }
                (Some(Container::Array), Some(b']')) | (Some(Container::Object(_)), Some(b'}')) => {
                    open.pop();
                    at += 1;
                }
                _ => return Err(Refused::Malformed),
            }
        }
    }
}

fn whitespace(bytes: &[u8], mut at: usize) -> usize {
    while let Some(b' ' | b'\t' | b'\n' | b'\r') = bytes.get(at) {
        at += 1;
    }
    at
}

/// The key of the object `open` ends with, at `at`, and the `:` after it:
/// where its value starts. A key holding an escape, or not after the one
/// before it, leaves the document out of order.
fn key(
    bytes: &[u8],
    at: usize,
    open: &mut [Container],
    ordered: &mut bool,
) -> Result<usize, Refused> {
    if bytes.get(at) != Some(&b'"') {
        return Err(Refused::Malformed);
    }
    let end = string(bytes, at)?;
    let span = (at + 1, end - 1);
    let text = bytes.get(span.0..span.1).unwrap_or_default();
    if let Some(Container::Object(last)) = open.last_mut() {
        let after =
            last.is_none_or(|(start, end)| bytes.get(start..end).unwrap_or_default() < text);
        *ordered &= after && !text.contains(&b'\\');
        *last = Some(span);
    }
    let at = whitespace(bytes, end);
    if bytes.get(at) != Some(&b':') {
        return Err(Refused::Malformed);
    }
    Ok(whitespace(bytes, at + 1))
}

fn literal(bytes: &[u8], at: usize, word: &[u8]) -> Result<usize, Refused> {
    match bytes.get(at..at + word.len()) {
        Some(found) if found == word => Ok(at + word.len()),
        _ => Err(Refused::Malformed),
    }
}

/// The end of the string opening at `at`, its characters and escapes held to
/// what serde_json reads into a string: no control character, and a
/// surrogate escape only in a pair.
fn string(bytes: &[u8], at: usize) -> Result<usize, Refused> {
    let mut at = at + 1;
    loop {
        match bytes.get(at) {
            Some(b'"') => return Ok(at + 1),
            Some(b'\\') => {
                at = match bytes.get(at + 1) {
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => at + 2,
                    Some(b'u') => match hex(bytes, at + 2)? {
                        0xD800..=0xDBFF
                            if bytes.get(at + 6..at + 8) == Some(b"\\u")
                                && (0xDC00..=0xDFFF).contains(&hex(bytes, at + 8)?) =>
                        {
                            at + 12
                        }
                        0xD800..=0xDFFF => return Err(Refused::Malformed),
                        _ => at + 6,
                    },
                    _ => return Err(Refused::Malformed),
                };
            }
            Some(0x00..=0x1F) | None => return Err(Refused::Malformed),
            Some(_) => at += 1,
        }
    }
}

/// The UTF-16 code unit the four hex digits at `at` spell.
fn hex(bytes: &[u8], at: usize) -> Result<u16, Refused> {
    let digits = bytes.get(at..at + 4).ok_or(Refused::Malformed)?;
    let mut unit = 0u16;
    for digit in digits {
        let value = match digit {
            b'0'..=b'9' => digit - b'0',
            b'a'..=b'f' => digit - b'a' + 10,
            b'A'..=b'F' => digit - b'A' + 10,
            _ => return Err(Refused::Malformed),
        };
        unit = unit << 4 | u16::from(value);
    }
    Ok(unit)
}

/// The end of the number starting at `at`, held to JSON's grammar.
fn number(bytes: &[u8], at: usize) -> Result<usize, Refused> {
    let digits = |start: usize| {
        let mut at = start;
        while let Some(b'0'..=b'9') = bytes.get(at) {
            at += 1;
        }
        if at > start {
            Ok(at)
        } else {
            Err(Refused::Malformed)
        }
    };
    let mut at = if bytes.get(at) == Some(&b'-') {
        at + 1
    } else {
        at
    };
    at = match bytes.get(at) {
        Some(b'0') => at + 1,
        Some(b'1'..=b'9') => digits(at)?,
        _ => return Err(Refused::Malformed),
    };
    if bytes.get(at) == Some(&b'.') {
        at = digits(at + 1)?;
    }
    if let Some(b'e' | b'E') = bytes.get(at) {
        at += 1;
        if let Some(b'+' | b'-') = bytes.get(at) {
            at += 1;
        }
        at = digits(at)?;
    }
    Ok(at)
}

/// Hold `original`, one number's spelling, to what serde_json reads it as,
/// and say whether it is an integer past 2^53 - 1 in magnitude.
fn exact(original: &str) -> Result<bool, Refused> {
    let integer = !original.contains(['.', 'e', 'E']);
    let negative = original.strip_prefix('-');
    let digits = negative.unwrap_or(original);
    // Up to 19 digits, or 18 after a minus, fit a 64-bit integer, which
    // serde_json reads exactly; except `-0`, which it reads as the float
    // negative zero.
    if integer
        && digits.len() <= if negative.is_some() { 18 } else { 19 }
        && (negative.is_none() || digits != "0")
    {
        return Ok(digits.len() > 15
            && digits
                .parse::<u64>()
                .is_ok_and(|value| value > MAX_FLOAT_INTEGER));
    }
    let number: Number = serde_json::from_str(original).map_err(|_| Refused::Malformed)?;
    if integer && !number.is_i64() && !number.is_u64() {
        return Err(Refused::Inexact(
            format!("JSON integer {original} exceeds 64 bits").into(),
        ));
    }
    let spelled = canonical(original).map_err(Refused::Inexact)?;
    let read = canonical(&number.to_string()).map_err(Refused::Inexact)?;
    if spelled != read {
        return Err(Refused::Inexact(
            format!("JSON number {original} has no exact binary form").into(),
        ));
    }
    Ok(integer)
}

/// A decimal JSON spelling as sign, significant digits and exponent, compared
/// without converting either side through a float.
fn canonical(text: &str) -> Result<(bool, String, i64), CodecError> {
    let negative = text.starts_with('-');
    let text = text.strip_prefix('-').unwrap_or(text);
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i64>()?),
        None => (text, 0),
    };
    let fraction = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Ok((negative, "0".to_owned(), 0));
    }
    let trimmed = significant.trim_end_matches('0');
    let zeros = significant.len() - trimmed.len();
    let exponent = exponent
        .checked_sub(i64::try_from(fraction)?)
        .and_then(|exponent| exponent.checked_add(i64::try_from(zeros).ok()?))
        .ok_or("JSON exponent exceeds the supported range")?;
    Ok((negative, trimmed.to_owned(), exponent))
}

/// Write the value of the scanned document `text` starting at `at` to `out`,
/// each object's keys in order and once each, and say where it ends.
fn ordered(text: &str, at: usize, out: &mut String) -> usize {
    let bytes = text.as_bytes();
    let slice = |range: Range<usize>| text.get(range).unwrap_or_default();
    match bytes.get(at) {
        Some(b'{') => {
            let mut members: Vec<(Cow<'_, str>, Range<usize>)> = Vec::new();
            let mut written = String::new();
            let mut at = whitespace(bytes, at + 1);
            while bytes.get(at) == Some(&b'"') {
                let end = string_end(bytes, at);
                let key = slice(at..end);
                let name = if key.contains('\\') {
                    serde_json::from_str::<String>(key).map_or(Cow::Borrowed(key), Cow::Owned)
                } else {
                    Cow::Borrowed(slice(at + 1..end - 1))
                };
                let start = written.len();
                written.push_str(key);
                written.push(':');
                at = whitespace(bytes, whitespace(bytes, end) + 1);
                at = whitespace(bytes, ordered(text, at, &mut written));
                members.push((name, start..written.len()));
                if bytes.get(at) == Some(&b',') {
                    at = whitespace(bytes, at + 1);
                }
            }
            // A stable sort leaves a repeated key's members in document
            // order, and the last of them is the one serde_json keeps.
            members.sort_by(|(left, _), (right, _)| left.cmp(right));
            out.push('{');
            let mut first = true;
            for (index, (name, range)) in members.iter().enumerate() {
                if members.get(index + 1).is_some_and(|(next, _)| next == name) {
                    continue;
                }
                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str(written.get(range.clone()).unwrap_or_default());
            }
            out.push('}');
            at + 1
        }
        Some(b'[') => {
            out.push('[');
            let mut at = whitespace(bytes, at + 1);
            let mut first = true;
            while !matches!(bytes.get(at), Some(b']') | None) {
                if !first {
                    out.push(',');
                }
                first = false;
                at = whitespace(bytes, ordered(text, at, out));
                if bytes.get(at) == Some(&b',') {
                    at = whitespace(bytes, at + 1);
                }
            }
            out.push(']');
            at + 1
        }
        Some(b'"') => {
            let end = string_end(bytes, at);
            out.push_str(slice(at..end));
            end
        }
        _ => {
            let mut end = at;
            while !matches!(
                bytes.get(end),
                Some(b',' | b']' | b'}' | b' ' | b'\t' | b'\n' | b'\r') | None
            ) {
                end += 1;
            }
            out.push_str(slice(at..end));
            end
        }
    }
}

/// The end of the string opening at `at`. The scan has held its escapes to
/// JSON's grammar, so each backslash is passed over with the character after
/// it, unread.
fn string_end(bytes: &[u8], at: usize) -> usize {
    let mut at = at + 1;
    loop {
        match bytes.get(at) {
            Some(b'"') | None => return at + 1,
            Some(b'\\') => at += 2,
            Some(_) => at += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(source: &str) -> Result<String, CodecError> {
        let mut out = String::new();
        ExactJsonText::from_sql(&Type::JSON, source.as_bytes())?.push_to(&mut out);
        Ok(out)
    }

    // [spec:pgorm:req:bindings.values/test]
    // [spec:pgorm:req:napi.values+2/test]
    #[test]
    fn json_numbers_are_read_exactly_or_refused() -> Result<(), CodecError> {
        for text in [
            "0.1",
            "-0.0",
            "1e20",
            "[1.2300,18446744073709551615]",
            "{\"a\": 1.2345e-8}",
            "-9223372036854775808",
        ] {
            assert_eq!(
                ExactJson::from_sql(&Type::JSON, text.as_bytes())?.0,
                Value::from_sql(&Type::JSON, text.as_bytes())?
            );
        }
        for text in [
            "18446744073709551616",
            "-9223372036854775809",
            "100000000000000000000",
            "-0",
            "[0.10000000000000000001]",
            "{\"a\":1e-400}",
        ] {
            assert!(
                ExactJson::from_sql(&Type::JSON, text.as_bytes()).is_err(),
                "{text}"
            );
        }
        Ok(())
    }

    // [spec:pgorm:req:bindings.values/test]
    #[test]
    fn json_is_refused_as_serde_json_refuses_it() {
        for text in [
            "",
            "[1,]",
            "{\"a\" 1}",
            "01",
            "\"\\ud800\"",
            "\"\\udc00x\"",
            "\"a\tb\"",
            "1e400",
            "[1] 2",
        ] {
            let refusal = ExactJsonText::from_sql(&Type::JSON, text.as_bytes()).map(|_| ());
            let serde = Value::from_sql(&Type::JSON, text.as_bytes()).map(|_| ());
            assert_eq!(
                refusal.map_err(|error| error.to_string()),
                serde.map_err(|error| error.to_string()),
                "{text:?}"
            );
        }
        assert!(ExactJsonText::from_sql(&Type::JSONB, b"\x02{}").is_err());
        assert!(ExactJsonText::from_sql(&Type::JSONB, b"").is_err());
        assert!(ExactJsonText::from_sql(&Type::JSONB, b"\x01{}").is_ok());
        assert!(ExactJsonText::from_sql(&Type::JSON, "\"\\ud83e\\udd80 🦀\"".as_bytes()).is_ok());
    }

    // [spec:pgorm:req:bindings.values/test]
    #[test]
    fn json_nests_at_most_64_levels() {
        let refusal = |depth: usize| {
            let nested = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
            ExactJsonText::from_sql(&Type::JSON, nested.as_bytes())
                .err()
                .map(|error| error.to_string())
        };
        assert_eq!(refusal(65), None);
        assert!(refusal(66).is_some_and(|error| error.contains("deeper than 64")));
        assert!(refusal(200).is_some_and(|error| error.contains("recursion limit")));
    }

    // [spec:pgorm:req:bindings.values/test]
    #[test]
    fn json_text_is_written_in_serde_jsons_order() -> Result<(), CodecError> {
        assert_eq!(
            text(" {\"a\": 1, \"b\": [2, {\"c\": null}]} ")?,
            " {\"a\": 1, \"b\": [2, {\"c\": null}]} "
        );
        assert_eq!(
            text("{\"i\": 1, \"tags\": [\"a\", \"b\"], \"nested\": {\"x\": 2, \"w\": 1.50}}")?,
            "{\"i\":1,\"nested\":{\"w\":1.50,\"x\":2},\"tags\":[\"a\",\"b\"]}"
        );
        assert_eq!(text("{\"b\":1,\"a\":2,\"b\":3}")?, "{\"a\":2,\"b\":3}");
        assert_eq!(text("{\"\\u0062\":1,\"a\":2}")?, "{\"a\":2,\"\\u0062\":1}");
        assert_eq!(
            text("[{\"é\":1,\"z\":2},{}, [] ]")?,
            "[{\"z\":2,\"é\":1},{},[]]"
        );
        for source in [
            "{\"b\":{\"d\":1,\"c\":2},\"a\":[{\"y\":0,\"x\":-1.5}],\"aa\":null,\"B\":\"v\\\"\"}",
            "{\"x\":{\"x\":{\"b\":[],\"a\":{}}},\"x\":true,\"\":false}",
        ] {
            assert_eq!(
                text(source)?,
                serde_json::from_str::<Value>(source)?.to_string(),
                "{source}"
            );
        }
        Ok(())
    }

    // [spec:pgorm:req:napi.values+2/test]
    #[test]
    fn json_text_flags_integers_past_2_53() -> Result<(), CodecError> {
        for (source, wide) in [
            ("[9007199254740991, -9007199254740991, 1e300, 12.5]", false),
            ("9007199254740992", true),
            ("{\"a\": [-9007199254740992]}", true),
            ("18446744073709551615", true),
        ] {
            let text = ExactJsonText::from_sql(&Type::JSON, source.as_bytes())?;
            assert_eq!(text.wide_integers(), wide, "{source}");
        }
        Ok(())
    }
}
