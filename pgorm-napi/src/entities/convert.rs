//! A registered entity's values across the boundary: a JavaScript value into
//! the `Value` a column declares, a model's columns out as JavaScript, and
//! each job's outcome — records, a count, row versions, graph rows — as the
//! arrays `lib/entities.js` makes records of.

use crate::errors::FailureJs as _;
use neon::{prelude::*, types::Finalize};
use pgorm::pgorm_query::{ArrayType, Value};

use super::{
    adapter::{Model, Versions, Written},
    info::{ColumnInfo, EntityInfo},
};
use crate::{
    codec::Codec,
    connect::job::Settled,
    errors::{Failure, throw},
    rows,
    values::{Tag, Tagged, read, value_tag},
};

/// A model as JavaScript holds it: the real `E::Model` behind a record.
#[derive(Debug)]
pub(crate) struct ModelHandle(pub(crate) Model);

impl Finalize for ModelHandle {}

/// What `value` becomes written to `column`: a value of the column's
/// declared kind, converted exactly or refused; a column of a type the
/// binding has no kind for takes a `Value`, or a value whose kind is
/// inferred.
// [spec:pgorm:req:bindings.entity-writes+1]
pub(crate) fn column_value<'cx>(
    cx: &mut Cx<'cx>,
    column: &ColumnInfo,
    data: Handle<'cx, JsValue>,
) -> NeonResult<Value> {
    let codec = Codec::get(cx)?;
    let tagged = match &column.tag {
        Some(tag) => read::tagged(cx, codec, data, tag)?,
        None => match read::infer(cx, codec, data)? {
            Some(tagged) => tagged,
            None => {
                return read::refuse(
                    cx,
                    format!(
                        "{} is of type {}: give its NULL as a Value of its kind",
                        column.name, column.sql_type
                    ),
                );
            }
        },
    };
    Ok(tagged.datum)
}

/// `data`, a JavaScript array, as one array of `column`'s kind, or `None` for
/// a column of an array or of no kind: what a quantified comparison with the
/// column ranges over, a created range's the array of their texts that the
/// column's array cast reads.
pub(crate) fn column_array<'cx>(
    cx: &mut Cx<'cx>,
    column: &ColumnInfo,
    data: Handle<'cx, JsValue>,
) -> NeonResult<Option<Value>> {
    let Some(tag) = column
        .tag
        .as_ref()
        .filter(|tag| !matches!(tag, Tag::Array(_)))
    else {
        return Ok(None);
    };
    let codec = Codec::get(cx)?;
    let tagged = read::tagged(cx, codec, data, &Tag::Array(Box::new(tag.clone())))?;
    Ok(Some(tagged.datum))
}

/// A column's value with the kind it reads as: the column's own where it
/// names a type — an enum's, a created range's — that the value, a string,
/// does not carry, and otherwise the value's.
pub(crate) fn tagged(column: &ColumnInfo, value: Value) -> Result<Tagged, Failure> {
    let named = match (&column.tag, &value) {
        (Some(tag @ (Tag::Enum(_) | Tag::Created(_))), Value::String(_)) => Some(tag.clone()),
        (Some(tag @ Tag::Array(element)), Value::Array(ArrayType::String, _))
            if matches!(**element, Tag::Enum(_) | Tag::Created(_)) =>
        {
            Some(tag.clone())
        }
        _ => None,
    };
    let tag = match named {
        Some(tag) => tag,
        None => value_tag(&value),
    };
    Ok(Tagged { tag, datum: value })
}

/// A model's columns as plain JavaScript values, in column order.
pub(crate) fn values<'cx>(cx: &mut Cx<'cx>, model: &Model) -> JsResult<'cx, JsArray> {
    let info = model.info().clone();
    let codec = Codec::get(cx)?;
    let array = JsArray::new(cx, info.columns.len());
    for (index, column) in info.columns.iter().enumerate() {
        let value = match model.get(&column.name) {
            Ok(value) => value,
            Err(failure) => {
                let error = failure.into_js(cx)?;
                return cx.throw(error);
            }
        };
        let value = tagged(column, value).or_else(|failure| throw(cx, failure))?;
        let value = rows::value(cx, codec, value, false)?;
        array.set(cx, u32::try_from(index).unwrap_or(u32::MAX), value)?;
    }
    Ok(array)
}

