//! A downstream crate's `#[derive(OneOf)]` enum, registered with its entity
//! and its targets' entities.

use std::{collections::BTreeMap, sync::Arc};

use pgorm::{EntityTrait, OneOf, OneOfShape, OneOfTarget, Related, TargetVisitor};
use pyo3::prelude::*;
use serde_json::json;

use super::{Core, NativeOneOf};
use crate::{
    UnsupportedCapabilityError,
    entities::{NativeRegistry, Registry},
    errors::ConstructionError,
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
    names: PyResult<Vec<String>>,
}

impl<O: OneOf> TargetVisitor<O> for Targets<'_> {
    fn target<T: OneOfTarget<O>>(&mut self)
    where
        O::Entity: Related<T>,
    {
        if let Ok(names) = &mut self.names {
            match self.registry.registration_name::<T>() {
                Ok(name) => names.push(name),
                Err(error) => self.names = Err(error),
            }
        }
    }
}

impl Registry {
    /// Register the `#[derive(OneOf)]` enum `O` under `name`, its entity and
    /// each target's registered before it. A name or an enum registered
    /// twice, and an entity not registered, are each a `ConstructionError`
    /// raised as it is registered.
    // [spec:pgorm:req:bindings.one-of+1]
    // [spec:pgorm:req:bindings.entities+6]
    pub fn one_of<O: OneOf>(&mut self, name: &str) -> PyResult<&mut Self> {
        if name.is_empty() || name.len() > 255 || name.contains('\0') {
            return Err(ConstructionError::new_err(
                "registration names require 1–255 UTF-8 bytes without NUL",
            ));
        }
        if self.one_ofs.by_name.contains_key(name)
            || self
                .one_ofs
                .by_name
                .values()
                .any(|core| core.rust == Some(std::any::TypeId::of::<O>()))
        {
            return Err(ConstructionError::new_err(
                "duplicate one-of registration name or Rust type",
            ));
        }
        let entity = self.registration_name::<O::Entity>()?;
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
                rust: Some(std::any::TypeId::of::<O>()),
            }),
        );
        Ok(self)
    }

    /// The name the entity `E` is registered under, or a refusal naming it.
    fn registration_name<E: EntityTrait>(&self) -> PyResult<String> {
        self.backend_of(std::any::TypeId::of::<E>())
            .map(|backend| backend.info().name.clone())
            .ok_or_else(|| {
                ConstructionError::new_err(format!(
                    "register entity {} before the one-of reference over it",
                    std::any::type_name::<E>()
                ))
            })
    }
}

fn registry<'py>(module: &Bound<'py, PyModule>) -> PyResult<Bound<'py, PyAny>> {
    module.getattr("_registry")
}

/// `one_of_entity(name)`: the one-of enum the module registers as `name`.
// [spec:pgorm:req:bindings.one-of+1]
#[pyfunction(pass_module, name = "_one_of_entity")]
pub(crate) fn one_of_entity(module: &Bound<'_, PyModule>, name: &str) -> PyResult<NativeOneOf> {
    let object = registry(module)?;
    let registry = object.extract::<PyRef<'_, NativeRegistry>>()?;
    registry
        .0
        .one_ofs
        .by_name
        .get(name)
        .cloned()
        .map(|core| NativeOneOf { core })
        .ok_or_else(|| {
            UnsupportedCapabilityError::new_err(
                "one-of entity is not registered in this native build",
            )
        })
}

/// `one_of_entities()`: the names of the one-of enums the module registers,
/// in order.
// [spec:pgorm:req:bindings.one-of+1]
#[pyfunction(pass_module, name = "_one_of_entities")]
pub(crate) fn one_of_entities(module: &Bound<'_, PyModule>) -> PyResult<Vec<String>> {
    let object = registry(module)?;
    let registry = object.extract::<PyRef<'_, NativeRegistry>>()?;
    Ok(registry.0.one_ofs.by_name.keys().cloned().collect())
}
