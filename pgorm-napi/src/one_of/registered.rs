//! A downstream crate's `#[derive(OneOf)]` enum, registered with its entity
//! and its targets' entities in pgorm-bindings' registry, as JavaScript finds
//! it.

use neon::prelude::*;

use super::OneOfHandle;
use crate::{
    entities::RegistryHandle,
    errors::{Failure, throw},
};

/// `oneOfEntity(registry, name)`: the one-of enum the module registers as
/// `name`.
// [spec:pgorm:req:bindings.one-of+1]
pub(crate) fn one_of_entity(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = cx.argument::<JsBox<RegistryHandle>>(0)?.0.clone();
    let name = cx.argument::<JsString>(1)?.value(&mut cx);
    match registry.0.one_ofs().get(&name) {
        Some(core) => Ok(cx.boxed(OneOfHandle(core.clone())).upcast()),
        None => throw(
            &mut cx,
            Failure::Construction(format!("no one-of entity is registered as {name:?}")),
        ),
    }
}

/// `oneOfEntities(registry)`: the names of the one-of enums the module
/// registers, in order.
// [spec:pgorm:req:bindings.one-of+1]
pub(crate) fn one_of_entities(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = cx.argument::<JsBox<RegistryHandle>>(0)?.0.clone();
    let one_ofs = registry.0.one_ofs();
    let names = JsArray::new(&mut cx, one_ofs.len());
    for (index, name) in one_ofs.keys().enumerate() {
        let name = cx.string(name);
        names.set(&mut cx, u32::try_from(index).unwrap_or(u32::MAX), name)?;
    }
    Ok(names.upcast())
}
