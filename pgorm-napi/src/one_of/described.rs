//! A one-of reference's shape, described from the models JavaScript
//! declares.

use std::sync::Arc;

use neon::prelude::*;
use pgorm::DisjointTable;
use pgorm_bindings::entity::{DescribedTarget, OneOfCore};

use super::OneOfHandle;
use crate::{disjoint::table_of, errors::throw};

fn text(cx: &mut FunctionContext, value: Handle<JsValue>, what: &str) -> NeonResult<String> {
    match value.downcast::<JsString, _>(cx) {
        Ok(text) => Ok(text.value(cx)),
        Err(_) => cx.throw_type_error(format!("a described one-of reference's {what} is a string")),
    }
}

/// A model's name beside its described table, `[name, table]`.
fn described<'cx>(
    cx: &mut FunctionContext<'cx>,
    pair: Handle<'cx, JsValue>,
) -> NeonResult<(String, DisjointTable)> {
    let Ok(pair) = pair.downcast::<JsArray, _>(cx) else {
        return cx.throw_type_error("a described model is [name, table]");
    };
    let name: Handle<JsValue> = pair.get(cx, 0)?;
    let name = text(cx, name, "model name")?;
    let table: Handle<JsValue> = pair.get(cx, 1)?;
    Ok((name, table_of(cx, table)?))
}

/// `oneOfModel(name, model, targets, optional)`: the reference `name` from
/// the described `model` to each target in order, `[value, field, column,
/// [model, table]]`. What the shape refuses is a `ConstructionError`,
/// thrown as the reference is declared.
// [spec:pgorm:req:bindings.one-of+1]
pub(crate) fn one_of_model(mut cx: FunctionContext) -> JsResult<JsValue> {
    let name = cx.argument::<JsString>(0)?.value(&mut cx);
    let model = cx.argument::<JsValue>(1)?;
    let (model_name, entity) = described(&mut cx, model)?;
    let targets = cx.argument::<JsArray>(2)?.to_vec(&mut cx)?;
    let optional = cx.argument::<JsBoolean>(3)?.value(&mut cx);
    let mut described_targets = Vec::with_capacity(targets.len());
    for target in targets {
        let Ok(target) = target.downcast::<JsArray, _>(&mut cx) else {
            return cx.throw_type_error("a target is [value, field, column, [model, table]]");
        };
        let value: Handle<JsValue> = target.get(&mut cx, 0)?;
        let value = text(&mut cx, value, "target value")?;
        let field: Handle<JsValue> = target.get(&mut cx, 1)?;
        let field = text(&mut cx, field, "field")?;
        let column: Handle<JsValue> = target.get(&mut cx, 2)?;
        let column = text(&mut cx, column, "column")?;
        let pair: Handle<JsValue> = target.get(&mut cx, 3)?;
        let (model, table) = described(&mut cx, pair)?;
        described_targets.push(DescribedTarget {
            value,
            field,
            column,
            model,
            table,
        });
    }
    match OneOfCore::model(&name, (model_name, entity), described_targets, optional) {
        Ok(core) => Ok(cx.boxed(OneOfHandle(Arc::new(core))).upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}
