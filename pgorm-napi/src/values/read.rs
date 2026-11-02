//! JavaScript values into the binding's value model: as a declared kind, or
//! inferred from a plain JavaScript value where its type leaves one answer.
//!
//! Nothing here rounds, wraps, truncates or stringifies. A value that does not
//! fit its kind exactly is refused with a `ConstructionError`.

use std::ops::Bound;

use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use neon::{
    prelude::*,
    types::{JsBigInt, buffer::TypedArray},
};
use pgorm::pgorm_query::{
    ArrayType, Interval, IpNetwork, MacAddress, Multirange, Range, Value, Vector,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::{
    CREATED_SUBTYPES, CreatedKind, Tag, Tagged, TypeName, created,
    json::{self, MAX_SAFE_INTEGER},
};
use crate::{
    codec::{Codec, Description},
    errors::{Failure, throw},
};

/// Throw a `ConstructionError`: the argument cannot become the value pgorm
/// sends.
pub(crate) fn refuse<'cx, T>(cx: &mut Cx<'cx>, message: impl Into<String>) -> NeonResult<T> {
    let error = Failure::Construction(message.into()).into_js(cx)?;
    cx.throw(error)
}

/// The string an argument that is text must be, `what` naming it in the
/// `TypeError` anything else is. A value to bind has no wrong type, since any
/// value may be a parameter; one that cannot be bound is `refuse`d.
// [spec:pgorm:req:bindings.argument-errors+1]
pub(crate) fn text<'cx>(
    cx: &mut Cx<'cx>,
    data: Handle<'cx, JsValue>,
    what: &str,
) -> NeonResult<String> {
    if !data.is_a::<JsString, _>(cx) {
        return cx.throw_type_error(format!("{what} is a string"));
    }
    string(cx, data)
}

/// A JavaScript string as UTF-8. A lone surrogate has no UTF-8 form, and is
/// refused rather than replaced with U+FFFD as Node-API's own conversion
/// would.
// [spec:pgorm:req:napi.values+1]
pub(crate) fn string<'cx>(cx: &mut Cx<'cx>, data: Handle<'cx, JsValue>) -> NeonResult<String> {
    let Ok(text) = data.downcast::<JsString, _>(cx) else {
        return refuse(cx, "expected a string");
    };
    let units = text.to_utf16(cx);
    match String::from_utf16(&units) {
        Ok(text) => Ok(text),
        Err(_) => refuse(
            cx,
            "the string holds a lone surrogate, which UTF-8 and PostgreSQL text cannot hold",
        ),
    }
}

/// Statement text: a strict string, without NUL, which the protocol's
/// NUL-terminated strings cannot carry.
pub(crate) fn sql<'cx>(cx: &mut Cx<'cx>, data: Handle<'cx, JsValue>) -> NeonResult<String> {
    if !data.is_a::<JsString, _>(cx) {
        return cx.throw_type_error("the statement is a string");
    }
    let text = string(cx, data)?;
    if text.contains('\0') {
        return refuse(cx, "statement text cannot contain NUL");
    }
    Ok(text)
}

fn describe<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
) -> NeonResult<Option<Description<'cx>>> {
    if data.is_a::<JsObject, _>(cx) && !data.is_a::<JsFunction, _>(cx) {
        codec.describe(cx, data)
    } else {
        Ok(None)
    }
}

/// What the module's `Value` wraps.
fn boxed<'cx>(cx: &mut Cx<'cx>, data: Handle<'cx, JsValue>) -> NeonResult<Tagged> {
    match data.downcast::<JsBox<Tagged>, _>(cx) {
        Ok(boxed) => Ok((**boxed).clone()),
        Err(_) => cx.throw_type_error("a Value's native half is missing"),
    }
}

