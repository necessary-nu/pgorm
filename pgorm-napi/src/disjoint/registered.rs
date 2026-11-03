//! A downstream crate's `#[derive(Disjoint)]` enum, registered with its base
//! and kind entities.

use std::{any::TypeId, collections::BTreeMap, sync::Arc};

use neon::prelude::*;
use pgorm::{Disjoint, DisjointShape, Iterable, KindModel, KindVisitor};
use serde_json::json;

use super::{Core, DisjointHandle, RowCodec};
use crate::{
    entities::{EntityBackend, RegistrationError, Registry, RegistryHandle},
    errors::{Failure, throw},
};

/// The disjoint enums a module registers, by name.
#[derive(Default)]
pub(crate) struct Registered {
    pub(crate) by_name: BTreeMap<String, Arc<Core>>,
}

impl std::fmt::Debug for Registered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.by_name.keys()).finish()
    }
}

/// Finds each kind's registered entity.
struct Kinds<'r>(&'r Registry);

impl<D: Disjoint> KindVisitor<D> for Kinds<'_> {
    type Output = Result<Option<Arc<dyn EntityBackend>>, RegistrationError>;

    fn table<M: KindModel<D>>(self, _: D::Kind) -> Self::Output {
        self.0.backend::<M::Entity>().map(Some)
    }

    fn unit(self, _: D) -> Self::Output {
        Ok(None)
    }
}

/// The kind type's schema and name, as a registration describes it.
fn kind_type(shape: &DisjointShape) -> serde_json::Value {
    let name = shape.kind_type_name();
    json!({
        "schema": name.as_ref().and_then(|name| name.schema.as_ref()).map(|schema| schema.to_string()),
        "name": name.map(|name| name.name.to_string()),
    })
}

impl Registry {
    /// Register the `#[derive(Disjoint)]` enum `D` under `name`, its base and
    /// each kind's entity registered before it. A name or an enum registered
    /// twice, an entity not registered, and a declaration `check_disjoint`
    /// refuses are each refused, and nothing is added.
    // [spec:pgorm:req:bindings.disjoint+1]
    // [spec:pgorm:req:bindings.entities+3]
    pub fn disjoint<D>(&mut self, name: &str) -> Result<&mut Self, RegistrationError>
    where
        D: Disjoint + 'static,
    {
        crate::entities::registration_name(name)?;
        let rust = TypeId::of::<D>();
        if self.disjoints.by_name.contains_key(name)
            || self
                .disjoints
                .by_name
                .values()
                .any(|core| core.rust == Some(rust))
        {
            return Err(RegistrationError(format!(
                "{name:?} or its Rust disjoint enum is already registered"
            )));
        }
        let shape = DisjointShape::of::<D>();
        shape
            .check()
            .map_err(|error| RegistrationError(error.to_string()))?;
        let base = self.backend::<D::Base>()?;
        let kinds = D::Kind::iter()
            .map(|kind| D::visit_kind(kind, Kinds(self)))
            .collect::<Result<Vec<_>, _>>()?;
        let description = json!({
            "name": name,
            "base": base.info().name,
            "kindField": shape.kind_column().name().to_string(),
            "kindType": kind_type(&shape),
            "kinds": shape
                .kinds()
                .iter()
                .zip(&kinds)
                .map(|(kind, entity)| json!({
                    "value": kind.value(),
                    "model": entity.as_ref().map(|entity| entity.info().name.clone()),
                }))
                .collect::<Vec<_>>(),
        });
        self.disjoints.by_name.insert(
            name.to_owned(),
            Arc::new(Core {
                shape,
                codec: RowCodec::Entities { base, kinds },
                description,
                rust: Some(rust),
            }),
        );
        Ok(self)
    }
}

/// `disjointEntity(registry, name)`: the disjoint enum the module registers
/// as `name`.
// [spec:pgorm:req:bindings.disjoint+1]
pub(crate) fn disjoint_entity(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = cx.argument::<JsBox<RegistryHandle>>(0)?.0.clone();
    let name = cx.argument::<JsString>(1)?.value(&mut cx);
    match registry.disjoints.by_name.get(&name) {
        Some(core) => Ok(cx.boxed(DisjointHandle(core.clone())).upcast()),
        None => throw(
            &mut cx,
            Failure::Construction(format!("no disjoint entity is registered as {name:?}")),
        ),
    }
}

/// `disjointEntities(registry)`: the names of the disjoint enums the module
/// registers, in order.
// [spec:pgorm:req:bindings.disjoint+1]
pub(crate) fn disjoint_entities(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = cx.argument::<JsBox<RegistryHandle>>(0)?.0.clone();
    let names = JsArray::new(&mut cx, registry.disjoints.by_name.len());
    for (index, name) in registry.disjoints.by_name.keys().enumerate() {
        let name = cx.string(name);
        names.set(&mut cx, u32::try_from(index).unwrap_or(u32::MAX), name)?;
    }
    Ok(names.upcast())
}
