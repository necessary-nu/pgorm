//! Checked scalar conversion at the native boundary. Prepared statements keep
//! these codec plans; returned rows contain only ordinary PostgreSQL binary data.

use crate::types::{BorrowToSql, Format, IsNull, Type};
use crate::{Error, Row, Statement};
use bytes::{BufMut, BytesMut};
use std::error::Error as StdError;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;
use turso_core::{Numeric, Value};

type Cause = Box<dyn StdError + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScalarCodec {
    Bool,
    I16,
    I32,
    I64,
    F32,
    F64,
    Utf8,
    Bytes,
}

#[derive(Debug)]
enum CodecError {
    UnsupportedType(Type),
    TextFormat,
    BinaryLength { expected: usize, actual: usize },
    Value(&'static str),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedType(ty) => write!(f, "native Turso has no scalar codec for {ty}"),
            Self::TextFormat => f.write_str("native Turso requires a binary ToSql encoding"),
            Self::BinaryLength { expected, actual } => {
                write!(f, "expected {expected} binary bytes, received {actual}")
            }
            Self::Value(message) => f.write_str(message),
        }
    }
}

impl StdError for CodecError {}

impl ScalarCodec {
    pub(crate) fn for_type(ty: &Type) -> Option<Self> {
        Some(match *ty {
            Type::BOOL => Self::Bool,
            Type::INT2 => Self::I16,
            Type::INT4 => Self::I32,
            Type::INT8 => Self::I64,
            Type::FLOAT4 => Self::F32,
            Type::FLOAT8 => Self::F64,
            Type::TEXT | Type::VARCHAR | Type::BPCHAR => Self::Utf8,
            Type::BYTEA => Self::Bytes,
            _ => return None,
        })
    }

    fn decode(self, bytes: &[u8]) -> Result<Value, Cause> {
        Ok(match self {
            Self::Bool => match fixed::<1>(bytes)?[0] {
                0 => Value::from_i64(0),
                1 => Value::from_i64(1),
                _ => {
                    return Err(CodecError::Value("a binary BOOL must be 0 or 1").into());
                }
            },
            Self::I16 => Value::from_i64(i16::from_be_bytes(fixed(bytes)?).into()),
            Self::I32 => Value::from_i64(i32::from_be_bytes(fixed(bytes)?).into()),
            Self::I64 => Value::from_i64(i64::from_be_bytes(fixed(bytes)?)),
            Self::F32 => native_float(f32::from_be_bytes(fixed(bytes)?).into())?,
            Self::F64 => native_float(f64::from_be_bytes(fixed(bytes)?))?,
            Self::Utf8 => Value::build_text(std::str::from_utf8(bytes)?.to_owned()),
            Self::Bytes => Value::from_slice(bytes)?,
        })
    }

    fn size(self, value: &Value) -> usize {
        if matches!(value, Value::Null) {
            return 0;
        }
        match (self, value) {
            (Self::Bool, _) => 1,
            (Self::I16, _) => 2,
            (Self::I32 | Self::F32, _) => 4,
            (Self::I64 | Self::F64, _) => 8,
            (Self::Utf8, Value::Text(text)) => text.as_str().len(),
            (Self::Bytes, Value::Blob(blob)) => blob.len(),
            _ => 0,
        }
    }

    fn append(self, value: &Value, bytes: &mut BytesMut) -> Result<Option<Range<usize>>, Cause> {
        if matches!(value, Value::Null) {
            return Ok(None);
        }
        let start = bytes.len();
        match (self, value) {
            (Self::Bool, Value::Numeric(Numeric::Integer(value @ (0 | 1)))) => {
                bytes.put_u8(*value as u8);
            }
            (Self::Bool, _) => {
                return Err(CodecError::Value("native BOOL must be integer 0 or 1").into());
            }
            (Self::I16, Value::Numeric(Numeric::Integer(value))) => bytes.put_i16(
                i16::try_from(*value)
                    .map_err(|_| CodecError::Value("native integer exceeds INT2"))?,
            ),
            (Self::I32, Value::Numeric(Numeric::Integer(value))) => bytes.put_i32(
                i32::try_from(*value)
                    .map_err(|_| CodecError::Value("native integer exceeds INT4"))?,
            ),
            (Self::I64, Value::Numeric(Numeric::Integer(value))) => bytes.put_i64(*value),
            (Self::F32 | Self::F64, Value::Numeric(value)) => {
                let value = exact_float(*value)?;
                if self == Self::F32 {
                    let narrowed = value as f32;
                    if !narrowed.is_finite() || f64::from(narrowed) != value {
                        return Err(CodecError::Value(
                            "native number cannot be represented exactly as FLOAT4",
                        )
                        .into());
                    }
                    bytes.put_f32(narrowed);
                } else {
                    bytes.put_f64(value);
                }
            }
            (Self::Utf8, Value::Text(text)) => bytes.extend_from_slice(text.as_str().as_bytes()),
            (Self::Bytes, Value::Blob(blob)) => bytes.extend_from_slice(blob),
            _ => {
                return Err(CodecError::Value(
                    "native value does not match its prepared scalar type",
                )
                .into());
            }
        }
        Ok(Some(start..bytes.len()))
    }
}

