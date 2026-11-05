//! A downstream crate's `#[derive(Disjoint)]` enum, registered with its base
//! and kind entities.

use std::{collections::BTreeMap, sync::Arc};

use pgorm::{Disjoint, DisjointShape, EntityTrait, Iterable, KindModel, KindVisitor};
use pyo3::prelude::*;
use serde_json::json;

use super::{Codec, Core, NativeDisjoint};
use crate::{
    UnsupportedCapabilityError,
    entities::{NativeRegistry, Registry, backend::EntityBackend},
    errors::ConstructionError,
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
    type Output = PyResult<Option<Arc<dyn EntityBackend>>>;

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
    /// refuses are each a `ConstructionError` raised as it is registered. An
    /// unkinded enum registers the same way and is read-only: its shape
    /// refuses every write and its schema.
    // [spec:pgorm:req:bindings.disjoint+1]
    // [spec:pgorm:req:bindings.disjoint-unkinded]
    // [spec:pgorm:req:bindings.entities+5]
    // [spec:pgorm:sem:disjoint.unkinded+1]
    pub fn disjoint<D>(&mut self, name: &str) -> PyResult<&mut Self>
    where
        D: Disjoint + 'static,
    {
        if name.is_empty() || name.len() > 255 || name.contains('\0') {
            return Err(ConstructionError::new_err(
                "registration names require 1–255 UTF-8 bytes without NUL",
            ));
        }
        if self.disjoints.by_name.contains_key(name)
            || self
                .disjoints
                .by_name
                .values()
                .any(|core| core.rust == Some(std::any::TypeId::of::<D>()))
        {
            return Err(ConstructionError::new_err(
                "duplicate disjoint registration name or Rust type",
            ));
        }
        let shape = DisjointShape::of::<D>();
        shape
            .check()
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        let base = self.backend::<D::Base>()?;
        let kinds = D::Kind::iter()
            .map(|kind| D::visit_kind(kind, Kinds(self)))
            .collect::<PyResult<Vec<_>>>()?;
        let description = json!({
            "name": name,
            "base": base.info().name,
            "kind_field": shape.kind_column().map(|column| column.name().to_string()),
            "kind_type": kind_type(&shape),
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
                codec: Codec::Entities { base, kinds },
                description,
                rust: Some(std::any::TypeId::of::<D>()),
            }),
        );
        Ok(self)
    }

    /// The registered entity `E`, or a refusal naming it.
    fn backend<E: EntityTrait + 'static>(&self) -> PyResult<Arc<dyn EntityBackend>> {
        self.backend_of(std::any::TypeId::of::<E>()).ok_or_else(|| {
            ConstructionError::new_err(format!(
                "register entity {} before the disjoint entity over it",
                std::any::type_name::<E>()
            ))
        })
    }
}

fn registry<'py>(module: &Bound<'py, PyModule>) -> PyResult<Bound<'py, PyAny>> {
    module.getattr("_registry")
}

/// `disjoint_entity(name)`: the disjoint enum the module registers as
/// `name`.
// [spec:pgorm:req:bindings.disjoint+1]
#[pyfunction(pass_module, name = "_disjoint_entity")]
pub(crate) fn disjoint_entity(
    module: &Bound<'_, PyModule>,
    name: &str,
) -> PyResult<NativeDisjoint> {
    let object = registry(module)?;
    let registry = object.extract::<PyRef<'_, NativeRegistry>>()?;
    registry
        .0
        .disjoints
        .by_name
        .get(name)
        .cloned()
        .map(|core| NativeDisjoint { core })
        .ok_or_else(|| {
            UnsupportedCapabilityError::new_err(
                "disjoint entity is not registered in this native build",
            )
        })
}

/// `disjoint_entities()`: the names of the disjoint enums the module
/// registers, in order.
// [spec:pgorm:req:bindings.disjoint+1]
#[pyfunction(pass_module, name = "_disjoint_entities")]
pub(crate) fn disjoint_entities(module: &Bound<'_, PyModule>) -> PyResult<Vec<String>> {
    let object = registry(module)?;
    let registry = object.extract::<PyRef<'_, NativeRegistry>>()?;
    Ok(registry.0.disjoints.by_name.keys().cloned().collect())
}
