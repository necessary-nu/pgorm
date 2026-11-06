use std::{
    any::TypeId,
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use pgorm::{EntityTrait, IntoActiveModel};
use pyo3::prelude::*;

use super::{PyEntity, backend::EntityBackend, generic::EntityAdapter, metadata::EntityInfo};
use crate::{UnsupportedCapabilityError, errors::ConstructionError};

/// Build-time collection of concrete Rust registrations for one native module.
/// A module owns the installed registry; Python cannot append generic types.
#[derive(Debug, Default)]
pub struct Registry {
    entries: BTreeMap<String, Arc<dyn EntityBackend>>,
    /// The first registration of each Rust entity type, which a graph or a
    /// source tuple over the type reads its metadata from.
    types: HashMap<TypeId, String>,
    /// Every registered type with the table its instance names, so one table
    /// is not registered twice under one type.
    tables: HashSet<(TypeId, Option<String>, String)>,
    pub(crate) graphs: BTreeMap<String, Arc<dyn crate::graphs::backend::Factory>>,
    pub(crate) sources: BTreeMap<String, Arc<dyn crate::pipeline::selected_backend::Factory>>,
    pub(crate) disjoints: crate::disjoint::Registered,
    pub(crate) one_ofs: crate::one_of::Registered,
}

impl Registry {
    /// Register the real entity, model, columns and ActiveModel implementation.
    /// Duplicate names or duplicate Rust entity types are rejected atomically.
    /// Its statements name `E::default()`'s table.
    // [spec:pgorm:req:bindings.entities+6]
    pub fn entity<E>(&mut self, name: &str) -> PyResult<&mut Self>
    where
        E: EntityTrait + Send + Sync + 'static,
        E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
        E::ActiveModel: Send + Sync + 'static,
    {
        self.entity_in(name, E::default())
    }

    /// Register `entity`, an instance of an entity whose names are computed
    /// at run time, under `name`: every statement of the registration names
    /// the instance's table, as Rust's `_in` constructors do. A type may be
    /// registered once for each table its instances name; a name, or a type
    /// and table already registered, is rejected atomically.
    // [spec:pgorm:req:bindings.entities+6]
    pub fn entity_in<E>(&mut self, name: &str, entity: E) -> PyResult<&mut Self>
    where
        E: EntityTrait + Send + Sync + 'static,
        E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
        E::ActiveModel: Send + Sync + 'static,
    {
        let table = (
            TypeId::of::<E>(),
            entity.schema_name().map(str::to_owned),
            entity.table_name().to_owned(),
        );
        if self.entries.contains_key(name) || self.tables.contains(&table) {
            return Err(ConstructionError::new_err(
                "duplicate entity registration name or Rust type",
            ));
        }
        let info = Arc::new(EntityInfo::of(name, entity)?);
        self.entries.insert(
            name.to_owned(),
            Arc::new(EntityAdapter::<E> { info, entity }),
        );
        self.types
            .entry(TypeId::of::<E>())
            .or_insert_with(|| name.to_owned());
        self.tables.insert(table);
        Ok(self)
    }

    /// The registered entity whose Rust type is `entity`.
    pub(crate) fn backend_of(&self, entity: TypeId) -> Option<Arc<dyn EntityBackend>> {
        self.types
            .get(&entity)
            .and_then(|name| self.entries.get(name))
            .cloned()
    }

    pub(crate) fn describe(&self) -> Vec<serde_json::Value> {
        self.entries.values().map(|e| e.info().describe()).collect()
    }

    pub(crate) fn registered<E: EntityTrait + 'static>(&self) -> PyResult<Arc<EntityInfo>> {
        let entry = self
            .types
            .get(&TypeId::of::<E>())
            .and_then(|name| self.entries.get(name));
        entry.map(|entry| entry.info().clone()).ok_or_else(|| {
            ConstructionError::new_err(format!(
                "register entity {} before its graph or source tuple",
                std::any::type_name::<E>()
            ))
        })
    }
}

#[derive(Debug)]
#[pyclass(name = "_Registry", module = "pgorm._native", frozen)]
pub(crate) struct NativeRegistry(pub(crate) Registry);

/// The names the module registers of one kind — entities, graph shapes or
/// source tuples — in order.
fn names(
    module: &Bound<'_, PyModule>,
    kind: impl Fn(&Registry) -> Vec<String>,
) -> PyResult<Vec<String>> {
    let registry_object = module.getattr("_registry")?;
    let registry = registry_object.extract::<PyRef<'_, NativeRegistry>>()?;
    Ok(kind(&registry.0))
}

/// `entities()`: the names of the entities this module registers.
// [spec:pgorm:req:bindings.entities+6]
#[pyfunction(pass_module)]
pub(crate) fn entities(module: &Bound<'_, PyModule>) -> PyResult<Vec<String>> {
    names(module, |registry| {
        registry.entries.keys().cloned().collect()
    })
}

/// `graphs()`: the names of the graph shapes this module registers.
// [spec:pgorm:req:bindings.entities+6]
#[pyfunction(pass_module)]
pub(crate) fn graphs(module: &Bound<'_, PyModule>) -> PyResult<Vec<String>> {
    names(module, |registry| registry.graphs.keys().cloned().collect())
}

/// `pipeline.source_tuples()`: the names of the source tuples this module
/// registers.
// [spec:pgorm:req:bindings.entities+6]
#[pyfunction(pass_module)]
pub(crate) fn pipeline_source_tuples(module: &Bound<'_, PyModule>) -> PyResult<Vec<String>> {
    names(module, |registry| {
        registry.sources.keys().cloned().collect()
    })
}

#[pyfunction(pass_module)]
pub(crate) fn entity(module: &Bound<'_, PyModule>, name: &str) -> PyResult<PyEntity> {
    let registry_object = module.getattr("_registry")?;
    let registry = registry_object.extract::<PyRef<'_, NativeRegistry>>()?;
    registry
        .0
        .entries
        .get(name)
        .cloned()
        .map(|backend| PyEntity { backend })
        .ok_or_else(|| {
            UnsupportedCapabilityError::new_err("entity is not registered in this native build")
        })
}
