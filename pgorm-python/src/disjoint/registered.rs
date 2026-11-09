//! A downstream crate's `#[derive(Disjoint)]` enum, registered with its base
//! and kind entities in pgorm-bindings' registry, as Python finds it.

use pyo3::prelude::*;

use super::NativeDisjoint;
use crate::{UnsupportedCapabilityError, entities::NativeRegistry};

fn registry<'py>(module: &Bound<'py, PyModule>) -> PyResult<Bound<'py, PyAny>> {
    module.getattr("_registry")
}

/// `disjoint_entity(name)`: the disjoint enum the module registers as
/// `name`.
// [spec:pgorm:req:bindings.disjoint+2]
#[pyfunction(pass_module, name = "_disjoint_entity")]
pub(crate) fn disjoint_entity(
    module: &Bound<'_, PyModule>,
    name: &str,
) -> PyResult<NativeDisjoint> {
    let object = registry(module)?;
    let registry = object.extract::<PyRef<'_, NativeRegistry>>()?;
    registry
        .0
        .0
        .disjoints()
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
// [spec:pgorm:req:bindings.disjoint+2]
#[pyfunction(pass_module, name = "_disjoint_entities")]
pub(crate) fn disjoint_entities(module: &Bound<'_, PyModule>) -> PyResult<Vec<String>> {
    let object = registry(module)?;
    let registry = object.extract::<PyRef<'_, NativeRegistry>>()?;
    Ok(registry.0.0.disjoints().keys().cloned().collect())
}
