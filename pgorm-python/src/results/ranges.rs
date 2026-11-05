//! Range and multirange columns, read through pgorm's range readers, each
//! bound by the same codec a scalar of the subtype is read with, so a numeric
//! bound is held to the same exactness.

use jiff::{
    Timestamp,
    civil::{Date, DateTime},
};
use pgorm::{
    CreatedRangeCodec, CreatedRangeText, ExactDecimal, MultirangeSql, RangeSql,
    pgorm_query::{ArrayType, RangeType, Value},
};
use pyo3::PyResult;
use tokio_postgres::types::{Kind, Type};

use super::decode::{Field, typed};
use crate::{
    errors::DecodeError,
    values::{CreatedKind, PyTypeName, PyValue},
};

/// The built-in range type over `subtype`, which a range type a schema
/// created over the same subtype reads as.
fn range_type(subtype: &Type) -> Option<RangeType> {
    Some(match *subtype {
        Type::INT4 => RangeType::Int4,
        Type::INT8 => RangeType::Int8,
        Type::NUMERIC => RangeType::Numeric,
        Type::DATE => RangeType::Date,
        Type::TIMESTAMP => RangeType::Timestamp,
        Type::TIMESTAMPTZ => RangeType::TimestampTz,
        _ => return None,
    })
}

/// Whether `ty` is one of the six built-in range types or their multiranges,
/// rather than a type a schema created.
fn built_in(ty: &Type) -> bool {
    matches!(
        *ty,
        Type::INT4_RANGE
            | Type::INT8_RANGE
            | Type::NUM_RANGE
            | Type::DATE_RANGE
            | Type::TS_RANGE
            | Type::TSTZ_RANGE
            | Type::INT4MULTI_RANGE
            | Type::INT8MULTI_RANGE
            | Type::NUMMULTI_RANGE
            | Type::DATEMULTI_RANGE
            | Type::TSMULTI_RANGE
            | Type::TSTZMULTI_RANGE
    )
}

/// A range or multirange column, or an array of either, as a tagged value;
/// `None` for a column of any other type. A range type a schema created reads
/// as its own kind, named as the column's type is, whatever its subtype.
// [spec:pgorm:req:bindings.values]
// [spec:pgorm:def:exec.decode.range+4]
pub(super) fn value(field: Field<'_>, ty: &Type, array: bool) -> PyResult<Option<PyValue>> {
    let index = field.index();
    let (subtype, multi) = match ty.kind() {
        Kind::Range(subtype) => (subtype, false),
        Kind::Multirange(subtype) => (subtype, true),
        _ => return Ok(None),
    };
    if !built_in(ty) {
        return created(field, ty, subtype, multi, array).map(Some);
    }
    let Some(range) = range_type(subtype) else {
        return Err(DecodeError::new_err(format!(
            "column {index} ranges over `{subtype}`, which none of the built-in range types does"
        )));
    };
    macro_rules! decode {
        ($rust:ty, $bound:expr) => {
            if multi {
                typed::<MultirangeSql<$rust>>(field, array, ArrayType::Multirange(range), |v| {
                    Value::Multirange(range, v.map(|v| Box::new(v.0.map($bound))))
                })
            } else {
                typed::<RangeSql<$rust>>(field, array, ArrayType::Range(range), |v| {
                    Value::Range(range, v.map(|v| Box::new(v.0.map($bound))))
                })
            }
        };
    }
    let value = match range {
        RangeType::Int4 => decode!(i32, |v| Value::Int(Some(v))),
        RangeType::Int8 => decode!(i64, |v| Value::BigInt(Some(v))),
        RangeType::Numeric => decode!(ExactDecimal, |v: ExactDecimal| Value::Decimal(Some(
            Box::new(v.0)
        ))),
        RangeType::Date => decode!(Date, |v| Value::Date(Some(Box::new(v)))),
        RangeType::Timestamp => decode!(DateTime, |v| Value::DateTime(Some(Box::new(v)))),
        RangeType::TimestampTz => decode!(Timestamp, |v| Value::DateTimeWithTimeZone(Some(
            Box::new(v)
        ))),
        _ => Err(crate::values::unrepresented()),
    }?;
    Ok(Some(PyValue::from_rust(value)))
}

/// A column of a range type a schema created, or of an array of one: read
/// through pgorm's codec of the type, each element of an array alike, and
/// held as the text form that type's values travel as, tagged with the
/// type's name.
// [spec:pgorm:req:bindings.created-ranges+3]
fn created(
    field: Field<'_>,
    ty: &Type,
    subtype: &Type,
    multi: bool,
    array: bool,
) -> PyResult<PyValue> {
    let Some(kind) = CreatedRangeCodec::subtype_kind(subtype) else {
        return Err(DecodeError::new_err(format!(
            "column {} ranges over `{subtype}`, which a created range read from Python cannot",
            field.index()
        )));
    };
    let value = typed::<CreatedRangeText>(field, array, ArrayType::String, |v| {
        Value::String(v.map(|v| Box::new(v.0)))
    })?;
    let kind = CreatedKind {
        name: PyTypeName {
            name: ty.name().to_owned(),
            schema: Some(ty.schema().to_owned()),
        },
        subtype: kind,
        multirange: multi,
    };
    Ok(match value {
        Value::String(text) => PyValue::from_created(text.map(|text| *text), kind),
        array => PyValue::from_created_array(array, kind),
    })
}
