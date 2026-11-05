//! The native half of the models JavaScript declares.
//!
//! A model is JavaScript data lowered into the statement builders, as
//! pgorm-python's are Python data over its native builders; what it needs
//! from Rust is small and lives here: one spelling of a value's kind, which
//! a declared column and a decoded result column are compared by; the value
//! a created multirange's text, the form a model projects one in, spells;
//! the bounded composition of a prefixed result-column name pgorm's own
//! graph writer mints, so a model graph's aliases are the ones pgorm writes;
//! and the statement pgorm's paginator counts a query's rows with.

#[cfg(test)]
mod parity;

use neon::prelude::*;
use pgorm::pgorm_query::{Asterisk, Expr, Func, Name, Query, SelectStatement, Value};

use crate::{
    codec::Codec,
    errors::{Failure, throw},
    statements::{self, Node},
    values::{Tag, Tagged, TypeName, read, write},
};

pub(crate) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("modelKindKey", kind_key_of_kind)?;
    cx.export_function("modelValueKey", kind_key_of_value)?;
    cx.export_function("modelReadText", read_text)?;
    cx.export_function("modelResultName", result_name)?;
    cx.export_function("modelCount", count)?;
    Ok(())
}

/// A kind as one string, the same for a declared column and a decoded one:
/// a scalar kind's name (`i32`, `tstzrange`), an enum's type as SQL quotes
/// it, a created range's type and subtype, and an array's element followed
/// by `[]`.
// [spec:pgorm:req:bindings.model-records+1]
pub(crate) fn kind_key(tag: &Tag) -> Result<String, Failure> {
    Ok(match tag {
        Tag::Scalar(kind) => kind.kind_name().to_owned(),
        Tag::Enum(name) => format!("enum {}", quoted(name)),
        Tag::Created(kind) => format!(
            "{} {} of {}",
            if kind.multirange {
                "multirange"
            } else {
                "range"
            },
            quoted(&kind.name),
            kind.subtype.kind_name()
        ),
        Tag::Array(element) => format!("{}[]", kind_key(element)?),
    })
}

fn quoted(name: &TypeName) -> String {
    let part = |part: &str| format!("\"{}\"", part.replace('"', "\"\""));
    match &name.schema {
        Some(schema) => format!("{}.{}", part(schema), part(&name.name)),
        None => part(&name.name),
    }
}

/// `modelKindKey(kind, array)`: the key of a declared column's kind, an
/// array of it when `array` is true.
fn kind_key_of_kind(mut cx: FunctionContext) -> JsResult<JsString> {
    let kind = cx.argument::<JsValue>(0)?;
    let array = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    let codec = Codec::get(&mut cx)?;
    let tag = read::kind(&mut cx, codec, kind)?;
    let tag = if array {
        if matches!(tag, Tag::Created(_)) {
            return read::refuse(&mut cx, "arrays of a created range type are not supported");
        }
        Tag::Array(Box::new(tag))
    } else {
        tag
    };
    let key = kind_key(&tag).or_else(|failure| throw(&mut cx, failure))?;
    Ok(cx.string(key))
}

/// `modelValueKey(value)`: the key of the kind a `Value` declares.
fn kind_key_of_value(mut cx: FunctionContext) -> JsResult<JsString> {
    let value = cx.argument::<JsValue>(0)?;
    let codec = Codec::get(&mut cx)?;
    match read::infer(&mut cx, codec, value)? {
        Some(tagged) => {
            let key = kind_key(&tagged.tag).or_else(|failure| throw(&mut cx, failure))?;
            Ok(cx.string(key))
        }
        None => read::refuse(&mut cx, "null declares no kind"),
    }
}

/// `modelReadText(kind, text)`: the value `text` spells as a created
/// multirange `kind`. The driver reports a created multirange with no
/// subtype to read its binary form by, so a model projects such a column
/// cast to `text`, as pgorm's `ColumnTrait::select_as` reads an entity's,
/// and its record reads the text through the declared subtype, as a decoded
/// created range's is read: text the subtype does not read is a
/// `DecodeError`, never a value of another type.
// [spec:pgorm:req:bindings.model-records+1]
// [spec:pgorm:req:bindings.created-ranges+2]
fn read_text(mut cx: FunctionContext) -> JsResult<JsValue> {
    let kind = cx.argument::<JsValue>(0)?;
    let text = cx.argument::<JsString>(1)?.value(&mut cx);
    let codec = Codec::get(&mut cx)?;
    let tag = read::kind(&mut cx, codec, kind)?;
    if !matches!(&tag, Tag::Created(kind) if kind.multirange) {
        return cx.throw_type_error("only a created multirange is read from its text");
    }
    let tagged = Tagged {
        datum: Value::String(Some(Box::new(text))),
        tag,
    };
    write::plain(&mut cx, codec, &tagged)
}

/// `modelResultName(prefix, column)`: the result-set name of `column` under
/// `prefix`, composed by pgorm's own `result_column_name`, so a model graph's
/// aliases are the ones `SelectGraph` writes.
// [spec:pgorm:req:bindings.graphs]
fn result_name(mut cx: FunctionContext) -> JsResult<JsString> {
    let prefix = cx.argument::<JsString>(0)?.value(&mut cx);
    let column = cx.argument::<JsString>(1)?.value(&mut cx);
    Ok(cx.string(pgorm::result_column_name(&prefix, &column)))
}

/// The statement counting the rows `query` reads, as pgorm's paginator
/// counts them: its limit, offset and ordering dropped, and the rest read as
/// a subquery, `SELECT COUNT(*) AS "num_items" FROM (..) AS "sub_query"`.
// [spec:pgorm:req:bindings.pagination]
pub(crate) fn counted(mut query: SelectStatement) -> SelectStatement {
    query.reset_limit().reset_offset().clear_order_by();
    Query::select()
        .expr_as(Func::count(Expr::col(Asterisk)), Name::runtime("num_items"))
        .from_subquery(query, Name::runtime("sub_query"))
        .to_owned()
}

/// `modelCount(select)`: the counting statement of a `Select`.
fn count(mut cx: FunctionContext) -> JsResult<JsBox<Node>> {
    let value = cx.argument::<JsValue>(0)?;
    match statements::node(&mut cx, value) {
        Some(Node::Select(select)) => Ok(cx.boxed(Node::Select(counted(select)))),
        _ => cx.throw_type_error("only a Select's rows are counted"),
    }
}
