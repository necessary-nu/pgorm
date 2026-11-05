//! One-of references over pgorm's [`OneOfShape`]: a reference declared over
//! models JavaScript describes, or a downstream crate's registered
//! `#[derive(OneOf)]` enum.
//!
//! Either is one native handle, the shape and its description. Every
//! decision about a reference is the shape's — which target a row
//! references, which columns a write sets, the condition a target's rows
//! are loaded by, the error for a key its target does not hold, and the
//! `CHECK` — so a model's reference is the Rust declaration's;
//! `lib/one-of.js` only reads the records' fields and runs the reads.

mod described;
mod registered;

use std::sync::Arc;

use neon::{prelude::*, types::Finalize};
use pgorm::{Error, OneOfShape, pgorm_query::Value};

use crate::{
    codec::Codec,
    errors::{Failure, throw},
    statements::{Ddl, Node},
    values::read,
};

pub(crate) use registered::Registered;

/// One reference or registration: its shape and its description.
pub(crate) struct Core {
    pub(crate) shape: OneOfShape,
    pub(crate) description: serde_json::Value,
    /// The registered enum's type, which no second registration may name.
    pub(crate) rust: Option<std::any::TypeId>,
}

/// A one-of reference or registration as JavaScript holds it.
pub(crate) struct OneOfHandle(pub(crate) Arc<Core>);

impl std::fmt::Debug for OneOfHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("OneOfHandle")
            .field(&self.0.description)
            .finish()
    }
}

impl Finalize for OneOfHandle {}

pub(crate) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    for (name, export) in EXPORTS {
        cx.export_function(name, *export)?;
    }
    Ok(())
}

type Export = fn(FunctionContext) -> JsResult<JsValue>;

const EXPORTS: &[(&str, Export)] = &[
    ("oneOfModel", described::one_of_model),
    ("oneOfEntity", registered::one_of_entity),
    ("oneOfEntities", registered::one_of_entities),
    ("oneOfDescribe", describe),
    ("oneOfTarget", target),
    ("oneOfReferenced", referenced),
    ("oneOfAssigned", assigned),
    ("oneOfLoadCondition", load_condition),
    ("oneOfAbsent", absent),
    ("oneOfStatement", statement),
];

fn core(cx: &mut FunctionContext) -> NeonResult<Arc<Core>> {
    Ok(cx.argument::<JsBox<OneOfHandle>>(0)?.0.clone())
}

/// What the shape refuses as a reference is used: a row against its
/// declaration, read back, is a `DecodeError`, and anything else, refused
/// before anything is sent, a `ConstructionError`.
fn refused(error: Error) -> Failure {
    match error {
        Error::Query(_) => Failure::Decode(error.to_string()),
        error => Failure::Construction(error.to_string()),
    }
}

/// The `Value`s the array argument `index` holds.
fn values_at(cx: &mut FunctionContext, index: usize) -> NeonResult<Vec<Value>> {
    let given = cx.argument::<JsArray>(index)?.to_vec(cx)?;
    let codec = Codec::get(cx)?;
    let mut values = Vec::with_capacity(given.len());
    for value in given {
        match read::infer(cx, codec, value)? {
            Some(value) => values.push(value.datum),
            None => return cx.throw_type_error("a one-of reference's keys are Values"),
        }
    }
    Ok(values)
}

/// A target's place, or null.
fn place(cx: &mut FunctionContext, index: usize) -> NeonResult<Option<usize>> {
    let value = cx.argument::<JsValue>(index)?;
    if value.is_a::<JsNull, _>(cx) {
        return Ok(None);
    }
    let number = value.downcast_or_throw::<JsNumber, _>(cx)?.value(cx);
    Ok(Some(number as usize))
}

/// `oneOfDescribe(oneOf)`: the description, as JSON text.
// [spec:pgorm:req:bindings.one-of+1]
fn describe(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = core(&mut cx)?;
    Ok(cx.string(core.description.to_string()).upcast())
}

/// `oneOfTarget(oneOf, value)`: where the target `value` stands.
// [spec:pgorm:req:bindings.one-of+1]
fn target(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = core(&mut cx)?;
    let value = cx.argument::<JsString>(1)?.value(&mut cx);
    match core.shape.target(&value) {
        Ok((index, _)) => Ok(cx.number(index as f64).upcast()),
        Err(error) => throw(&mut cx, refused(error)),
    }
}

/// `oneOfReferenced(oneOf, rowKey, set)`: the target a row references, or
/// null for a row referencing none.
// [spec:pgorm:req:bindings.one-of+1]
fn referenced(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = core(&mut cx)?;
    let row = values_at(&mut cx, 1)?;
    let set = cx.argument::<JsArray>(2)?.to_vec(&mut cx)?;
    let set = set
        .into_iter()
        .map(|flag| {
            Ok(flag
                .downcast_or_throw::<JsBoolean, _>(&mut cx)?
                .value(&mut cx))
        })
        .collect::<NeonResult<Vec<bool>>>()?;
    match core.shape.referenced(&row, &set) {
        Ok(Some(index)) => Ok(cx.number(index as f64).upcast()),
        Ok(None) => Ok(cx.null().upcast()),
        Err(error) => throw(&mut cx, refused(error)),
    }
}

/// `oneOfAssigned(oneOf, target)`: for each target, whether a write of a
/// reference to `target` sets its column to the key, the rest null.
// [spec:pgorm:req:bindings.one-of+1]
fn assigned(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = core(&mut cx)?;
    let target = place(&mut cx, 1)?;
    let assigned = match core.shape.assigned(target) {
        Ok(assigned) => assigned,
        Err(error) => return throw(&mut cx, refused(error)),
    };
    let array = JsArray::new(&mut cx, assigned.len());
    for (index, set) in assigned.into_iter().enumerate() {
        let set = cx.boolean(set);
        array.set(&mut cx, u32::try_from(index).unwrap_or(u32::MAX), set)?;
    }
    Ok(array.upcast())
}

/// `oneOfLoadCondition(oneOf, target, keys)`: the condition the rows of the
/// target at `target` are loaded by.
// [spec:pgorm:req:bindings.one-of+1]
fn load_condition(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = core(&mut cx)?;
    let target = place(&mut cx, 1)?.unwrap_or_default();
    let keys = values_at(&mut cx, 2)?;
    match core.shape.load_condition(target, keys) {
        Ok(condition) => Ok(cx.boxed(Node::Expr(condition)).upcast()),
        Err(error) => throw(&mut cx, refused(error)),
    }
}

/// `oneOfAbsent(oneOf, target, rowKey, key)`: the message of the
/// `DecodeError` for a key the target's table does not hold.
// [spec:pgorm:req:bindings.one-of+1]
fn absent(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = core(&mut cx)?;
    let target = place(&mut cx, 1)?.unwrap_or_default();
    let row = values_at(&mut cx, 2)?;
    let codec = Codec::get(&mut cx)?;
    let key = cx.argument::<JsValue>(3)?;
    let Some(key) = read::infer(&mut cx, codec, key)? else {
        return cx.throw_type_error("a one-of reference's keys are Values");
    };
    let error = core.shape.absent(target, &row, &key.datum);
    Ok(cx.string(error.to_string()).upcast())
}

/// `oneOfStatement(oneOf)`: the `ALTER TABLE` adding the reference's
/// `CHECK`.
// [spec:pgorm:req:bindings.one-of+1]
fn statement(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = core(&mut cx)?;
    let check = core.shape.check();
    let table = check.get_table_name().clone();
    Ok(cx.boxed(Node::from(Ddl::AlterTable(table, check))).upcast())
}
