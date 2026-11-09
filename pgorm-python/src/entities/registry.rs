//! The build-time collection of an application's concrete registrations,
//! pgorm-bindings' own, refusing as a `ConstructionError`, and the module
//! functions Python finds them by. A module owns the installed registry;
//! Python cannot add a generic instantiation, only use the ones compiled in.

use pgorm::{
    Disjoint, EntityTrait, GraphItem, GraphRow, IntoActiveModel, OneOf, SelectGraph, SelectorTrait,
    pipeline::SourceList,
};
use pgorm_bindings::entity::{GraphSlots, SourceTypes};
use pyo3::prelude::*;

use super::PyEntity;
use crate::{UnsupportedCapabilityError, config::DIALECT, errors::Raise};

/// The concrete Rust entities, graph shapes, source tuples, disjoint enums
/// and one-of enums one native module carries, which Python reaches by
/// name.
#[derive(Debug)]
pub struct Registry(pub(crate) pgorm_bindings::entity::Registry);

impl Default for Registry {
    fn default() -> Self {
        Self(pgorm_bindings::entity::Registry::new(DIALECT))
    }
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
        self.0.entity_in(name, entity).py()?;
        Ok(self)
    }

    /// Register a concrete SelectGraph shape and its application join factory.
    /// The factory receives one validated alias for each declared joined slot,
    /// in tuple order, and must use those aliases in the corresponding joins.
    /// Register every source entity first. Python can vary aliases and builder
    /// operations but cannot change this Rust tuple's entity or slot types.
    // [spec:pgorm:req:bindings.entity-graphs+4]
    pub fn graph<E, S, Build>(&mut self, name: &str, build: Build) -> PyResult<&mut Self>
    where
        E: EntityTrait + Send + Sync + 'static,
        S: GraphSlots<E>,
        GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
        GraphItem<E, S>: Send + 'static,
        Build: Fn(&[String]) -> SelectGraph<E, S> + Send + Sync + 'static,
    {
        self.0.graph(name, build).py()?;
        Ok(self)
    }

    /// Register a concrete tuple of one to six entity types for select_sources.
    /// Register each entity first; Python cannot invent Rust source types.
    // [spec:pgorm:req:bindings.pipeline-sources+3]
    pub fn sources<T: SourceTypes>(&mut self, name: &str) -> PyResult<&mut Self>
    where
        <T::Selection as SourceList>::Row: Send,
    {
        self.0.sources::<T>(name).py()?;
        Ok(self)
    }

    /// Register the `#[derive(Disjoint)]` enum `D` under `name`, its base and
    /// each kind's entity registered before it. A name or an enum registered
    /// twice, an entity not registered, and a declaration `check_disjoint`
    /// refuses are each a `ConstructionError` raised as it is registered. An
    /// unkinded enum registers the same way and is read-only: its shape
    /// refuses every write and its schema.
    // [spec:pgorm:req:bindings.disjoint+2]
    // [spec:pgorm:req:bindings.disjoint-unkinded]
    pub fn disjoint<D: Disjoint + 'static>(&mut self, name: &str) -> PyResult<&mut Self> {
        self.0.disjoint::<D>(name).py()?;
        Ok(self)
    }

    /// Register the `#[derive(OneOf)]` enum `O` under `name`, its entity and
    /// each target's registered before it. A name or an enum registered
    /// twice, and an entity not registered, are each a `ConstructionError`
    /// raised as it is registered.
    // [spec:pgorm:req:bindings.one-of+1]
    pub fn one_of<O: OneOf>(&mut self, name: &str) -> PyResult<&mut Self> {
        self.0.one_of::<O>(name).py()?;
        Ok(self)
    }

    pub(crate) fn describe(&self) -> Vec<serde_json::Value> {
        self.0
            .entities()
            .values()
            .map(|entity| entity.info().describe(DIALECT))
            .collect()
    }
}

#[derive(Debug)]
#[pyclass(name = "_Registry", module = "pgorm._native", frozen)]
pub(crate) struct NativeRegistry(pub(crate) Registry);

/// The registry `module` installed.
pub(crate) fn installed<'py>(
    module: &Bound<'py, PyModule>,
) -> PyResult<PyRef<'py, NativeRegistry>> {
    module
        .getattr("_registry")?
        .extract::<PyRef<'_, NativeRegistry>>()
        .map_err(Into::into)
}

/// The names the module registers of one kind, entities, graph shapes or
/// source tuples, in order.
fn names(
    module: &Bound<'_, PyModule>,
    kind: impl Fn(&pgorm_bindings::entity::Registry) -> Vec<String>,
) -> PyResult<Vec<String>> {
    Ok(kind(&installed(module)?.0.0))
}

/// `entities()`: the names of the entities this module registers.
// [spec:pgorm:req:bindings.entities+6]
#[pyfunction(pass_module)]
pub(crate) fn entities(module: &Bound<'_, PyModule>) -> PyResult<Vec<String>> {
    names(module, |registry| {
        registry.entities().keys().cloned().collect()
    })
}

/// `graphs()`: the names of the graph shapes this module registers.
// [spec:pgorm:req:bindings.entities+6]
#[pyfunction(pass_module)]
pub(crate) fn graphs(module: &Bound<'_, PyModule>) -> PyResult<Vec<String>> {
    names(module, |registry| {
        registry.graphs().keys().cloned().collect()
    })
}

/// `pipeline.source_tuples()`: the names of the source tuples this module
/// registers.
// [spec:pgorm:req:bindings.entities+6]
#[pyfunction(pass_module)]
pub(crate) fn pipeline_source_tuples(module: &Bound<'_, PyModule>) -> PyResult<Vec<String>> {
    names(module, |registry| {
        registry.source_tuples().keys().cloned().collect()
    })
}

#[pyfunction(pass_module)]
pub(crate) fn entity(module: &Bound<'_, PyModule>, name: &str) -> PyResult<PyEntity> {
    installed(module)?
        .0
        .0
        .entities()
        .get(name)
        .cloned()
        .map(|backend| PyEntity { backend })
        .ok_or_else(|| {
            UnsupportedCapabilityError::new_err("entity is not registered in this native build")
        })
}
