//! A downstream crate's `#[derive(OneOf)]` enum, registered with its entity
//! and its targets' entities in pgorm-bindings' registry, as Python finds it.

use pyo3::prelude::*;

use super::NativeOneOf;
use crate::{UnsupportedCapabilityError, entities::NativeRegistry};

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
        .0
        .one_ofs()
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
    Ok(registry.0.0.one_ofs().keys().cloned().collect())
}