/// `[values, model]`: a record's values and the model behind it.
pub(crate) fn record<'cx>(cx: &mut Cx<'cx>, model: Model) -> JsResult<'cx, JsValue> {
    let values = values(cx, &model)?;
    let handle = cx.boxed(ModelHandle(model));
    let pair = JsArray::new(cx, 2);
    pair.set(cx, 0, values)?;
    pair.set(cx, 1, handle)?;
    Ok(pair.upcast())
}

/// An entity's column names, in column order.
pub(crate) fn names<'cx>(cx: &mut Cx<'cx>, info: &EntityInfo) -> JsResult<'cx, JsArray> {
    let array = JsArray::new(cx, info.columns.len());
    for (index, column) in info.columns.iter().enumerate() {
        let name = cx.string(&column.name);
        array.set(cx, u32::try_from(index).unwrap_or(u32::MAX), name)?;
    }
    Ok(array)
}

pub(crate) fn list<'cx, T>(
    cx: &mut Cx<'cx>,
    items: Vec<T>,
    mut each: impl FnMut(&mut Cx<'cx>, T) -> JsResult<'cx, JsValue>,
) -> JsResult<'cx, JsArray> {
    let array = JsArray::new(cx, items.len());
    for (index, item) in items.into_iter().enumerate() {
        let item = each(cx, item)?;
        array.set(cx, u32::try_from(index).unwrap_or(u32::MAX), item)?;
    }
    Ok(array)
}

/// The records a read or a write gave: `[columns, [values, model][]]`.
pub(crate) struct Records(pub(crate) std::sync::Arc<EntityInfo>, pub(crate) Vec<Model>);

impl Settled for Records {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        let Self(info, models) = *self;
        let columns = names(cx, &info)?;
        let rows = list(cx, models, record)?;
        let pair = JsArray::new(cx, 2);
        pair.set(cx, 0, columns)?;
        pair.set(cx, 1, rows)?;
        Ok(pair.upcast())
    }
}

/// A count of rows as a number, refused past the integers one holds exactly.
pub(crate) fn count<'cx>(cx: &mut Cx<'cx>, count: u64) -> JsResult<'cx, JsValue> {
    if count > (1 << 53) - 1 {
        let error = Failure::Decode(format!(
            "{count} rows is past the integers a number holds exactly"
        ))
        .into_js(cx)?;
        return cx.throw(error);
    }
    #[allow(clippy::cast_precision_loss)]
    Ok(cx.number(count as f64).upcast())
}

/// What an ActiveModel's write gave: its record, or a deletion's count.
pub(crate) struct Wrote(pub(crate) Written);

impl Settled for Wrote {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        match self.0 {
            Written::Model(model) => {
                let info = model.info().clone();
                Box::new(Records(info, vec![model])).into_js(cx)
            }
            Written::Count(written) => count(cx, written),
        }
    }
}

/// Written rows' versions: `[columns, [old | null, new][]]`, each version
/// `[values, model]`.
pub(crate) struct VersionRows(
    pub(crate) std::sync::Arc<EntityInfo>,
    pub(crate) Vec<Versions>,
);

impl Settled for VersionRows {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        let Self(info, versions) = *self;
        let columns = names(cx, &info)?;
        let rows = list(cx, versions, |cx, versions| {
            let old = match versions.old {
                Some(old) => record(cx, old)?,
                None => cx.null().upcast(),
            };
            let new = record(cx, versions.new)?;
            let pair = JsArray::new(cx, 2);
            pair.set(cx, 0, old)?;
            pair.set(cx, 1, new)?;
            Ok(pair.upcast())
        })?;
        let pair = JsArray::new(cx, 2);
        pair.set(cx, 0, columns)?;
        pair.set(cx, 1, rows)?;
        Ok(pair.upcast())
    }
}

/// Rows of several sources — a graph's, or a pipeline's selected sources —
/// as `[columns per source, ([values, model] | null)[][]]`.
pub(crate) struct GraphRows(
    pub(crate) Vec<std::sync::Arc<EntityInfo>>,
    pub(crate) Vec<Vec<Option<Model>>>,
);

impl Settled for GraphRows {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        let Self(sources, rows) = *self;
        let columns = list(cx, sources, |cx, info| Ok(names(cx, &info)?.upcast()))?;
        let rows = list(cx, rows, |cx, row| {
            Ok(list(cx, row, |cx, model| match model {
                Some(model) => record(cx, model),
                None => Ok(cx.null().upcast()),
            })?
            .upcast())
        })?;
        let pair = JsArray::new(cx, 2);
        pair.set(cx, 0, columns)?;
        pair.set(cx, 1, rows)?;
        Ok(pair.upcast())
    }
}