/// A kind: a name such as `"i32"` or `"tstzrange"`, a `TypeName` for an enum,
/// or a `CreatedRange` / `CreatedMultirange`.
// [spec:pgorm:req:bindings.value-tags]
pub(crate) fn kind<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
) -> NeonResult<Tag> {
    if data.is_a::<JsString, _>(cx) {
        let name = string(cx, data)?;
        return match ArrayType::from_kind_name(&name) {
            Some(scalar) => Ok(Tag::Scalar(scalar)),
            None => refuse(cx, format!("{name:?} is not a value kind")),
        };
    }
    match describe(cx, codec, data)? {
        Some(description) if description.tag == "typename" => {
            Ok(Tag::Enum(type_name(cx, &description, 0)?))
        }
        Some(description) if description.tag == "created" => {
            let name = type_name(cx, &description, 0)?;
            let subtype = description.field(cx, 2);
            let subtype = string(cx, subtype)?;
            let multirange = description.field(cx, 3);
            let multirange = multirange.downcast_or_throw::<JsBoolean, _>(cx)?.value(cx);
            let subtype = match ArrayType::from_kind_name(&subtype) {
                Some(kind) if CREATED_SUBTYPES.contains(&kind) => kind,
                _ => {
                    return refuse(
                        cx,
                        "a created range's subtype is one of i16, i32, i64, f32, f64, decimal, \
                         text, date, time, datetime, datetime_utc or uuid",
                    );
                }
            };
            Ok(Tag::Created(CreatedKind {
                name,
                subtype,
                multirange,
            }))
        }
        _ => cx.throw_type_error(
            "a kind is a kind name, a TypeName, a CreatedRange or a CreatedMultirange",
        ),
    }
}

/// The type a `TypeName` names, or `None` for anything else.
pub(crate) fn named_type<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
) -> NeonResult<Option<TypeName>> {
    match describe(cx, codec, data)? {
        Some(description) if description.tag == "typename" => {
            type_name(cx, &description, 0).map(Some)
        }
        _ => Ok(None),
    }
}

/// The name and optional schema at `index` and `index + 1`, each 1–63 UTF-8
/// bytes without NUL.
fn type_name<'cx>(
    cx: &mut Cx<'cx>,
    description: &Description<'cx>,
    index: usize,
) -> NeonResult<TypeName> {
    let name = description.field(cx, index);
    let name = string(cx, name)?;
    let schema = description.field(cx, index + 1);
    let schema = if schema.is_a::<JsNull, _>(cx) || schema.is_a::<JsUndefined, _>(cx) {
        None
    } else {
        Some(string(cx, schema)?)
    };
    for part in std::iter::once(&name).chain(schema.iter()) {
        if part.is_empty() || part.contains('\0') || part.len() > 63 {
            return refuse(cx, "type name parts are 1–63 UTF-8 bytes without NUL");
        }
    }
    Ok(TypeName { name, schema })
}

/// A value of the declared `tag`. `null` is SQL NULL of the kind.
// [spec:pgorm:req:bindings.value-tags]
pub(crate) fn tagged<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
    tag: &Tag,
) -> NeonResult<Tagged> {
    if data.is_a::<JsNull, _>(cx) {
        return Ok(Tagged::null(tag.clone()));
    }
    if let Some(description) = describe(cx, codec, data)?
        && description.tag == "value"
    {
        let value = description.field(cx, 0);
        let value = boxed(cx, value)?;
        if value.tag != *tag {
            return refuse(
                cx,
                format!(
                    "a {} value is not a value of kind {}",
                    value.tag.name(),
                    tag.name()
                ),
            );
        }
        return Ok(value);
    }
    let datum = match tag {
        Tag::Scalar(kind) => scalar(cx, codec, data, kind)?,
        Tag::Enum(_) => Value::String(Some(Box::new(string(cx, data)?))),
        Tag::Created(kind) => Value::String(Some(Box::new(created_text(cx, codec, data, kind)?))),
        Tag::Array(element) => array(cx, codec, data, element)?,
    };
    Ok(Tagged {
        datum,
        tag: tag.clone(),
    })
}

/// An array of `element`s: a JavaScript array whose items are each `null`, a
/// `Value` of the element kind, or data of it.
// [spec:pgorm:req:bindings.value-tags]
pub(crate) fn array<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
    element: &Tag,
) -> NeonResult<Value> {
    let items = match data.downcast::<JsArray, _>(cx) {
        Ok(array) => array.to_vec(cx)?,
        Err(_) => return refuse(cx, "expected an array"),
    };
    let element_kind = match element {
        Tag::Scalar(kind) => kind.clone(),
        Tag::Enum(_) => ArrayType::String,
        Tag::Created(_) => {
            return refuse(
                cx,
                "arrays of a created range or multirange type are not supported",
            );
        }
        Tag::Array(_) => return refuse(cx, "nested arrays are not supported"),
    };
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        values.push(tagged(cx, codec, item, element)?.datum);
    }
    Ok(Value::Array(element_kind, Some(Box::new(values))))
}

