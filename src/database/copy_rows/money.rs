//! `money` in binary COPY rows. A `money` travels as an `int8` count of its
//! currency's minor units, whose places only the session's `lc_monetary`
//! knows, so a description asks the server for them and the rows of a
//! `money` column are moved as the `Decimal` amount the server's cast to
//! `numeric` gives, counted at those places.

use bytes::{BufMut, BytesMut};
use pgorm_pool::GenericClient;
use pgorm_query::{ArrayType, Value, base_type};
use postgres_protocol::types::{self as wire, ArrayDimension};
use rust_decimal::{Decimal, RoundingStrategy, prelude::ToPrimitive};
use tokio_postgres::types::{FromSql, IsNull, Kind, ToSql, Type};

use super::{Element, WireError};
use crate::{Error, error::query_err};

/// Whether a column of `ty` holds `money`: the type itself, an array of it,
/// or a domain over either.
pub(super) fn holds_money(ty: &Type) -> bool {
    let ty = base_type(ty);
    match ty.kind() {
        Kind::Array(member) => *base_type(member) == Type::MONEY,
        _ => *ty == Type::MONEY,
    }
}

/// The places of the currency the session's `lc_monetary` names, as the
/// server converts a `money` to `numeric` at them.
// [spec:pgorm:req:conn.copy.rows+5]
pub(super) async fn money_places<C>(client: &C) -> Result<u32, Error>
where
    C: GenericClient,
{
    let row = client
        .query_one("SELECT scale(CAST(CAST(0 AS money) AS numeric))", &[])
        .await?;
    u32::try_from(row.try_get::<_, i32>(0)?)
        .map_err(|_| query_err("the server reports a negative scale for money"))
}

/// `value` as a field of a `money` column, or an array of one, counted in a
/// currency of `places`: bound as the `numeric` it binds as (so a `Decimal`
/// and an integer, whole units, are taken), and its amount multiplied out to
/// the currency's minor units, rounded half away from zero as PostgreSQL's
/// cast of a `numeric` to `money` rounds it.
// [spec:pgorm:req:conn.copy.rows+5]
pub(super) fn money_to_sql<T: ToSql>(
    value: &T,
    ty: &Type,
    places: u32,
    out: &mut BytesMut,
) -> Result<IsNull, WireError> {
    let member = match base_type(ty).kind() {
        Kind::Array(member) => Some(member),
        _ => None,
    };
    let carrier = match member {
        Some(_) => Type::NUMERIC_ARRAY,
        None => Type::NUMERIC,
    };
    let mut numeric = BytesMut::new();
    if let IsNull::Yes = value.to_sql_checked(&carrier, &mut numeric)? {
        return Ok(IsNull::Yes);
    }
    let Some(member) = member else {
        out.put_i64(minor_units(Decimal::from_sql(&carrier, &numeric)?, places)?);
        return Ok(IsNull::No);
    };
    let amounts: Vec<Option<Decimal>> = FromSql::from_sql(&carrier, &numeric)?;
    let units = amounts
        .into_iter()
        .map(|amount| amount.map(|amount| minor_units(amount, places)).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    let length =
        i32::try_from(units.len()).map_err(|_| "the array is longer than PostgreSQL holds")?;
    wire::array_to_sql(
        [ArrayDimension {
            len: length,
            lower_bound: 1,
        }],
        member.oid(),
        units,
        |unit, out| match unit {
            Some(unit) => {
                out.put_i64(unit);
                Ok(postgres_protocol::IsNull::No)
            }
            None => Ok(postgres_protocol::IsNull::Yes),
        },
        out,
    )?;
    Ok(IsNull::No)
}

/// `amount` in the minor units of a currency of `places`.
fn minor_units(amount: Decimal, places: u32) -> Result<i64, WireError> {
    let scale = 10_i64
        .checked_pow(places)
        .map(Decimal::from)
        .ok_or("the currency has more places than money holds")?;
    amount
        .checked_mul(scale)
        .map(|units| units.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero))
        .and_then(|units| units.to_i64())
        .ok_or_else(|| format!("value `{amount}` is out of range for Postgres type `money`").into())
}

/// A field of a `money` column, or an array of one, counted in a currency of
/// `places`, as the `Decimal` amount the server's cast to `numeric` gives.
// [spec:pgorm:req:conn.copy.rows+5]
pub(super) fn decode_money(ty: &Type, places: u32, raw: Option<&[u8]>) -> Result<Value, WireError> {
    let amount = |raw: &[u8]| -> Result<Value, WireError> {
        let units = i64::from_sql(&Type::INT8, raw)?;
        Ok(Value::Decimal(Some(Box::new(Decimal::new(units, places)))))
    };
    let ty = base_type(ty);
    if !matches!(ty.kind(), Kind::Array(_)) {
        return raw.map_or(Ok(Value::Decimal(None)), amount);
    }
    let Some(raw) = raw else {
        return Ok(Value::Array(ArrayType::Decimal, None));
    };
    let elements: Vec<Option<Element<'_>>> = FromSql::from_sql(ty, raw)?;
    let amounts = elements
        .into_iter()
        .map(|element| match element {
            None => Ok(Value::Decimal(None)),
            Some(Element(raw)) => amount(raw),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(ArrayType::Decimal, Some(Box::new(amounts))))
}
