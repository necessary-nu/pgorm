//! A `json` or `jsonb` column read so that every number in it reaches the
//! language exactly or not at all.
//!
//! serde_json reads an integer that fits 64 bits as one and anything else as
//! an `f64`. The document's own spelling of each number is compared with what
//! was read, so a number that changed in the reading is refused rather than
//! delivered rounded.

use std::{collections::BTreeMap, error::Error};

use serde_json::{Value, value::RawValue};
use tokio_postgres::types::{FromSql, Type};

type CodecError = Box<dyn Error + Send + Sync>;

/// How deep a document may nest.
pub const MAX_DEPTH: usize = 64;

/// A `json` or `jsonb` value whose every number serde_json read exactly.
// [spec:pgorm:req:bindings.values]
// [spec:pgorm:req:napi.values+1]
#[derive(Debug)]
pub struct ExactJson(pub Value);

impl<'a> FromSql<'a> for ExactJson {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, CodecError> {
        let value = Value::from_sql(ty, raw)?;
        let text = if *ty == Type::JSONB {
            raw.get(1..).ok_or("missing JSONB version")?
        } else {
            raw
        };
        let original: &RawValue = serde_json::from_slice(text)?;
        validate(original, &value, 0)?;
        Ok(Self(value))
    }

    fn accepts(ty: &Type) -> bool {
        <Value as FromSql>::accepts(ty)
    }
}

fn validate(raw: &RawValue, decoded: &Value, depth: usize) -> Result<(), CodecError> {
    if depth > MAX_DEPTH {
        return Err("JSON nests deeper than 64 levels".into());
    }
    match decoded {
        Value::Number(number) => {
            let original = raw.get();
            if !original.contains(['.', 'e', 'E']) && !number.is_i64() && !number.is_u64() {
                return Err(format!("JSON integer {original} exceeds 64 bits").into());
            }
            if canonical(original)? != canonical(&number.to_string())? {
                return Err(format!("JSON number {original} has no exact binary form").into());
            }
        }
        Value::Array(values) => {
            let originals: Vec<&RawValue> = serde_json::from_str(raw.get())?;
            if originals.len() != values.len() {
                return Err("JSON array shape changed".into());
            }
            for (original, value) in originals.into_iter().zip(values) {
                validate(original, value, depth + 1)?;
            }
        }
        Value::Object(values) => {
            let originals: BTreeMap<String, &RawValue> = serde_json::from_str(raw.get())?;
            for (name, value) in values {
                validate(
                    originals.get(name).ok_or("JSON object shape changed")?,
                    value,
                    depth + 1,
                )?;
            }
        }
        _ => (),
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:pgorm:req:bindings.values/test]
    // [spec:pgorm:req:napi.values+1/test]
    #[test]
    fn json_numbers_are_read_exactly_or_refused() -> Result<(), CodecError> {
        for text in [
            "0.1",
            "-0.0",
            "1e20",
            "[1.2300,18446744073709551615]",
            "{\"a\": 1.2345e-8}",
        ] {
            assert_eq!(
                ExactJson::from_sql(&Type::JSON, text.as_bytes())?.0,
                Value::from_sql(&Type::JSON, text.as_bytes())?
            );
        }
        for text in [
            "18446744073709551616",
            "100000000000000000000",
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
}