/// An integer of `T`, from a number that is a safe integer or from a bigint.
/// A number past 2^53 - 1 is refused even when integral: it may already be
/// another integer rounded to it.
fn integer<'cx, T: TryFrom<i128>>(
    cx: &mut Cx<'cx>,
    data: Handle<'cx, JsValue>,
    kind: &str,
) -> NeonResult<T> {
    let wide = if let Ok(number) = data.downcast::<JsNumber, _>(cx) {
        let number = number.value(cx);
        if number.fract() != 0.0 {
            return refuse(
                cx,
                format!("{number} is not an integer, as {kind} requires"),
            );
        }
        if number.abs() > MAX_SAFE_INTEGER {
            return refuse(
                cx,
                format!(
                    "{number} is past 2^53 - 1, where a number is no exact integer: pass a bigint"
                ),
            );
        }
        #[allow(clippy::cast_possible_truncation)]
        let integer = number as i128;
        integer
    } else if let Ok(big) = data.downcast::<JsBigInt, _>(cx) {
        match big.to_i128(cx) {
            Ok(integer) => integer,
            Err(_) => return refuse(cx, format!("the bigint is outside {kind}'s range")),
        }
    } else {
        return refuse(cx, format!("{kind} takes a number or a bigint"));
    };
    match T::try_from(wide) {
        Ok(integer) => Ok(integer),
        Err(_) => refuse(cx, format!("{wide} is outside {kind}'s range")),
    }
}

fn number<'cx>(cx: &mut Cx<'cx>, data: Handle<'cx, JsValue>, kind: &str) -> NeonResult<f64> {
    match data.downcast::<JsNumber, _>(cx) {
        Ok(number) => Ok(number.value(cx)),
        Err(_) => refuse(cx, format!("{kind} takes a number")),
    }
}

/// A number that `f32` holds exactly: narrowing it must not change it.
/// Signed zero and the infinities survive; NaN is NaN.
fn float32<'cx>(cx: &mut Cx<'cx>, data: Handle<'cx, JsValue>) -> NeonResult<f32> {
    let wide = number(cx, data, "f32")?;
    if wide.is_nan() {
        return Ok(f32::NAN);
    }
    #[allow(clippy::cast_possible_truncation)]
    let narrow = wide as f32;
    if f64::from(narrow).to_bits() != wide.to_bits() {
        return refuse(cx, format!("{wide} has no exact f32 form"));
    }
    Ok(narrow)
}

fn bytes<'cx>(cx: &mut Cx<'cx>, data: Handle<'cx, JsValue>) -> NeonResult<Vec<u8>> {
    match data.downcast::<JsUint8Array, _>(cx) {
        Ok(bytes) => Ok(bytes.as_slice(cx).to_vec()),
        Err(_) => refuse(cx, "bytes are a Uint8Array"),
    }
}

/// The described field at `index`, an integer of `T`.
fn field<'cx, T: TryFrom<i128>>(
    cx: &mut Cx<'cx>,
    description: &Description<'cx>,
    index: usize,
    name: &str,
) -> NeonResult<T> {
    let value = description.field(cx, index);
    integer(cx, value, name)
}

fn civil_date<'cx>(
    cx: &mut Cx<'cx>,
    description: &Description<'cx>,
    at: usize,
) -> NeonResult<Date> {
    let year = field::<i16>(cx, description, at, "year")?;
    let month = field::<i8>(cx, description, at + 1, "month")?;
    let day = field::<i8>(cx, description, at + 2, "day")?;
    match Date::new(year, month, day) {
        Ok(date) => Ok(date),
        Err(_) => refuse(
            cx,
            format!(
                "{year:04}-{month:02}-{day:02} is outside the years -9999 to 9999 pgorm's dates hold"
            ),
        ),
    }
}

