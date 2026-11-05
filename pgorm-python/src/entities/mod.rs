//! Concrete Rust entity registrations, owned by one native extension module.

mod active;
pub(crate) mod backend;
mod column;
pub(crate) mod generic;
mod inserts;
pub(crate) mod io;
pub(crate) mod metadata;
mod model;
pub(crate) mod query;
mod registry;
mod relations;
#[cfg(test)]
mod variants;
mod versions;

pub use active::{ActiveState, PyActiveModel, PyActiveValue};
pub use column::PyEntityColumn;
pub(crate) use inserts::TERMINALS as insert_terminals;
pub use inserts::{PyEntityInsert, PyEntityInsertMany};
pub use model::PyEntityModel;
pub use query::PyEntityQuery;
pub(crate) use registry::NativeRegistry;
pub use registry::Registry;
pub(crate) use versions::TERMINALS as version_terminals;
pub use versions::{PyChange, PyEntityUpdate, PyEntityUpdateMany, PyUpserted};

use crate::identifiers::PyIdentifier;
use pyo3::prelude::*;
use std::sync::Arc;

#[derive(Clone, Debug)]
#[pyclass(name = "Entity", module = "pgorm", frozen, from_py_object)]
pub struct PyEntity {
    pub(crate) backend: Arc<dyn backend::EntityBackend>,
}

#[pymethods]
impl PyEntity {
    #[getter]
    fn name(&self) -> &str {
        &self.backend.info().name
    }

    fn describe<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        py.import("json")?
            .call_method1("loads", (self.backend.info().describe().to_string(),))
    }

    fn col<'py>(
        &self,
        py: Python<'py>,
        name: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyEntityColumn>> {
        let name = PyIdentifier::new(name)?.name;
        self.backend.info().column(&name)?;
        Bound::new(py, PyEntityColumn::new(self.backend.clone(), name)?)
    }

    fn find(&self) -> PyEntityQuery {
        PyEntityQuery {
            inner: self.backend.select(),
        }
    }

    fn active(&self) -> PyActiveModel {
        PyActiveModel {
            inner: self.backend.active(),
        }
    }

    /// An update by the ActiveModel's key, whose terminal returns the row
    /// before and after it.
    fn update(&self, active: PyRef<'_, PyActiveModel>) -> PyResult<PyEntityUpdate> {
        PyEntityUpdate::new(self.backend.clone(), &active)
    }

    /// An update of every row its filters admit, whose terminal returns each
    /// changed row before and after.
    fn update_many(&self) -> PyEntityUpdateMany {
        PyEntityUpdateMany::new(self.backend.clone())
    }

    /// An insert of one ActiveModel, whose terminal says what it did.
    fn insert(&self, active: PyRef<'_, PyActiveModel>) -> PyResult<PyEntityInsert> {
        PyEntityInsert::new(self.backend.clone(), active)
    }

    /// An insert of a batch of ActiveModels, whose terminal says what it did
    /// with each row.
    fn insert_many(&self, actives: Vec<PyRef<'_, PyActiveModel>>) -> PyResult<PyEntityInsertMany> {
        PyEntityInsertMany::new(self.backend.clone(), &actives)
    }

    fn __repr__(&self) -> String {
        format!("Entity({:?})", self.name())
    }
}

pub(crate) fn install(module: &Bound<'_, PyModule>, registry: Registry) -> PyResult<()> {
    module.add_class::<PyEntity>()?;
    module.add_class::<PyEntityColumn>()?;
    module.add_class::<PyEntityQuery>()?;
    module.add_class::<PyEntityModel>()?;
    module.add_class::<PyActiveModel>()?;
    module.add_class::<PyActiveValue>()?;
    module.add_class::<ActiveState>()?;
    module.add_class::<PyChange>()?;
    module.add_class::<PyUpserted>()?;
    module.add_class::<PyEntityUpdate>()?;
    module.add_class::<PyEntityUpdateMany>()?;
    module.add_class::<PyEntityInsert>()?;
    module.add_class::<PyEntityInsertMany>()?;
    module.add("_registry", registry::NativeRegistry(registry))?;
    module.add_function(wrap_pyfunction!(registry::entity, module)?)?;
    module.add_function(wrap_pyfunction!(registry::entities, module)?)?;
    module.add_function(wrap_pyfunction!(registry::graphs, module)?)?;
    module.add_function(wrap_pyfunction!(registry::pipeline_source_tuples, module)?)?;
    Ok(())
}

pub(crate) fn capabilities() -> serde_json::Map<String, serde_json::Value> {
    [
        ("entity", "pgorm_python::entities::Registry"),
        ("entities", "pgorm_python::entities::Registry"),
        ("entity.find", "pgorm::EntityTrait::find"),
        ("entity.column", "pgorm::ColumnTrait"),
        ("entity.all", "pgorm::Select<E>::all"),
        ("entity.one", "pgorm::Select<E>::one"),
        ("entity.optional", "pgorm::Select<E>::one_opt"),
        ("entity.model", "pgorm::ModelTrait"),
        (
            "entity.active",
            "pgorm::ActiveModelBehavior::new / IntoActiveModel",
        ),
        ("entity.active.insert", "pgorm::ActiveModelTrait::insert"),
        ("entity.active.update", "pgorm::ActiveModelTrait::update"),
        ("entity.active.delete", "pgorm::ActiveModelTrait::delete"),
    ]
    .into_iter()
    .map(|(name, api)| {
        (
            name.to_owned(),
            serde_json::json!({"rust_api": api, "features": [], "registration_required": true}),
        )
    })
    .collect()
}
