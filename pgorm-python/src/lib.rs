//! Public native bindings for pgorm.
//!
//! The installed capability manifest records the implemented surface and the
//! concrete Rust API behind each operation. Downstream bindings share this crate.

// The capability manifest is one `json!` literal, deeper than the default limit.
#![recursion_limit = "256"]

mod capabilities;
mod config;
mod copy;
mod disjoint;
pub mod entities;
mod errors;
mod execution;
pub mod expressions;
pub mod graphs;
pub mod identifiers;
pub mod json;
mod listen;
mod models;
mod one_of;
pub mod pipeline;
mod raw;
pub mod results;
mod runtime;
pub mod schema;
pub mod statements;
mod transactions;
pub mod values;

use pyo3::prelude::*;

pyo3::create_exception!(pgorm, PgormError, pyo3::exceptions::PyException);
// An operation outside the capability manifest is an argument the build has
// nothing for, so it is the ConstructionError the shared rules name too.
pyo3::create_exception!(pgorm, UnsupportedCapabilityError, errors::ConstructionError);

// [spec:pgorm:def:bindings.api]
// [spec:pgorm:def:python.api+1]
// [spec:pgorm:req:python.package]
// [spec:pgorm:req:bindings.optional]
/// Initialize the native library without database or scanner side effects.
#[pymodule(gil_used = true)]
#[cfg(feature = "standalone-module")]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    install(module, entities::Registry::default())
}

/// Install the shared Python API and a concrete application registry into one
/// native module. Downstream crates disable `standalone-module` and export
/// their own PyO3 initializer calling this function.
// [spec:pgorm:req:bindings.entities+5]
pub fn install(module: &Bound<'_, PyModule>, registry: entities::Registry) -> PyResult<()> {
    if module.hasattr("_registry")? {
        return Err(errors::ConstructionError::new_err(
            "native module is already initialized",
        ));
    }
    let py = module.py();
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add("__pgorm_version__", env!("PGORM_VERSION"))?;
    module.add("PgormError", py.get_type::<PgormError>())?;
    module.add(
        "UnsupportedCapabilityError",
        py.get_type::<UnsupportedCapabilityError>(),
    )?;
    module.add_function(wrap_pyfunction!(capabilities::capabilities, module)?)?;
    module.add_function(wrap_pyfunction!(capabilities::require_capability, module)?)?;
    errors::register(module)?;
    runtime::register(module)?;
    values::register(module)?;
    expressions::register(module)?;
    json::register(module)?;
    identifiers::register(module)?;
    statements::register(module)?;
    copy::register(module)?;
    listen::register(module)?;
    results::register(module)?;
    raw::register(module)?;
    models::register(module)?;
    entities::install(module, registry)?;
    disjoint::register(module)?;
    one_of::register(module)?;
    graphs::install(module)?;
    pipeline::install(module)?;
    schema::install(module)?;
    Ok(())
}
