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
pub use inserts::{PyEntityInsert, PyEntityInsertMany, PyEntityTryInsert, PyEntityTryInsertMany};
pub use model::PyEntityModel;
pub use query::{PyEntityQuery, PyEntityRawQuery};
pub use reads_py::{PyEntityCursor, PyEntityPaginator};
pub(crate) use registry::NativeRegistry;
pub use registry::Registry;
pub use selection::{PyEntityRelation, PyEntitySelection};
pub(crate) use versions::TERMINALS as version_terminals;
pub use versions::{PyChange, PyEntityUpdate, PyEntityUpdateMany, PyUpserted};

use crate::{config::DIALECT, errors::Raise, identifiers::Identifier, scopes::PyScopeContext};
use pgorm_bindings::entity::EntityBackend;
use pyo3::prelude::*;
use std::sync::Arc;

#[derive(Clone, Debug)]
#[pyclass(name = "Entity", module = "pgorm", frozen, from_py_object)]
pub struct PyEntity {
    pub(crate) backend: Arc<dyn EntityBackend>,
}

impl PyEntity {
    fn find_under(&self, cx: Option<&PyScopeContext>) -> PyResult<PyEntityQuery> {
        let cx = PyScopeContext::scoping(cx, self.backend.info())?;
        Ok(PyEntityQuery {
            inner: self.backend.select(&cx),
        })
    }
}

#[pymethods]
impl PyEntity {
    #[getter]
    fn name(&self) -> &str {
        &self.backend.info().name
    }

    /// The registration as pgorm-bindings describes it, each relation with
    /// its condition type and whether an `on_condition` adds a condition.
    // [spec:pgorm:req:bindings.relation-conditions]
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

    /// The scopes the entity declares, by name: none for an unscoped one.
    #[getter]
    fn scopes(&self) -> Vec<&'static str> {
        self.backend.info().scoped.clone()
    }

    fn find(&self) -> PyResult<PyEntityQuery> {
        self.find_under(None)
    }

    /// `Entity::find_for`: the rows `cx`'s scopes hold.
    // [spec:pgorm:req:bindings.scopes-registered]
    fn find_for(&self, cx: &Bound<'_, PyAny>) -> PyResult<PyEntityQuery> {
        self.find_under(Some(PyScopeContext::read(cx)?.get()))
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
        let cx = PyScopeContext::scoping(None, self.backend.info())?;
        PyEntityUpdate::new(self.backend.clone(), &active, cx)
    }

    /// `Update::one_for`: the update by key under `cx`, a row outside its
    /// scopes no row.
    // [spec:pgorm:req:bindings.scopes-registered]
    fn update_for(
        &self,
        cx: &Bound<'_, PyAny>,
        active: PyRef<'_, PyActiveModel>,
    ) -> PyResult<PyEntityUpdate> {
        let cx =
            PyScopeContext::scoping(Some(PyScopeContext::read(cx)?.get()), self.backend.info())?;
        PyEntityUpdate::new(self.backend.clone(), &active, cx)
    }

    /// An update of every row its filters admit, whose terminal returns each
    /// changed row before and after.
    fn update_many(&self) -> PyResult<PyEntityUpdateMany> {
        let cx = PyScopeContext::scoping(None, self.backend.info())?;
        Ok(PyEntityUpdateMany::new(self.backend.clone(), cx))
    }

    /// `Update::many_for`: the update of the rows `cx`'s scopes hold.
    // [spec:pgorm:req:bindings.scopes-registered]
    fn update_many_for(&self, cx: &Bound<'_, PyAny>) -> PyResult<PyEntityUpdateMany> {
        let cx =
            PyScopeContext::scoping(Some(PyScopeContext::read(cx)?.get()), self.backend.info())?;
        Ok(PyEntityUpdateMany::new(self.backend.clone(), cx))
    }

    /// An insert of one ActiveModel, whose terminal says what it did.
    fn insert(&self, active: PyRef<'_, PyActiveModel>) -> PyResult<PyEntityInsert> {
        let cx = PyScopeContext::scoping(None, self.backend.info())?;
        PyEntityInsert::new(self.backend.clone(), active, cx)
    }

    /// `Insert::one_for`: the row's scope columns filled from `cx`.
    // [spec:pgorm:req:bindings.scopes-registered]
    fn insert_for(
        &self,
        cx: &Bound<'_, PyAny>,
        active: PyRef<'_, PyActiveModel>,
    ) -> PyResult<PyEntityInsert> {
        let cx =
            PyScopeContext::scoping(Some(PyScopeContext::read(cx)?.get()), self.backend.info())?;
        PyEntityInsert::new(self.backend.clone(), active, cx)
    }

    /// An insert of a batch of ActiveModels, whose terminal says what it did
    /// with each row.
    fn insert_many(&self, actives: Vec<PyRef<'_, PyActiveModel>>) -> PyResult<PyEntityInsertMany> {
        let cx = PyScopeContext::scoping(None, self.backend.info())?;
        PyEntityInsertMany::new(self.backend.clone(), &actives, cx)
    }

    /// `Insert::many_for`: each row's scope columns filled from `cx`.
    // [spec:pgorm:req:bindings.scopes-registered]
    fn insert_many_for(
        &self,
        cx: &Bound<'_, PyAny>,
        actives: Vec<PyRef<'_, PyActiveModel>>,
    ) -> PyResult<PyEntityInsertMany> {
        let cx =
            PyScopeContext::scoping(Some(PyScopeContext::read(cx)?.get()), self.backend.info())?;
        PyEntityInsertMany::new(self.backend.clone(), &actives, cx)
    }

    /// `Pipeline::from_for`: a pipeline over the rows `cx`'s scopes hold.
    // [spec:pgorm:req:bindings.scopes-registered]
    fn _pipeline_for(&self, cx: &Bound<'_, PyAny>) -> PyResult<crate::pipeline::PyPipeline> {
        let cx =
            PyScopeContext::scoping(Some(PyScopeContext::read(cx)?.get()), self.backend.info())?;
        Ok(crate::pipeline::PyPipeline {
            inner: self.backend.pipeline(&cx),
        })
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
    module.add_class::<PyEntityTryInsert>()?;
    module.add_class::<PyEntityTryInsertMany>()?;
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
        ("entity.find", "pgorm::Entity::find"),
        ("entity.column", "pgorm::Column"),
        ("entity.all", "pgorm::Select<E>::all"),
        ("entity.one", "pgorm::Select<E>::one"),
        ("entity.optional", "pgorm::Select<E>::one_opt"),
        ("entity.count", "pgorm::Paginate::count"),
        ("entity.stream", "pgorm::Select<E>::stream"),
        (
            "entity.paginate",
            "pgorm::Paginate::paginate, Paginator::{fetch_page,num_items_and_pages}",
        ),
        (
            "entity.cursor",
            "pgorm::Select<E>::cursor_by, pgorm::Cursor",
        ),
        ("entity.join", "pgorm::QuerySelect::join, Relation::def"),
        ("entity.select", "pgorm::QuerySelect::{select_only,columns}"),
        ("entity.model", "pgorm::Model"),
        (
            "entity.active",
            "pgorm::ActiveModelBehavior::new / IntoActiveModel",
        ),
        ("entity.active.insert", "pgorm::ActiveModel::insert"),
        ("entity.active.update", "pgorm::ActiveModel::update"),
        ("entity.active.delete", "pgorm::ActiveModel::delete"),
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