/// A time of day from Temporal's fields. PostgreSQL keeps microseconds, so a
/// nanosecond digit is refused rather than truncated.
// [spec:pgorm:req:napi.temporal+1]
fn civil_time<'cx>(
    cx: &mut Cx<'cx>,
    description: &Description<'cx>,
    at: usize,
) -> NeonResult<Time> {
    let hour = field::<i8>(cx, description, at, "hour")?;
    let minute = field::<i8>(cx, description, at + 1, "minute")?;
    let second = field::<i8>(cx, description, at + 2, "second")?;
    let millisecond = field::<i32>(cx, description, at + 3, "millisecond")?;
    let microsecond = field::<i32>(cx, description, at + 4, "microsecond")?;
    let nanosecond = field::<i32>(cx, description, at + 5, "nanosecond")?;
    if nanosecond != 0 {
        return refuse(
            cx,
            "the time has sub-microsecond digits, which PostgreSQL does not keep: round it to \
             microseconds first",
        );
    }
    match Time::new(
        hour,
        minute,
        second,
        millisecond * 1_000_000 + microsecond * 1_000,
    ) {
        Ok(time) => Ok(time),
        Err(_) => refuse(cx, "the time of day is out of range"),
    }
}

/// A Temporal value of the temporal `kind`.
// [spec:pgorm:req:napi.temporal+1]
fn temporal<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
    kind: &ArrayType,
) -> NeonResult<Value> {
    let expected = match kind {
        ArrayType::Date => "date",
        ArrayType::Time => "time",
        ArrayType::DateTime => "datetime",
        _ => "instant",
    };
    let description = describe(cx, codec, data)?;
    let description = match description {
        Some(description) if description.tag == expected => description,
        Some(description) if description.tag == "refused" => {
            let reason = description.field(cx, 0);
            let reason = string(cx, reason)?;
            return refuse(cx, reason);
        }
        _ => {
            let class = match kind {
                ArrayType::Date => "Temporal.PlainDate",
                ArrayType::Time => "Temporal.PlainTime",
                ArrayType::DateTime => "Temporal.PlainDateTime",
                _ => "Temporal.Instant",
            };
            return refuse(cx, format!("{} takes a {class}", kind.kind_name()));
        }
    };
    temporal_value(cx, &description)
}

/// The pgorm value a described Temporal object holds.
// [spec:pgorm:req:napi.temporal+1]
fn temporal_value<'cx>(cx: &mut Cx<'cx>, description: &Description<'cx>) -> NeonResult<Value> {
    Ok(match description.tag.as_str() {
        "date" => Value::Date(Some(Box::new(civil_date(cx, description, 0)?))),
        "time" => Value::Time(Some(Box::new(civil_time(cx, description, 0)?))),
        "datetime" => {
            let date = civil_date(cx, description, 0)?;
            let time = civil_time(cx, description, 3)?;
            Value::DateTime(Some(Box::new(DateTime::from_parts(date, time))))
        }
        _ => {
            let nanoseconds = description.field(cx, 0);
            let nanoseconds = match nanoseconds.downcast::<JsBigInt, _>(cx) {
                Ok(big) => big.to_i128(cx).ok(),
                Err(_) => None,
            };
            let Some(nanoseconds) = nanoseconds else {
                return refuse(cx, "an instant's epochNanoseconds is a bigint");
            };
            if nanoseconds % 1_000 != 0 {
                return refuse(
                    cx,
                    "the instant has sub-microsecond digits, which PostgreSQL does not keep: \
                     round it to microseconds first",
                );
            }
            match Timestamp::from_nanosecond(nanoseconds) {
                Ok(instant) => Value::DateTimeWithTimeZone(Some(Box::new(instant))),
                Err(_) => {
                    return refuse(
                        cx,
                        "the instant is outside the years -9999 to 9999 pgorm's instants hold",
                    );
                }
            }
        }
    })
}

/// An interval from the module's `Interval`, or from a `Temporal.Duration`,
/// which `lib/values.js` converts to one.
// [spec:pgorm:req:napi.temporal+1]
// [spec:pgorm:req:bindings.values]
fn interval<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
) -> NeonResult<Interval> {
    match describe(cx, codec, data)? {
        Some(description) if description.tag == "interval" => interval_fields(cx, &description),
        _ => refuse(cx, "interval takes an Interval or a Temporal.Duration"),
    }
}

