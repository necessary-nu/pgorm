//! A `numeric` read only when a [`Decimal`] holds it exactly.

use bytes::BytesMut;
use rust_decimal::Decimal;
use tokio_postgres::row::RowIndex;
use tokio_postgres::types::{FromSql, ToSql, Type};

use crate::{QueryResult, TryGetError, TryGetable};

type WireError = Box<dyn std::error::Error + Sync + Send>;

/// A `numeric` that [`Decimal`] holds exactly, its scale included, or a
/// decode error. rust_decimal's own decoder rounds a value with more than 28
/// fractional digits and keeps no coefficient past 96 bits, so the value it
/// reads is written back and compared with what the server sent. Every
/// `Decimal` pgorm decodes reads through this: a `Decimal` or `Vec<Decimal>`
/// field or tuple element, a `numrange`'s bounds, a range type a schema
/// created ([`CreatedRangeCodec`](crate::CreatedRangeCodec)), a binary COPY's
/// rows, and the language bindings' `numeric` values.
// [spec:pgorm:req:bindings.values]
// [spec:pgorm:req:exec.decode.decimal-exact]
#[derive(Debug)]
pub struct ExactDecimal(pub Decimal);

/// A `numeric` read only where a `Decimal` holds it exactly: one past 28
/// fractional digits or a 96-bit coefficient fails to decode rather than
/// coming back rounded, as COPY rows and the bindings read it.
// [spec:pgorm:req:exec.decode.decimal-exact]
impl TryGetable for Decimal {
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, TryGetError> {
        let result: ExactDecimal = res.row.try_get(idx).map_err(TryGetError::postgres)?;
        Ok(result.0)
    }

    // [spec:pgorm:sem:exec.verify.accepts+1]
    fn accepts(ty: &Type) -> bool {
        <Decimal as FromSql>::accepts(ty)
    }
}

impl From<ExactDecimal> for Decimal {
    fn from(exact: ExactDecimal) -> Self {
        exact.0
    }
}

impl<'a> FromSql<'a> for ExactDecimal {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, WireError> {
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

/// A `numeric`'s binary form: its weight, sign, display scale and base-10000
/// digits.
struct Numeric {
    weight: i16,
    sign: u16,
    scale: u16,
    digits: Vec<u16>,
}

impl Numeric {
    fn read(raw: &[u8]) -> Result<Self, WireError> {
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

    /// The value with its scale set aside: the weight of its first nonzero
    /// digit group, its sign and its digit groups from that one to the last
    /// nonzero, or zero.
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
    use super::*;

    fn numeric(text: &str) -> Result<Vec<u8>, WireError> {
        let mut out = BytesMut::new();
        Decimal::from_str_exact(text)?.to_sql(&Type::NUMERIC, &mut out)?;
        Ok(out.to_vec())
    }

    // [spec:pgorm:req:bindings.values/test]
    #[test]
    fn numerics_decode_exactly_or_not_at_all() -> Result<(), WireError> {
        assert_eq!(
            ExactDecimal::from_sql(&Type::NUMERIC, &numeric("19.9900")?)?
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
        assert!(ExactDecimal::from_sql(&Type::NUMERIC, &[0; 7]).is_err());
        Ok(())
    }

    // [spec:pgorm:req:bindings.values/test]
    #[test]
    fn an_exact_numeric_keeps_rust_decimal_bytes() -> Result<(), WireError> {
        for text in [
            "0.0000",
            "1.2300",
            "-42.001",
            "79228162514264337593543950335",
        ] {
            let raw = numeric(text)?;
            // The Rust encoder canonicalizes zero scale on the wire.
            let rust = Decimal::from_sql(&Type::NUMERIC, &raw)?;
            let exact = ExactDecimal::from_sql(&Type::NUMERIC, &raw)?.0;
            assert_eq!(exact.serialize(), rust.serialize());
        }
        Ok(())
    }

    // [spec:pgorm:req:bindings.values/test]
    #[test]
    fn a_rounded_numeric_is_refused() {
        // 1 + 1e-32 in base-10000 groups, which rust_decimal's decoder rounds.
        let words: [u16; 13] = [9, 0, 0, 32, 1, 0, 0, 0, 0, 0, 0, 0, 1];
        let raw: Vec<_> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
        assert!(Decimal::from_sql(&Type::NUMERIC, &raw).is_ok());
        assert!(ExactDecimal::from_sql(&Type::NUMERIC, &raw).is_err());
    }
}
