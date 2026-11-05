//! The binding's values as plain JavaScript values: each kind as the one type
//! the declarations name for it, and SQL NULL as `null`.

use std::ops::Bound;

use neon::{prelude::*, types::JsBigInt};
use pgorm::pgorm_query::{Range, Value};

use super::{Tag, Tagged};
use crate::{
    codec::Codec,
    errors::{Failure, throw},
};

/// Throw a `DecodeError`: a value has no form in the JavaScript type its kind
/// declares.
fn undecodable<'cx, T>(cx: &mut Cx<'cx>, message: impl Into<String>) -> NeonResult<T> {
    let error = Failure::Decode(message.into()).into_js(cx)?;
    cx.throw(error)
}

/// A value as the plain JavaScript value of its kind.
// [spec:pgorm:req:bindings.values]
pub(crate) fn plain<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    tagged: &Tagged,
) -> JsResult<'cx, JsValue> {
    match (&tagged.tag, &tagged.datum) {
        (Tag::Created(kind), Value::String(Some(text))) => {
            let Some(ranges) = kind.codec().and_then(|codec| codec.ranges(text)) else {
                return undecodable(cx, format!("{text:?} is no value of {}", kind.name.name));
            };
            if kind.multirange {
                multirange(cx, codec, &ranges)
            } else {
                match ranges.first() {
                    Some(first) => range(cx, codec, first),
                    None => undecodable(cx, "a created range's text holds no range"),
                }
            }
        }
        // A registered entity's column of an array of a created range reads
        // each element as the kind's value.
        (Tag::Array(element), Value::Array(_, Some(items)))
            if matches!(element.as_ref(), Tag::Created(_)) =>
        {
            let array = JsArray::new(cx, items.len());
            for (index, item) in items.iter().enumerate() {
                let item = plain(
                    cx,
                    codec,
                    &Tagged {
                        tag: element.as_ref().clone(),
                        datum: item.clone(),
                    },
                )?;
                array.set(cx, index_u32(index), item)?;
            }
            Ok(array.upcast())
        }
        (_, value) => self::value(cx, codec, value),
    }
}

fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

fn range<'cx>(cx: &mut Cx<'cx>, codec: Codec<'cx>, range: &Range<Value>) -> JsResult<'cx, JsValue> {
    let Range::Bounds { lower, upper } = range else {
        return codec.make(cx, ("empty",));
    };
    let side =
        |cx: &mut Cx<'cx>, bound: &Bound<Value>| -> NeonResult<(Handle<'cx, JsValue>, bool)> {
            Ok(match bound {
                Bound::Included(value) | Bound::Excluded(value) if !value.is_null() => (
                    self::value(cx, codec, value)?,
                    matches!(bound, Bound::Included(_)),
                ),
                _ => (cx.null().upcast(), false),
            })
        };
    let (lower, lower_inclusive) = side(cx, lower)?;
    let (upper, upper_inclusive) = side(cx, upper)?;
    codec.make(
        cx,
        ("range", lower, upper, lower_inclusive, upper_inclusive),
    )
}

fn multirange<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    ranges: &[Range<Value>],
) -> JsResult<'cx, JsValue> {
    let array = JsArray::new(cx, ranges.len());
    for (index, item) in ranges.iter().enumerate() {
        let item = range(cx, codec, item)?;
        array.set(cx, index_u32(index), item)?;
    }
    codec.make(cx, ("multirange", array))
}

/// A Temporal time's fields from a civil time's nanoseconds, which hold whole
/// microseconds: every time pgorm reads or the binding writes does.
fn subsecond(nanoseconds: i32) -> (i32, i32) {
    (nanoseconds / 1_000_000, nanoseconds / 1_000 % 1_000)
}