fn interval_fields<'cx>(cx: &mut Cx<'cx>, description: &Description<'cx>) -> NeonResult<Interval> {
    Ok(Interval::new(
        field::<i32>(cx, description, 0, "an interval's months")?,
        field::<i32>(cx, description, 1, "an interval's days")?,
        field::<i64>(cx, description, 2, "an interval's microseconds")?,
    ))
}

fn decimal<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
) -> NeonResult<Decimal> {
    let text = if data.is_a::<JsString, _>(cx) {
        string(cx, data)?
    } else if data.is_a::<JsBigInt, _>(cx) || data.is_a::<JsNumber, _>(cx) {
        integer::<i128>(cx, data, "decimal")?.to_string()
    } else {
        match describe(cx, codec, data)? {
            Some(description) if description.tag == "decimal" => {
                let text = description.field(cx, 0);
                string(cx, text)?
            }
            _ => {
                return refuse(
                    cx,
                    "decimal takes a Decimal, a decimal string or an integer",
                );
            }
        }
    };
    parse_decimal(cx, &text)
}

/// A decimal written in plain notation, held exactly: at most 28 fractional
/// digits and a 96-bit coefficient, rust_decimal's and so pgorm's range.
// [spec:pgorm:req:napi.values+1]
pub(crate) fn parse_decimal<'cx>(cx: &mut Cx<'cx>, text: &str) -> NeonResult<Decimal> {
    let body = text.strip_prefix(['-', '+']).unwrap_or(text);
    let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    if whole.is_empty()
        || !digits(whole)
        || !digits(fraction)
        || (body.contains('.') && fraction.is_empty())
    {
        return refuse(cx, format!("{text:?} is not a decimal in plain notation"));
    }
    match Decimal::from_str_exact(text) {
        Ok(decimal) => Ok(decimal),
        Err(_) => refuse(
            cx,
            format!(
                "{text} is outside a decimal's exact range: 28 fractional digits and a 96-bit coefficient"
            ),
        ),
    }
}

fn uuid<'cx>(cx: &mut Cx<'cx>, codec: Codec<'cx>, data: Handle<'cx, JsValue>) -> NeonResult<Uuid> {
    let text = if data.is_a::<JsString, _>(cx) {
        string(cx, data)?
    } else {
        match describe(cx, codec, data)? {
            Some(description) if description.tag == "uuid" => {
                let text = description.field(cx, 0);
                string(cx, text)?
            }
            _ => return refuse(cx, "uuid takes a Uuid or a UUID string"),
        }
    };
    parse_uuid(cx, &text)
}

pub(crate) fn parse_uuid<'cx>(cx: &mut Cx<'cx>, text: &str) -> NeonResult<Uuid> {
    match Uuid::try_parse(text) {
        Ok(uuid) => Ok(uuid),
        Err(_) => refuse(cx, format!("{text:?} is not a UUID")),
    }
}

/// A `Range`'s bounds as values of the scalar `kind`, each converted with that
/// kind's limits; `null` on a side is no bound.
fn range<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
    kind: &ArrayType,
) -> NeonResult<Range<Value>> {
    let description = match describe(cx, codec, data)? {
        Some(description) if description.tag == "range" => description,
        _ => return refuse(cx, "expected a Range"),
    };
    let empty = description.field(cx, 4);
    if empty.downcast_or_throw::<JsBoolean, _>(cx)?.value(cx) {
        return Ok(Range::Empty);
    }
    let side = |cx: &mut Cx<'cx>, at: usize| -> NeonResult<Bound<Value>> {
        let value = description.field(cx, at);
        if value.is_a::<JsNull, _>(cx) {
            return Ok(Bound::Unbounded);
        }
        let inclusive = description.field(cx, at + 2);
        let inclusive = inclusive.downcast_or_throw::<JsBoolean, _>(cx)?.value(cx);
        let value = scalar(cx, codec, value, kind)?;
        Ok(if inclusive {
            Bound::Included(value)
        } else {
            Bound::Excluded(value)
        })
    };
    let lower = side(cx, 0)?;
    let upper = side(cx, 1)?;
    Ok(Range::new(lower, upper))
}

