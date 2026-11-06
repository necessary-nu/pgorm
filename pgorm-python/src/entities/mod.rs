//! Concrete Rust entity registrations, owned by one native extension module.

mod active;
mod column;
pub(crate) mod input;
mod inserts;
pub(crate) mod io;
mod model;
pub(crate) mod query;
pub(crate) mod reads;
pub(crate) mod reads_py;
mod registry;
mod selection;
mod versions;

pub use active::{ActiveState, PyActiveModel, PyActiveValue};
pub use column::PyEntityColumn;
pub(crate) use inserts::TERMINALS as insert_terminals;
pub use inserts::{PyEntityInsert, PyEntityInsertMany};
pub use model::PyEntityModel;
pub use query::{PyEntityQuery, PyEntityRawQuery};
pub use reads_py::{PyEntityCursor, PyEntityPaginator};
pub(crate) use registry::NativeRegistry;
pub use registry::Registry;
pub use selection::{PyEntityRelation, PyEntitySelection};
pub(crate) use versions::TERMINALS as version_terminals;
pub use versions::{PyChange, PyEntityUpdate, PyEntityUpdateMany, PyUpserted};

use crate::{config::DIALECT, errors::Raise, identifiers::Identifier};
use pgorm_bindings::entity::EntityBackend;
use pyo3::prelude::*;
use std::sync::Arc;

#[derive(Clone, Debug)]
#[pyclass(name = "Entity", module = "pgorm", frozen, from_py_object)]
pub struct PyEntity {
    pub(crate) backend: Arc<dyn EntityBackend>,
}

#[pymethods]
impl PyEntity {
    #[getter]
    fn name(&self) -> &str {
        &self.backend.info().name
    }

    fn describe<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        py.import("json")?.call_method1(
            "loads",
            (self.backend.info().describe(DIALECT).to_string(),),
        )
    }

    fn col<'py>(
        &self,
        py: Python<'py>,
        name: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyEntityColumn>> {
        let name = Identifier::new(name)?.name;
        self.backend.info().column(&name).py()?;
        Bound::new(py, PyEntityColumn::new(self.backend.clone(), name)?)
    }

    fn find(&self) -> PyEntityQuery {
        PyEntityQuery {
            inner: self.backend.select(),
        }
    }

    /// The relation the entity's `Relation` enum declares under `name`, for
    /// a query to join.
    fn relation(&self, name: &str) -> PyResult<PyEntityRelation> {
        self.backend.relation(name).py()?;
        Ok(PyEntityRelation {
            entity: self.backend.clone(),
            name: name.to_owned(),
        })
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
    module.add_class::<PyEntityRawQuery>()?;
    module.add_class::<PyEntitySelection>()?;
    module.add_class::<PyEntityPaginator>()?;
    module.add_class::<reads_py::PyEntityPages>()?;
    module.add_class::<PyEntityCursor>()?;
    module.add_class::<PyEntityRelation>()?;
    module.add_class::<reads::NativeRowDecoder>()?;
    module.add_class::<PyEntityModel>()?;
    module.add_class::<PyActiveModel>()?;
    module.add_class::<PyActiveValue>()?;
    module.add_class::<ActiveState>()?;
    module.add_class::<PyChange>()?;
    module.add_class::<PyUpserted>()?;
    module.add_function(wrap_pyfunction!(versions::make_change, module)?)?;
    module.add_function(wrap_pyfunction!(versions::make_inserted, module)?)?;
    module.add_function(wrap_pyfunction!(versions::make_updated, module)?)?;
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
        ("entity.count", "pgorm::PaginatorTrait::count"),
        ("entity.stream", "pgorm::Select<E>::stream"),
        (
            "entity.paginate",
            "pgorm::PaginatorTrait::paginate, Paginator::{fetch_page,num_items_and_pages}",
        ),
        (
            "entity.cursor",
            "pgorm::Select<E>::cursor_by, pgorm::Cursor",
        ),
        (
            "entity.join",
            "pgorm::QuerySelect::join, RelationTrait::def",
        ),
        ("entity.select", "pgorm::QuerySelect::{select_only,columns}"),
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
