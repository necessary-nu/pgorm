//! Decoded rows handed to JavaScript, where `lib/operations.js` makes each an
//! object keyed by column name.

use neon::prelude::*;
use pgorm_driver::Row;

use crate::{
    codec::Codec,
    decode,
    errors::Failure,
    values::{Tagged, write},
};

/// A result's column names and its rows' values, decoded on the runtime
/// thread.
#[derive(Debug, Default)]
pub(crate) struct Decoded {
    pub(crate) names: Vec<String>,
    pub(crate) rows: Vec<Vec<Tagged>>,
}

// [spec:pgorm:req:bindings.rows]
pub(crate) fn decode(rows: &[Row]) -> Result<Decoded, Failure> {
    let Some(first) = rows.first() else {
        return Ok(Decoded::default());
    };
    Ok(Decoded {
        names: decode::names(first)?,
        rows: rows.iter().map(decode::row).collect::<Result<_, _>>()?,
    })
}

fn index(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// One value as JavaScript: plain, or the module's `Value` when the caller
/// asked for tagged rows.
pub(crate) fn value<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    value: Tagged,
    tagged: bool,
) -> JsResult<'cx, JsValue> {
    if tagged {
        let boxed = cx.boxed(value);
        codec.make(cx, ("value", boxed))
    } else {
        write::plain(cx, codec, &value)
    }
}

pub(crate) fn names<'cx>(cx: &mut Cx<'cx>, names: &[String]) -> JsResult<'cx, JsArray> {
    let list = JsArray::new(cx, names.len());
    for (at, name) in names.iter().enumerate() {
        let name = cx.string(name);
        list.set(cx, index(at), name)?;
    }
    Ok(list)
}

/// One row's values as an array, in column order.
pub(crate) fn values<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    row: Vec<Tagged>,
    tagged: bool,
) -> JsResult<'cx, JsArray> {
    let values = JsArray::new(cx, row.len());
    for (column, item) in row.into_iter().enumerate() {
        let item = value(cx, codec, item, tagged)?;
        values.set(cx, index(column), item)?;
    }
    Ok(values)
}

/// Each column's kind as `Tag::key` spells it, read off the first
/// row: every row of a column holds its type's kind, NULLs included.
pub(crate) fn kinds<'cx>(cx: &mut Cx<'cx>, row: Option<&[Tagged]>) -> JsResult<'cx, JsArray> {
    let row = row.unwrap_or_default();
    let kinds = JsArray::new(cx, row.len());
    for (at, value) in row.iter().enumerate() {
        let kind = value.tag.key();
        let kind = cx.string(kind);
        kinds.set(cx, index(at), kind)?;
    }
    Ok(kinds)
}

/// `[names, rows, kinds]`, each row an array of its values in column order,
/// and each column's kind for a model to hold to its declaration.
// [spec:pgorm:req:bindings.rows]
// [spec:pgorm:req:napi.records]
// [spec:pgorm:req:bindings.model-records+2]
pub(crate) fn to_js<'cx>(
    cx: &mut Cx<'cx>,
    decoded: Decoded,
    tagged: bool,
) -> JsResult<'cx, JsValue> {
    let codec = Codec::get(cx)?;
    let names = names(cx, &decoded.names)?;
    let kinds = kinds(cx, decoded.rows.first().map(Vec::as_slice))?;
    let rows = JsArray::new(cx, decoded.rows.len());
    for (at, row) in decoded.rows.into_iter().enumerate() {
        let values = values(cx, codec, row, tagged)?;
        rows.set(cx, index(at), values)?;
    }
    let result = JsArray::new(cx, 3);
    result.set(cx, 0, names)?;
    result.set(cx, 1, rows)?;
    result.set(cx, 2, kinds)?;
    Ok(result.upcast())
}
