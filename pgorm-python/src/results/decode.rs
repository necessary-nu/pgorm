use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use pgorm::{
    CopyColumn,
    pgorm_query::{ArrayType, Interval, Value, Vector},
};
use pyo3::prelude::*;
use tokio_postgres::{
    Row,
    types::{FromSql, Kind, Type},
};
use uuid::Uuid;

use super::codecs::{CheckedArray, EnumLabel, Inet, Mac};
use super::json::ExactJson;
use crate::{
    errors::DecodeError,
    values::{PyTypeName, PyValue},
};
use pgorm::ExactDecimal;

/// Where a column's value is read from: a result row, or one field of a
/// binary COPY's row, its bytes and the column the server describes for it.
/// Both read through the same codecs, so a COPY's value is the value a query
/// of the same column gives.
#[derive(Clone, Copy)]
pub(crate) enum Field<'a> {
    Row(&'a Row, usize),
    Copy {
        column: &'a CopyColumn,
        raw: Option<&'a [u8]>,
        index: usize,
    },
}

impl Field<'_> {
    fn ty(&self) -> &Type {
        match self {
            Self::Row(row, index) => row.columns()[*index].type_(),
            Self::Copy { column, .. } => column.type_(),
        }
    }

    pub(super) fn index(&self) -> usize {
        match self {
            Self::Row(_, index) | Self::Copy { index, .. } => *index,
        }
    }
}

pub(super) fn read<T: for<'a> FromSql<'a>>(field: Field<'_>) -> PyResult<Option<T>> {
    let refused = || {
        DecodeError::new_err(format!(
            "column {} cannot be decoded without loss; its PostgreSQL type or value is unsupported",
            field.index()
        ))
    };
    match field {
        Field::Row(row, index) => row.try_get(index).map_err(|_| refused()),
        Field::Copy { column, raw, .. } => {
            let ty = column.type_();
            if !<Option<T> as FromSql>::accepts(ty) {
                return Err(refused());
            }
            <Option<T> as FromSql>::from_sql_nullable(ty, raw).map_err(|_| refused())
        }
    }
}

pub(super) fn typed<T: for<'a> FromSql<'a>>(
    field: Field<'_>,
    array: bool,
    kind: ArrayType,
    convert: impl Fn(Option<T>) -> Value,
) -> PyResult<Value> {
    if array {
        let values = read::<CheckedArray<T>>(field)?
            .map(|values| Box::new(values.0.into_iter().map(convert).collect()));
        Ok(Value::Array(kind, values))
    } else {
        read::<T>(field).map(convert)
    }
}

// [spec:pgorm:req:bindings.values]
// [spec:pgorm:req:bindings.value-tags+1]
pub(crate) fn value(field: Field<'_>) -> PyResult<PyValue> {
    let index = field.index();
    // A COPY's `money` field is the count of its currency's minor units,
    // which pgorm reads as a `Decimal` at the places the COPY's description
    // learned; a query's `money` column has no places beside it and is
    // refused below, as a query reads it cast to `numeric`.
    // [spec:pgorm:req:bindings.copy-rows+1]
    if let Field::Copy { column, raw, .. } = field
        && column.money_places().is_some()
    {
        return match column.decode(raw) {
            Ok(value) => Ok(PyValue::from_rust(value)),
            Err(pgorm::Error::Type(message)) => Err(DecodeError::new_err(message)),
            Err(error) => Err(DecodeError::new_err(error.to_string())),
        };
    }
    let declared = field.ty();
    let (ty, array) = match declared.kind() {
        Kind::Array(member) => (member, true),
        _ => (declared, false),
    };
    macro_rules! scalar {
        ($rust:ty, $variant:ident) => {
            typed::<$rust>(field, array, ArrayType::$variant, Value::$variant)
        };
    }
    macro_rules! boxed {
        ($rust:ty, $variant:ident) => {
            typed::<$rust>(field, array, ArrayType::$variant, |v| {
                Value::$variant(v.map(Box::new))
            })
        };
    }
    if let Some(value) = super::ranges::value(field, ty, array)? {
        return Ok(value);
    }
    if matches!(ty.kind(), Kind::Enum(_)) {
        let inner = typed::<EnumLabel>(field, array, ArrayType::String, |v| {
            Value::String(v.map(|v| Box::new(v.0)))
        })?;
        return Ok(PyValue::from_enum(
            inner,
            PyTypeName {
                name: ty.name().to_owned(),
                schema: Some(ty.schema().to_owned()),
            },
            array,
        ));
    }
    let inner = match *ty {
        Type::BOOL => scalar!(bool, Bool),
        Type::CHAR => scalar!(i8, TinyInt),
        Type::INT2 => scalar!(i16, SmallInt),
        Type::INT4 => scalar!(i32, Int),
        Type::INT8 => scalar!(i64, BigInt),
        Type::OID => scalar!(u32, Unsigned),
        Type::FLOAT4 => scalar!(f32, Float),
        Type::FLOAT8 => scalar!(f64, Double),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => boxed!(String, String),
        Type::BYTEA => boxed!(Vec<u8>, Bytes),
        Type::JSON | Type::JSONB => typed::<ExactJson>(field, array, ArrayType::Json, |v| {
            Value::Json(v.map(|v| Box::new(v.0)))
        }),
        Type::UUID => boxed!(Uuid, Uuid),
        Type::DATE => boxed!(Date, Date),
        // The driver's civil-time codec already refuses PostgreSQL 24:00:00 and
        // every other value outside a civil day, so no exactness wrapper is needed.
        Type::TIME => boxed!(Time, Time),
        Type::TIMESTAMP => boxed!(DateTime, DateTime),
        Type::TIMESTAMPTZ => boxed!(Timestamp, DateTimeWithTimeZone),
        Type::NUMERIC => typed::<ExactDecimal>(field, array, ArrayType::Decimal, |v| {
            Value::Decimal(v.map(|v| Box::new(v.0)))
        }),
        Type::INET | Type::CIDR => typed::<Inet>(field, array, ArrayType::IpNetwork, |v| {
            Value::IpNetwork(v.map(|v| Box::new(v.0)))
        }),
        Type::MACADDR => typed::<Mac>(field, array, ArrayType::MacAddress, |v| {
            Value::MacAddress(v.map(|v| Box::new(v.0)))
        }),
        // [spec:pgorm:req:bindings.values]
        Type::INTERVAL => boxed!(Interval, Interval),
        _ if ty.name() == "vector" && matches!(ty.kind(), Kind::Simple) => boxed!(Vector, Vector),
        _ => {
            return Err(DecodeError::new_err(format!(
                "column {index} has an unsupported PostgreSQL type"
            )));
        }
    }?;
    Ok(PyValue::from_rust(inner))
}
