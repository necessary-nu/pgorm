use pgorm::{EntityTrait, GraphItem, GraphRow, SelectGraph, SelectorTrait};
use pyo3::prelude::*;
use std::{marker::PhantomData, sync::Arc};

use super::{PyEntityGraph, backend::Info, generic::GraphFactory, slots::GraphSlots};
use crate::{
    UnsupportedCapabilityError,
    entities::{NativeRegistry, Registry},
    errors::ConstructionError,
};

impl Registry {
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
        if name.is_empty()
            || name.len() > 255
            || name.contains('\0')
            || self.graphs.contains_key(name)
        {
            return Err(ConstructionError::new_err(
                "graph registration requires a unique name of 1–255 UTF-8 bytes without NUL",
            ));
        }
        let info = Arc::new(Info {
            name: name.to_owned(),
            rust_shape: std::any::type_name::<SelectGraph<E, S>>(),
            bindings: S::bindings(self)?,
        });
        self.graphs.insert(
            name.to_owned(),
            Arc::new(GraphFactory::<E, S, Build> {
                info,
                build,
                marker: PhantomData,
            }),
        );
        Ok(self)
    }
}

#[pyfunction(pass_module)]
pub(crate) fn graph(module: &Bound<'_, PyModule>, name: &str) -> PyResult<PyEntityGraph> {
    let registry_object = module.getattr("_registry")?;
    let registry = registry_object.extract::<PyRef<'_, NativeRegistry>>()?;
    registry
        .0
        .graphs
        .get(name)
        .cloned()
        .map(|factory| PyEntityGraph { factory })
        .ok_or_else(|| {
            UnsupportedCapabilityError::new_err(
                "graph shape is not registered in this native build",
            )
        })
}
