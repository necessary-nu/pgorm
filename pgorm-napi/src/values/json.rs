//! JSON between serde_json, which pgorm's `Value::Json` holds, and JavaScript.
//!
//! A JSON number reaches JavaScript exactly or not at all. serde_json reads an
//! integer that fits 64 bits as one and anything else as an `f64`; the
//! document's own spelling is compared with what was read, so a number that
//! changed in the reading is refused rather than delivered rounded. The text
//! then crosses as text and `lib/values.js` parses it, turning an integer
//! literal past JavaScript's safe range into a `bigint`.

use neon::{prelude::*, types::JsBigInt};
use serde_json::{Map, Number, Value};

use super::read::{refuse, string};
use crate::codec::Codec;

use pgorm_bindings::JSON_MAX_DEPTH as MAX_DEPTH;

/// The largest integer a JavaScript number holds exactly, 2^53 - 1.
pub(crate) const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// A JavaScript value as JSON: `null`, a boolean, a finite number, a bigint
/// within 64 bits, a string, an array, or an object of the module's plain
/// kind. Anything JSON cannot hold is refused, never dropped or turned into a
/// string as `JSON.stringify` would.
// [spec:pgorm:req:napi.values+1]
pub(crate) fn read<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
    depth: usize,
) -> NeonResult<Value> {
    if depth > MAX_DEPTH {
        return refuse(cx, "JSON nests deeper than 64 levels, or holds a cycle");
    }
    if data.is_a::<JsNull, _>(cx) {
        return Ok(Value::Null);
    }
    if let Ok(flag) = data.downcast::<JsBoolean, _>(cx) {
        return Ok(Value::Bool(flag.value(cx)));
    }
    if let Ok(number) = data.downcast::<JsNumber, _>(cx) {
        let number = number.value(cx);
        if number.fract() == 0.0 && number.abs() <= MAX_SAFE_INTEGER {
            // An integral number keeps an integer's spelling, `1` not `1.0`;
            // negative zero is no integer and stays a float.
            if number != 0.0 || number.is_sign_positive() {
                #[allow(clippy::cast_possible_truncation)]
                return Ok(Value::Number((number as i64).into()));
            }
        }
        return match Number::from_f64(number) {
            Some(number) => Ok(Value::Number(number)),
            None => refuse(cx, "JSON has no NaN or infinite number"),
        };
    }
    if let Ok(big) = data.downcast::<JsBigInt, _>(cx) {
        if let Ok(value) = big.to_i64(cx) {
            return Ok(Value::Number(value.into()));
        }
        if let Ok(value) = big.to_u64(cx) {
            return Ok(Value::Number(value.into()));
        }
        return refuse(cx, "a JSON integer must fit 64 bits");
    }
    if data.is_a::<JsString, _>(cx) {
        return Ok(Value::String(string(cx, data)?));
    }
    if let Ok(array) = data.downcast::<JsArray, _>(cx) {
        let items = array.to_vec(cx)?;
        let mut values = Vec::with_capacity(items.len());
        for item in items {
            values.push(read(cx, codec, item, depth + 1)?);
        }
        return Ok(Value::Array(values));
    }
    if data.is_a::<JsObject, _>(cx) && !data.is_a::<JsFunction, _>(cx) {
        let described = codec.describe(cx, data)?;
        let Some(description) = described.filter(|description| description.tag == "json") else {
            return refuse(
                cx,
                "JSON holds only null, booleans, numbers, bigints, strings, arrays and plain \
                 objects; encode any other value as one of them first",
            );
        };
        let entries = description
            .field(cx, 0)
            .downcast_or_throw::<JsArray, _>(cx)?
            .to_vec(cx)?;
        let mut object = Map::new();
        for entry in entries {
            let entry = entry.downcast_or_throw::<JsArray, _>(cx)?;
            let key: Handle<JsValue> = entry.get(cx, 0)?;
            let key = string(cx, key)?;
            let value: Handle<JsValue> = entry.get(cx, 1)?;
            if value.is_a::<JsUndefined, _>(cx) {
                return refuse(
                    cx,
                    format!("JSON has no undefined: property {key:?} must be null or be left out"),
                );
            }
            object.insert(key, read(cx, codec, value, depth + 1)?);
        }
        return Ok(Value::Object(object));
    }
    refuse(
        cx,
        "JSON holds only null, booleans, numbers, bigints, strings, arrays and plain objects",
    )
}
