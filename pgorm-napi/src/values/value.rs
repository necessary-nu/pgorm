//! The native half of the module's `Value` class and of its `Decimal` and
//! `Uuid` validation: synchronous, because none of it does I/O.

use neon::prelude::*;

use super::{
    Tag, Tagged,
    read::{self, parse_decimal, parse_uuid, refuse, string},
    write,
};
use crate::codec::Codec;

pub(crate) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("valueNew", value_new)?;
    cx.export_function("valueNull", value_null)?;
    cx.export_function("valueJson", value_json)?;
    cx.export_function("valueArray", value_array)?;
    cx.export_function("valueKind", value_kind)?;
    cx.export_function("valueIsNull", value_is_null)?;
    cx.export_function("valueTypeName", value_type_name)?;
    cx.export_function("valueCreatedType", value_created_type)?;
    cx.export_function("valueElementType", value_element_type)?;
    cx.export_function("valueGet", value_get)?;
    cx.export_function("valueItems", value_items)?;
    cx.export_function("valueEquals", value_equals)?;
    cx.export_function("decimalText", decimal_text)?;
    cx.export_function("uuidText", uuid_text)?;
    Ok(())
}

fn tagged<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<Handle<'cx, JsBox<Tagged>>> {
    cx.argument::<JsBox<Tagged>>(index)
}

/// `valueNew(data, kind?)`: a value of the declared kind, or of the kind
/// `data` infers as.
// [spec:pgorm:req:bindings.value-tags]
fn value_new(mut cx: FunctionContext) -> JsResult<JsBox<Tagged>> {
    let data = cx.argument::<JsValue>(0)?;
    let kind = cx.argument_opt(1);
    let codec = Codec::get(&mut cx)?;
    let value = match kind {
        Some(kind) if !kind.is_a::<JsUndefined, _>(&mut cx) => {
            let tag = read::kind(&mut cx, codec, kind)?;
            read::tagged(&mut cx, codec, data, &tag)?
        }
        _ => match read::infer(&mut cx, codec, data)? {
            Some(value) => value,
            None => {
                return refuse(
                    &mut cx,
                    "null has no kind to infer: use Value.null(kind) for a typed NULL",
                );
            }
        },
    };
    Ok(cx.boxed(value))
}

/// `valueNull(kind)`: SQL NULL of a kind.
fn value_null(mut cx: FunctionContext) -> JsResult<JsBox<Tagged>> {
    let kind = cx.argument::<JsValue>(0)?;
    let codec = Codec::get(&mut cx)?;
    let tag = read::kind(&mut cx, codec, kind)?;
    Ok(cx.boxed(Tagged::null(tag)))
}

/// `valueJson(data)`: a JSON document, `null` being JSON's null rather than
/// SQL NULL.
// [spec:pgorm:req:bindings.value-tags]
fn value_json(mut cx: FunctionContext) -> JsResult<JsBox<Tagged>> {
    let data = cx.argument::<JsValue>(0)?;
    let codec = Codec::get(&mut cx)?;
    let json = super::json::read(&mut cx, codec, data, 0)?;
    let value = Tagged::value(pgorm::pgorm_query::Value::Json(Some(Box::new(json))));
    Ok(cx.boxed(value))
}

/// `valueArray(kind, items)`: an array of the element kind, `null` items SQL
/// NULLs; `items` `null` is an SQL NULL array that still names its element
/// kind.
// [spec:pgorm:req:bindings.value-tags]
fn value_array(mut cx: FunctionContext) -> JsResult<JsBox<Tagged>> {
    let kind = cx.argument::<JsValue>(0)?;
    let items = cx.argument::<JsValue>(1)?;
    let codec = Codec::get(&mut cx)?;
    let element = read::kind(&mut cx, codec, kind)?;
    let tag = Tag::Array(Box::new(element.clone()));
    if items.is_a::<JsNull, _>(&mut cx) {
        return Ok(cx.boxed(Tagged::null(tag)));
    }
    let datum = read::array(&mut cx, codec, items, &element)?;
    Ok(cx.boxed(Tagged { datum, tag }))
}

fn value_kind(mut cx: FunctionContext) -> JsResult<JsString> {
    let value = tagged(&mut cx, 0)?;
    Ok(cx.string(value.tag.name()))
}

fn value_is_null(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let value = tagged(&mut cx, 0)?;
    Ok(cx.boolean(value.is_null()))
}

fn type_name<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    name: &super::TypeName,
) -> JsResult<'cx, JsValue> {
    let schema: Handle<JsValue> = match &name.schema {
        Some(schema) => cx.string(schema).upcast(),
        None => cx.null().upcast(),
    };
    codec.make(cx, ("typename", name.name.clone(), schema))
}

