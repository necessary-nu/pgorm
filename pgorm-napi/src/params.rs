//! Statement parameters: JavaScript values bound, never interpolated, through
//! pgorm's own `ValueHolder`, which writes each against the type PostgreSQL
//! inferred for its placeholder.

use std::error::Error;

use bytes::BytesMut;
use neon::prelude::*;
use pgorm::{
    ValueHolder,
    pgorm_query::{Value, base_type},
    types::{IsNull, Kind, ToSql, Type, to_sql_checked},
};

use crate::{codec::Codec, values::read};

type BindError = Box<dyn Error + Sync + Send>;

/// One bound parameter.
#[derive(Debug)]
pub(crate) enum Param {
    /// SQL NULL of no declared kind, which binds to whatever type the
    /// placeholder has.
    Null,
    Value(ValueHolder),
}

impl Param {
    pub(crate) fn native_value(&self) -> Option<&Value> {
        match self {
            Self::Null => None,
            Self::Value(holder) => Some(&holder.0),
        }
    }
}

/// `params`, a JavaScript array, as parameters: each a `Value`, or a plain
/// value whose kind is inferred.
// [spec:pgorm:req:napi.inference]
pub(crate) fn read<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    params: Handle<'cx, JsArray>,
) -> NeonResult<Vec<Param>> {
    let items = params.to_vec(cx)?;
    let mut params = Vec::with_capacity(items.len());
    for item in items {
        params.push(match read::infer(cx, codec, item)? {
            None => Param::Null,
            Some(value) => Param::Value(ValueHolder(value.datum)),
        });
    }
    Ok(params)
}

/// Refuse a civil date-time bound to `timestamptz`, or an instant to
/// `timestamp`. pgorm writes both as microseconds from 2000-01-01, so either
/// would be accepted and read as UTC, where PostgreSQL's own cast between the
/// two reads the wall clock in the session's time zone: a silent
/// reinterpretation, which the binding does not make.
// [spec:pgorm:req:napi.temporal+1]
fn check_temporal(value: &Value, ty: &Type) -> Result<(), BindError> {
    let ty = base_type(ty);
    match value {
        Value::DateTime(Some(_)) if *ty == Type::TIMESTAMPTZ => Err(
            "a Temporal.PlainDateTime has no time zone and does not bind to timestamptz: \
             pass an Instant, e.g. plain.toZonedDateTime(zone).toInstant()"
                .into(),
        ),
        Value::DateTimeWithTimeZone(Some(_)) if *ty == Type::TIMESTAMP => Err(
            "a Temporal.Instant does not bind to timestamp, which has no time zone: pass a \
             PlainDateTime, e.g. instant.toZonedDateTimeISO(zone).toPlainDateTime()"
                .into(),
        ),
        Value::Array(_, Some(items)) => match ty.kind() {
            Kind::Array(member) => items
                .iter()
                .try_for_each(|item| check_temporal(item, member)),
            _ => Ok(()),
        },
        Value::Range(_, Some(range)) => match ty.kind() {
            Kind::Range(subtype) => [range.lower(), range.upper()]
                .into_iter()
                .flatten()
                .try_for_each(|bound| bound_check(bound, subtype)),
            _ => Ok(()),
        },
        Value::Multirange(_, Some(ranges)) => match ty.kind() {
            Kind::Multirange(subtype) => ranges
                .iter()
                .flat_map(|range| [range.lower(), range.upper()])
                .flatten()
                .try_for_each(|bound| bound_check(bound, subtype)),
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

fn bound_check(bound: &std::ops::Bound<Value>, subtype: &Type) -> Result<(), BindError> {
    match bound {
        std::ops::Bound::Included(value) | std::ops::Bound::Excluded(value) => {
            check_temporal(value, subtype)
        }
        std::ops::Bound::Unbounded => Ok(()),
    }
}

impl ToSql for Param {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, BindError> {
        match self {
            Self::Null => Ok(IsNull::Yes),
            Self::Value(holder) => {
                check_temporal(&holder.0, ty)?;
                holder.to_sql(ty, out)
            }
        }
    }

    /// Every type is accepted here, as `ValueHolder` accepts every type: the
    /// decision needs the value, which only `to_sql` has.
    fn accepts(_: &Type) -> bool {
        true
    }

    to_sql_checked!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::{Timestamp, civil::DateTime};

    fn bind(param: &Param, ty: &Type) -> Result<IsNull, BindError> {
        param.to_sql(ty, &mut BytesMut::new())
    }

    // [spec:pgorm:req:napi.temporal+1/test]
    #[test]
    fn temporal_values_bind_only_to_their_own_type() {
        let civil = Param::Value(ValueHolder(Value::DateTime(Some(Box::new(
            DateTime::constant(2026, 10, 9, 12, 0, 0, 0),
        )))));
        let instant = Param::Value(ValueHolder(Value::DateTimeWithTimeZone(Some(Box::new(
            Timestamp::UNIX_EPOCH,
        )))));
        assert!(bind(&civil, &Type::TIMESTAMP).is_ok());
        assert!(bind(&civil, &Type::TIMESTAMPTZ).is_err());
        assert!(bind(&instant, &Type::TIMESTAMPTZ).is_ok());
        assert!(bind(&instant, &Type::TIMESTAMP).is_err());
        let civil_array = Param::Value(ValueHolder(Value::Array(
            pgorm::pgorm_query::ArrayType::DateTime,
            Some(Box::new(vec![Value::DateTime(Some(Box::new(
                DateTime::constant(2026, 10, 9, 12, 0, 0, 0),
            )))])),
        )));
        assert!(bind(&civil_array, &Type::TIMESTAMP_ARRAY).is_ok());
        assert!(bind(&civil_array, &Type::TIMESTAMPTZ_ARRAY).is_err());
        assert!(matches!(
            bind(&Param::Null, &Type::TIMESTAMPTZ),
            Ok(IsNull::Yes)
        ));
    }
}
