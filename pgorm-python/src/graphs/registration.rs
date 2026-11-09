//! A registered graph shape, as Python finds it by name.

use pyo3::prelude::*;

use super::PyEntityGraph;
use crate::{UnsupportedCapabilityError, entities::NativeRegistry};

#[pyfunction(pass_module)]
pub(crate) fn graph(module: &Bound<'_, PyModule>, name: &str) -> PyResult<PyEntityGraph> {
    let registry_object = module.getattr("_registry")?;
    let registry = registry_object.extract::<PyRef<'_, NativeRegistry>>()?;
    registry
        .0
        .0
        .graphs()
        .get(name)
        .cloned()
        .map(|factory| PyEntityGraph { factory })
        .ok_or_else(|| {
            UnsupportedCapabilityError::new_err(
                "graph shape is not registered in this native build",
            )
        })
}