fn ranges<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
    kind: &ArrayType,
    multirange: bool,
) -> NeonResult<Vec<Range<Value>>> {
    if !multirange {
        return Ok(vec![range(cx, codec, data, kind)?]);
    }
    let description = match describe(cx, codec, data)? {
        Some(description) if description.tag == "multirange" => description,
        _ => return refuse(cx, "expected a Multirange"),
    };
    let items = description
        .field(cx, 0)
        .downcast_or_throw::<JsArray, _>(cx)?
        .to_vec(cx)?;
    let mut ranges = Vec::with_capacity(items.len());
    for item in items {
        ranges.push(range(cx, codec, item, kind)?);
    }
    Ok(ranges)
}

/// A created range's text form: from a `Range` (or `Multirange`), each bound
/// converted as the subtype's kind, or from the text form itself, read with
/// the subtype's own parsing and written back canonically.
// [spec:pgorm:req:bindings.created-ranges+1]
fn created_text<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
    kind: &CreatedKind,
) -> NeonResult<String> {
    let label = if kind.multirange {
        "created_multirange"
    } else {
        "created_range"
    };
    let ranges = if data.is_a::<JsString, _>(cx) {
        let text = string(cx, data)?;
        match created::ranges(&kind.subtype, &text, kind.multirange) {
            Some(ranges) => ranges,
            None => {
                let subtype = kind.subtype.kind_name();
                return refuse(cx, format!("{text:?} is no {label} over {subtype}"));
            }
        }
    } else {
        ranges(cx, codec, data, &kind.subtype, kind.multirange)?
    };
    match created::text(&kind.subtype, ranges, kind.multirange) {
        Some(text) => Ok(text),
        None => refuse(cx, "a bound does not convert to the subtype"),
    }
}

/// A value of the scalar `kind`, read strictly: only the JavaScript types
/// that kind declares.
// [spec:pgorm:req:bindings.values]
fn scalar<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
    kind: &ArrayType,
) -> NeonResult<Value> {
    Ok(match kind {
        ArrayType::Bool => match data.downcast::<JsBoolean, _>(cx) {
            Ok(flag) => Value::Bool(Some(flag.value(cx))),
            Err(_) => return refuse(cx, "bool takes a boolean"),
        },
        ArrayType::TinyInt => Value::TinyInt(Some(integer(cx, data, "i8")?)),
        ArrayType::SmallInt => Value::SmallInt(Some(integer(cx, data, "i16")?)),
        ArrayType::Int => Value::Int(Some(integer(cx, data, "i32")?)),
        ArrayType::BigInt => Value::BigInt(Some(integer(cx, data, "i64")?)),
        ArrayType::Unsigned => Value::Unsigned(Some(integer(cx, data, "u32")?)),
        ArrayType::BigUnsigned => Value::BigUnsigned(Some(integer(cx, data, "u64")?)),
        ArrayType::Float => Value::Float(Some(float32(cx, data)?)),
        ArrayType::Double => Value::Double(Some(number(cx, data, "f64")?)),
        ArrayType::String => Value::String(Some(Box::new(string(cx, data)?))),
        ArrayType::Char => {
            let text = string(cx, data)?;
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(single), None) => Value::Char(Some(single)),
                _ => return refuse(cx, "char takes a string of exactly one code point"),
            }
        }
        ArrayType::Bytes => Value::Bytes(Some(Box::new(bytes(cx, data)?))),
        ArrayType::Json => Value::Json(Some(Box::new(json::read(cx, codec, data, 0)?))),
        ArrayType::Decimal => Value::Decimal(Some(Box::new(decimal(cx, codec, data)?))),
        ArrayType::Uuid => Value::Uuid(Some(Box::new(uuid(cx, codec, data)?))),
        ArrayType::Date
        | ArrayType::Time
        | ArrayType::DateTime
        | ArrayType::DateTimeWithTimeZone => temporal(cx, codec, data, kind)?,
        ArrayType::IpNetwork => {
            let text = string(cx, data)?;
            match text.parse::<IpNetwork>() {
                Ok(network) => Value::IpNetwork(Some(Box::new(network))),
                Err(_) => return refuse(cx, format!("{text:?} is not an IP network")),
            }
        }
        ArrayType::MacAddress => {
            let bytes = bytes(cx, data)?;
            match <[u8; 6]>::try_from(bytes.as_slice()) {
                Ok(bytes) => Value::MacAddress(Some(Box::new(MacAddress::new(bytes)))),
                Err(_) => return refuse(cx, "a MAC address is six bytes"),
            }
        }
        ArrayType::Vector => {
            let values = if let Ok(floats) = data.downcast::<JsFloat32Array, _>(cx) {
                floats.as_slice(cx).to_vec()
            } else if let Ok(array) = data.downcast::<JsArray, _>(cx) {
                let items = array.to_vec(cx)?;
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(float32(cx, item)?);
                }
                values
            } else {
                return refuse(cx, "vector takes a Float32Array or an array of numbers");
            };
            Value::Vector(Some(Box::new(Vector::from(values))))
        }
        ArrayType::Range(range_type) => {
            let element = range_type.element_type();
            Value::Range(
                *range_type,
                Some(Box::new(range(cx, codec, data, &element)?)),
            )
        }
        ArrayType::Multirange(range_type) => {
            let element = range_type.element_type();
            let ranges = ranges(cx, codec, data, &element, true)?;
            Value::Multirange(*range_type, Some(Box::new(Multirange::from(ranges))))
        }
        ArrayType::Interval => Value::from(interval(cx, codec, data)?),
        _ => return throw(cx, super::unrepresented()),
    })
}