/// A pgorm value as plain JavaScript.
// [spec:pgorm:req:bindings.values]
// [spec:pgorm:req:napi.temporal+1]
pub(crate) fn value<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    value: &Value,
) -> JsResult<'cx, JsValue> {
    if value.is_null() {
        return Ok(cx.null().upcast());
    }
    Ok(match value {
        Value::Bool(Some(flag)) => cx.boolean(*flag).upcast(),
        Value::TinyInt(Some(number)) => cx.number(*number).upcast(),
        Value::SmallInt(Some(number)) => cx.number(*number).upcast(),
        Value::Int(Some(number)) => cx.number(*number).upcast(),
        Value::Unsigned(Some(number)) => cx.number(*number).upcast(),
        Value::BigInt(Some(number)) => JsBigInt::from_i64(cx, *number).upcast(),
        Value::BigUnsigned(Some(number)) => JsBigInt::from_u64(cx, *number).upcast(),
        Value::Float(Some(number)) => cx.number(f64::from(*number)).upcast(),
        Value::Double(Some(number)) => cx.number(*number).upcast(),
        Value::String(Some(text)) => cx.string(text.as_str()).upcast(),
        Value::Char(Some(single)) => cx.string(single.to_string()).upcast(),
        Value::Bytes(Some(bytes)) => JsUint8Array::from_slice(cx, bytes)?.upcast(),
        Value::Json(Some(json)) => {
            let text = json.to_string();
            codec.make(cx, ("json", text))?
        }
        Value::Decimal(Some(decimal)) => codec.make(cx, ("decimal", decimal.to_string()))?,
        Value::Uuid(Some(uuid)) => codec.make(cx, ("uuid", uuid.hyphenated().to_string()))?,
        Value::Date(Some(date)) => codec.make(
            cx,
            (
                "date",
                i32::from(date.year()),
                i32::from(date.month()),
                i32::from(date.day()),
            ),
        )?,
        Value::Time(Some(time)) => {
            let (millisecond, microsecond) = subsecond(time.subsec_nanosecond());
            codec.make(
                cx,
                (
                    "time",
                    i32::from(time.hour()),
                    i32::from(time.minute()),
                    i32::from(time.second()),
                    millisecond,
                    microsecond,
                ),
            )?
        }
        Value::DateTime(Some(datetime)) => {
            let (millisecond, microsecond) = subsecond(datetime.subsec_nanosecond());
            codec.make(
                cx,
                (
                    "datetime",
                    i32::from(datetime.year()),
                    i32::from(datetime.month()),
                    i32::from(datetime.day()),
                    i32::from(datetime.hour()),
                    i32::from(datetime.minute()),
                    i32::from(datetime.second()),
                    millisecond,
                    microsecond,
                ),
            )?
        }
        Value::DateTimeWithTimeZone(Some(instant)) => {
            let nanoseconds = JsBigInt::from_i128(cx, instant.as_nanosecond());
            codec.make(cx, ("instant", nanoseconds))?
        }
        Value::IpNetwork(Some(network)) => cx.string(network.to_string()).upcast(),
        Value::MacAddress(Some(address)) => {
            JsUint8Array::from_slice(cx, &address.bytes())?.upcast()
        }
        Value::Vector(Some(vector)) => JsFloat32Array::from_slice(cx, vector.as_slice())?.upcast(),
        Value::Array(_, Some(items)) => {
            let array = JsArray::new(cx, items.len());
            for (index, item) in items.iter().enumerate() {
                let item = self::value(cx, codec, item)?;
                array.set(cx, index_u32(index), item)?;
            }
            array.upcast()
        }
        Value::Range(_, Some(value)) => range(cx, codec, value)?,
        Value::Multirange(_, Some(value)) => {
            let ranges: Vec<Range<Value>> = value.iter().cloned().collect();
            multirange(cx, codec, &ranges)?
        }
        Value::Interval(Some(interval)) => {
            let microseconds = JsBigInt::from_i64(cx, interval.microseconds);
            codec.make(
                cx,
                ("interval", interval.months, interval.days, microseconds),
            )?
        }
        _ => return throw(cx, super::unrepresented()),
    })
}