fn fixed<const N: usize>(bytes: &[u8]) -> Result<[u8; N], Cause> {
    bytes.try_into().map_err(|_| {
        Box::new(CodecError::BinaryLength {
            expected: N,
            actual: bytes.len(),
        }) as Cause
    })
}

fn native_float(value: f64) -> Result<Value, Cause> {
    if !value.is_finite() {
        return Err(
            CodecError::Value("native Turso scalar codecs do not support NaN or infinity").into(),
        );
    }
    // Value::from_f64 silently turns NaN into NULL; validate before calling it.
    Ok(Value::from_f64(value))
}

fn exact_float(value: Numeric) -> Result<f64, Cause> {
    let value = match value {
        Numeric::Float(value) => f64::from(value),
        Numeric::Integer(integer) => {
            let value = integer as f64;
            // i128 avoids the saturating f64 -> i64 cast accepting i64::MAX.
            if value as i128 != i128::from(integer) {
                return Err(CodecError::Value(
                    "native integer cannot be represented exactly as FLOAT8",
                )
                .into());
            }
            value
        }
    };
    if !value.is_finite() {
        return Err(
            CodecError::Value("native Turso scalar codecs do not support NaN or infinity").into(),
        );
    }
    Ok(value)
}

pub(crate) fn parameter_codecs(types: &[Type]) -> Result<Arc<[ScalarCodec]>, Error> {
    types
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            ScalarCodec::for_type(ty).ok_or_else(|| {
                Error::to_sql(Box::new(CodecError::UnsupportedType(ty.clone())), index)
            })
        })
        .collect()
}

// [spec:pgorm:req:turso.values]
// [spec:pgorm:req:turso.errors]
pub(crate) fn encode_params<P, I>(
    types: &[Type],
    codecs: &[ScalarCodec],
    params: I,
    scratch: &mut BytesMut,
) -> Result<Vec<Value>, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    let params = params.into_iter();
    if params.len() != types.len() {
        return Err(Error::parameters(params.len(), types.len()));
    }
    assert_eq!(
        codecs.len(),
        types.len(),
        "invalid native parameter codec plan"
    );
    let mut values = Vec::with_capacity(types.len());
    for (index, ((param, ty), codec)) in params.zip(types).zip(codecs).enumerate() {
        let param = param.borrow_to_sql();
        if !matches!(param.encode_format(ty), Format::Binary) {
            return Err(Error::to_sql(Box::new(CodecError::TextFormat), index));
        }
        scratch.clear();
        let null = param
            .to_sql_checked(ty, scratch)
            .map_err(|error| Error::to_sql(error, index))?;
        let value = match null {
            IsNull::Yes if scratch.is_empty() => Value::Null,
            IsNull::Yes => {
                return Err(Error::to_sql(
                    CodecError::Value("a null ToSql value wrote binary data").into(),
                    index,
                ));
            }
            IsNull::No => codec
                .decode(scratch)
                .map_err(|error| Error::to_sql(error, index))?,
        };
        values.push(value);
    }
    scratch.clear();
    Ok(values)
}

// [spec:pgorm:req:turso.values]
// [spec:pgorm:req:turso.rows]
// [spec:pgorm:req:turso.errors]
pub(crate) fn encode_row(
    statement: Statement,
    codecs: &[ScalarCodec],
    native: &turso_core::Row,
) -> Result<Row, Error> {
    if native.len() != statement.columns().len() || codecs.len() != native.len() {
        return Err(Error::column_count());
    }
    let size = row_size(codecs, native.get_values())?;
    encode_values(statement, codecs, native.get_values(), size)
}

fn row_size<'a>(
    codecs: &[ScalarCodec],
    values: impl Iterator<Item = &'a Value>,
) -> Result<usize, Error> {
    codecs
        .iter()
        .zip(values)
        .enumerate()
        .try_fold(0usize, |size, (index, (codec, value))| {
            size.checked_add(codec.size(value)).ok_or_else(|| {
                Error::from_sql(CodecError::Value("native row is too large").into(), index)
            })
        })
}

fn encode_values<'a>(
    statement: Statement,
    codecs: &[ScalarCodec],
    values: impl Iterator<Item = &'a Value>,
    size: usize,
) -> Result<Row, Error> {
    let mut bytes = BytesMut::with_capacity(size);
    let mut ranges = Vec::with_capacity(codecs.len());
    for (index, (codec, value)) in codecs.iter().zip(values).enumerate() {
        ranges.push(
            codec
                .append(value, &mut bytes)
                .map_err(|error| Error::from_sql(error, index))?,
        );
    }
    Ok(Row::from_native(statement, bytes.freeze(), ranges))
}

#[cfg(test)]
mod tests;