fn kind_object<'cx>(cx: &mut Cx<'cx>, codec: Codec<'cx>, tag: &Tag) -> JsResult<'cx, JsValue> {
    match tag {
        Tag::Scalar(kind) => Ok(cx.string(kind.kind_name()).upcast()),
        Tag::Enum(name) => type_name(cx, codec, name),
        Tag::Created(kind) => {
            let schema: Handle<JsValue> = match &kind.name.schema {
                Some(schema) => cx.string(schema).upcast(),
                None => cx.null().upcast(),
            };
            let subtype = kind.subtype.kind_name();
            codec.make(
                cx,
                (
                    "created",
                    kind.name.name.clone(),
                    subtype,
                    schema,
                    kind.multirange,
                ),
            )
        }
        Tag::Array(_) => Ok(cx.string("array").upcast()),
    }
}

/// An enum's type name, or a created range's.
fn value_type_name(mut cx: FunctionContext) -> JsResult<JsValue> {
    let value = tagged(&mut cx, 0)?;
    let codec = Codec::get(&mut cx)?;
    let name = match &value.tag {
        Tag::Enum(name) => name.clone(),
        Tag::Created(kind) => kind.name.clone(),
        Tag::Array(element) => match element.as_ref() {
            Tag::Enum(name) => name.clone(),
            _ => return Ok(cx.null().upcast()),
        },
        Tag::Scalar(_) => return Ok(cx.null().upcast()),
    };
    type_name(&mut cx, codec, &name)
}

fn value_created_type(mut cx: FunctionContext) -> JsResult<JsValue> {
    let value = tagged(&mut cx, 0)?;
    let codec = Codec::get(&mut cx)?;
    match &value.tag {
        Tag::Created(_) => {
            let tag = value.tag.clone();
            kind_object(&mut cx, codec, &tag)
        }
        _ => Ok(cx.null().upcast()),
    }
}

fn value_element_type(mut cx: FunctionContext) -> JsResult<JsValue> {
    let value = tagged(&mut cx, 0)?;
    let codec = Codec::get(&mut cx)?;
    match &value.tag {
        Tag::Array(element) => {
            let element = element.as_ref().clone();
            kind_object(&mut cx, codec, &element)
        }
        _ => Ok(cx.null().upcast()),
    }
}

/// `valueGet(value)`: the plain JavaScript value, independently owned.
fn value_get(mut cx: FunctionContext) -> JsResult<JsValue> {
    let value = tagged(&mut cx, 0)?;
    let codec = Codec::get(&mut cx)?;
    let value = (*value).clone();
    write::plain(&mut cx, codec, &value)
}

/// `valueItems(value)`: an array's items as values of its element kind, or
/// `null` for an SQL NULL array.
fn value_items(mut cx: FunctionContext) -> JsResult<JsValue> {
    let value = tagged(&mut cx, 0)?;
    let Tag::Array(element) = &value.tag else {
        return cx.throw_type_error("items() needs an array value");
    };
    let element = element.as_ref().clone();
    let items: Option<Vec<Tagged>> = match &value.datum {
        pgorm::pgorm_query::Value::Array(_, items) => items.as_ref().map(|items| {
            items
                .iter()
                .map(|item| Tagged {
                    datum: item.clone(),
                    tag: element.clone(),
                })
                .collect()
        }),
        _ => return cx.throw_type_error("items() needs an array value"),
    };
    let Some(items) = items else {
        return Ok(cx.null().upcast());
    };
    let array = JsArray::new(&mut cx, items.len());
    for (index, item) in items.into_iter().enumerate() {
        let item = cx.boxed(item);
        array.set(&mut cx, u32::try_from(index).unwrap_or(u32::MAX), item)?;
    }
    Ok(array.upcast())
}

/// `valueEquals(a, b)`: the same kind and the same value, floats compared by
/// their bits as pgorm compares them.
fn value_equals(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let left = tagged(&mut cx, 0)?;
    let right = tagged(&mut cx, 1)?;
    let equal = **left == **right;
    Ok(cx.boolean(equal))
}

/// `decimalText(text)`: the canonical text of an exact decimal, or a
/// `ConstructionError`.
fn decimal_text(mut cx: FunctionContext) -> JsResult<JsString> {
    let text = cx.argument::<JsValue>(0)?;
    let text = string(&mut cx, text)?;
    let decimal = parse_decimal(&mut cx, &text)?;
    Ok(cx.string(decimal.to_string()))
}

/// `uuidText(text)`: a UUID's canonical lower-case hyphenated text, or a
/// `ConstructionError`.
fn uuid_text(mut cx: FunctionContext) -> JsResult<JsString> {
    let text = cx.argument::<JsValue>(0)?;
    let text = string(&mut cx, text)?;
    let uuid = parse_uuid(&mut cx, &text)?;
    Ok(cx.string(uuid.hyphenated().to_string()))
}
