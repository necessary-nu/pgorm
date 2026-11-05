//! A downstream crate's `#[derive(OneOf)]` enum, registered with its entity
//! and its targets' entities.

use std::{any::TypeId, collections::BTreeMap, sync::Arc};

use neon::prelude::*;
use pgorm::{EntityTrait, OneOf, OneOfShape, OneOfTarget, Related, TargetVisitor};
use serde_json::json;

use super::{Core, OneOfHandle};
use crate::{
    entities::{RegistrationError, Registry, RegistryHandle},
    errors::{Failure, throw},
};

/// The one-of enums a module registers, by name.
#[derive(Default)]
pub(crate) struct Registered {
    pub(crate) by_name: BTreeMap<String, Arc<Core>>,
}

impl std::fmt::Debug for Registered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.by_name.keys()).finish()
    }
}

/// Each target's registration name, in declaration order, or the first
/// target not registered.
struct Targets<'r> {
    registry: &'r Registry,
    names: Result<Vec<String>, RegistrationError>,
}

impl<O: OneOf> TargetVisitor<O> for Targets<'_> {
    fn target<T: OneOfTarget<O>>(&mut self)
    where
        O::Entity: Related<T>,
    {
        if let Ok(names) = &mut self.names {
            match registration_name::<T>(self.registry) {
                Ok(name) => names.push(name),
                Err(error) => self.names = Err(error),
            }
        }
    }
}

/// The name the entity `E` is registered under, or a refusal naming it.
fn registration_name<E: EntityTrait>(registry: &Registry) -> Result<String, RegistrationError> {
    registry
        .backend_of(TypeId::of::<E>())
        .map(|backend| backend.info().name.clone())
        .ok_or_else(|| {
            RegistrationError(format!(
                "register the entity {} before the one-of reference over it",
                std::any::type_name::<E>()
            ))
        })
}

impl Registry {
    /// Register the `#[derive(OneOf)]` enum `O` under `name`, its entity and
    /// each target's registered before it. A name or an enum registered
    /// twice, and an entity not registered, are each refused, and nothing is
    /// added.
    // [spec:pgorm:req:bindings.one-of+1]
    // [spec:pgorm:req:bindings.entities+5]
    pub fn one_of<O: OneOf>(&mut self, name: &str) -> Result<&mut Self, RegistrationError> {
        crate::entities::registration_name(name)?;
        let rust = TypeId::of::<O>();
        if self.one_ofs.by_name.contains_key(name)
            || self
                .one_ofs
                .by_name
                .values()
                .any(|core| core.rust == Some(rust))
        {
            return Err(RegistrationError(format!(
                "{name:?} or its Rust one-of enum is already registered"
            )));
        }
        let entity = registration_name::<O::Entity>(self)?;
        let mut targets = Targets {
            registry: self,
            names: Ok(Vec::new()),
        };
        O::visit_targets(&mut targets);
        let targets = targets.names?;
        let shape = OneOfShape::of::<O>();
        let description = json!({
            "name": name,
            "model": entity,
            "targets": shape
                .targets()
                .iter()
                .zip(&targets)
                .map(|(target, registered)| json!({
                    "value": target.value(),
                    "field": target.column().to_string(),
                    "target": registered,
                }))
                .collect::<Vec<_>>(),
            "optional": shape.is_optional(),
        });
        self.one_ofs.by_name.insert(
            name.to_owned(),
            Arc::new(Core {
                shape,
                description,
                rust: Some(rust),
            }),
        );
        Ok(self)
    }
}

/// `oneOfEntity(registry, name)`: the one-of enum the module registers as
/// `name`.
// [spec:pgorm:req:bindings.one-of+1]
pub(crate) fn one_of_entity(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = cx.argument::<JsBox<RegistryHandle>>(0)?.0.clone();
    let name = cx.argument::<JsString>(1)?.value(&mut cx);
    match registry.one_ofs.by_name.get(&name) {
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
    let names = JsArray::new(&mut cx, registry.one_ofs.by_name.len());
    for (index, name) in registry.one_ofs.by_name.keys().enumerate() {
        let name = cx.string(name);
        names.set(&mut cx, u32::try_from(index).unwrap_or(u32::MAX), name)?;
    }
    Ok(names.upcast())
}
