//! A downstream crate's `#[derive(Disjoint)]` enum, registered with its base
//! and kind entities in pgorm-bindings' registry, as JavaScript finds it.

use neon::prelude::*;

use super::DisjointHandle;
use crate::{
    entities::RegistryHandle,
    errors::{Failure, throw},
};

/// `disjointEntity(registry, name)`: the disjoint enum the module registers
/// as `name`.
// [spec:pgorm:req:bindings.disjoint+2]
pub(crate) fn disjoint_entity(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = cx.argument::<JsBox<RegistryHandle>>(0)?.0.clone();
    let name = cx.argument::<JsString>(1)?.value(&mut cx);
    match registry.0.disjoints().get(&name) {
        Some(core) => Ok(cx.boxed(DisjointHandle(core.clone())).upcast()),
        None => throw(
            &mut cx,
            Failure::Construction(format!("no disjoint entity is registered as {name:?}")),
        ),
    }
}

/// `disjointEntities(registry)`: the names of the disjoint enums the module
/// registers, in order.
// [spec:pgorm:req:bindings.disjoint+2]
pub(crate) fn disjoint_entities(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = cx.argument::<JsBox<RegistryHandle>>(0)?.0.clone();
    let disjoints = registry.0.disjoints();
    let names = JsArray::new(&mut cx, disjoints.len());
    for (index, name) in disjoints.keys().enumerate() {
        let name = cx.string(name);
        names.set(&mut cx, u32::try_from(index).unwrap_or(u32::MAX), name)?;
    }
    Ok(names.upcast())
}