/// What a plain JavaScript value is when no kind is declared, or `None` for
/// `null`, SQL NULL of no declared kind.
///
/// A number is an `i64` when it is a safe integer and not negative zero, so
/// it binds to an integer column exactly, and an `f64` otherwise. A bigint is
/// an `i64`; a string `text`; a `Uint8Array` `bytes`; a `Decimal`, `Uuid` or
/// `Interval` its kind; a Temporal date, time, date-time or instant its kind;
/// a plain object `json`; an array an array of the one kind its items infer
/// as. A range's subtype, an empty array's element kind and the kind of
/// `undefined` cannot be inferred and are refused.
// [spec:pgorm:req:napi.inference]
pub(crate) fn infer<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
) -> NeonResult<Option<Tagged>> {
    if data.is_a::<JsNull, _>(cx) {
        return Ok(None);
    }
    if data.is_a::<JsUndefined, _>(cx) {
        return refuse(cx, "undefined is no SQL value: pass null for NULL");
    }
    let value = if let Ok(number) = data.downcast::<JsNumber, _>(cx) {
        infer_number(number.value(cx))
    } else if let Ok(flag) = data.downcast::<JsBoolean, _>(cx) {
        Value::Bool(Some(flag.value(cx)))
    } else if let Ok(big) = data.downcast::<JsBigInt, _>(cx) {
        match big.to_i64(cx) {
            Ok(value) => Value::BigInt(Some(value)),
            Err(_) => {
                return refuse(
                    cx,
                    "the bigint is outside i64; an unsigned 64-bit value needs new Value(value, \"u64\")",
                );
            }
        }
    } else if data.is_a::<JsString, _>(cx) {
        Value::String(Some(Box::new(string(cx, data)?)))
    } else if let Ok(bytes) = data.downcast::<JsUint8Array, _>(cx) {
        Value::Bytes(Some(Box::new(bytes.as_slice(cx).to_vec())))
    } else {
        return infer_described(cx, codec, data);
    };
    Ok(Some(Tagged::value(value)))
}

/// What a plain JavaScript value that is neither a primitive nor binary data
/// is when no kind is declared: an array, or an object the codec describes.
// [spec:pgorm:req:napi.inference]
fn infer_described<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
) -> NeonResult<Option<Tagged>> {
    if data.is_a::<JsArray, _>(cx) {
        return infer_array(cx, codec, data).map(Some);
    }
    let Some(description) = describe(cx, codec, data)? else {
        return refuse(
            cx,
            "the value's kind cannot be inferred: wrap it as new Value(value, kind)",
        );
    };
    let value = match description.tag.as_str() {
        "value" => {
            let value = description.field(cx, 0);
            return boxed(cx, value).map(Some);
        }
        "decimal" => {
            let text = description.field(cx, 0);
            let text = string(cx, text)?;
            Value::Decimal(Some(Box::new(parse_decimal(cx, &text)?)))
        }
        "uuid" => {
            let text = description.field(cx, 0);
            let text = string(cx, text)?;
            Value::Uuid(Some(Box::new(parse_uuid(cx, &text)?)))
        }
        "interval" => Value::from(interval_fields(cx, &description)?),
        "date" | "time" | "datetime" | "instant" => temporal_value(cx, &description)?,
        "json" => Value::Json(Some(Box::new(json::read(cx, codec, data, 0)?))),
        "range" | "multirange" => {
            return refuse(
                cx,
                "a range's kind is not inferred from its bounds: wrap it as new Value(range, \
                 \"int4range\"), or a CreatedRange for a range type a schema created",
            );
        }
        "refused" => {
            let reason = description.field(cx, 0);
            let reason = string(cx, reason)?;
            return refuse(cx, reason);
        }
        _ => return refuse(cx, "a kind is not a value"),
    };
    Ok(Some(Tagged::value(value)))
}

/// A number with no declared kind: an `i64` when it is a safe integer other
/// than negative zero, an `f64` otherwise.
// [spec:pgorm:req:napi.inference]
pub(crate) fn infer_number(number: f64) -> Value {
    let integral = number.fract() == 0.0 && number.abs() <= MAX_SAFE_INTEGER;
    if integral && (number != 0.0 || number.is_sign_positive()) {
        #[allow(clippy::cast_possible_truncation)]
        Value::BigInt(Some(number as i64))
    } else {
        Value::Double(Some(number))
    }
}

/// An array whose kind is that of its items: every item that is not `null`
/// must infer as the same scalar kind. Numbers are `i64` items only when all of
/// them are safe integers, and `f64` items otherwise.
// [spec:pgorm:req:napi.inference]
fn infer_array<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    data: Handle<'cx, JsValue>,
) -> NeonResult<Tagged> {
    let items = data.downcast_or_throw::<JsArray, _>(cx)?.to_vec(cx)?;
    let mut numbers = 0;
    let mut values: Vec<Option<Tagged>> = Vec::with_capacity(items.len());
    for item in &items {
        if item.is_a::<JsArray, _>(cx) {
            return refuse(cx, "nested arrays are not supported");
        }
        if item.is_a::<JsNumber, _>(cx) {
            numbers += 1;
        }
        values.push(infer(cx, codec, *item)?);
    }
    let present = values.iter().flatten().count();
    if present == 0 {
        return refuse(
            cx,
            "an array with no item but null has no element kind to infer: use \
             Value.array(kind, items)",
        );
    }
    if numbers > 0 {
        if numbers != present {
            return refuse(cx, "an array mixes numbers with values of another kind");
        }
        let all_integers = values
            .iter()
            .flatten()
            .all(|value| matches!(value.datum, Value::BigInt(_)));
        let kind = if all_integers {
            ArrayType::BigInt
        } else {
            ArrayType::Double
        };
        let mut elements = Vec::with_capacity(items.len());
        for (item, value) in items.iter().zip(&values) {
            elements.push(match value {
                None => kind.null(),
                Some(value) if all_integers => value.datum.clone(),
                Some(_) => {
                    let number = item.downcast_or_throw::<JsNumber, _>(cx)?.value(cx);
                    Value::Double(Some(number))
                }
            });
        }
        return Ok(Tagged {
            datum: Value::Array(kind.clone(), Some(Box::new(elements))),
            tag: Tag::Array(Box::new(Tag::Scalar(kind))),
        });
    }
    let element = values
        .iter()
        .flatten()
        .next()
        .map(|value| value.tag.clone())
        .unwrap_or(Tag::Scalar(ArrayType::String));
    if values.iter().flatten().any(|value| value.tag != element) {
        return refuse(cx, "an array's items infer as different kinds");
    }
    if matches!(element, Tag::Array(_)) {
        return refuse(cx, "nested arrays are not supported");
    }
    if matches!(element, Tag::Created(_)) {
        return refuse(
            cx,
            "arrays of a created range or multirange type are not supported",
        );
    }
    let kind = match &element {
        Tag::Scalar(kind) => kind.clone(),
        _ => ArrayType::String,
    };
    let elements = values
        .into_iter()
        .map(|value| match value {
            Some(value) => value.datum,
            None => match &element {
                Tag::Enum(_) => Value::String(None),
                _ => kind.null(),
            },
        })
        .collect();
    let datum = Value::Array(kind, Some(Box::new(elements)));
    Ok(Tagged {
        datum,
        tag: Tag::Array(Box::new(element)),
    })
}
