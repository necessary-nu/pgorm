//! Concrete SelectGraph registrations with the Rust slot and decode shape intact.

pub(crate) mod backend;
mod cursor;
mod generic;
mod io;
mod keysets;
mod query;
mod registration;
mod slots;

pub use cursor::PyEntityGraphCursor;
pub use query::PyEntityGraphQuery;
pub use slots::{GraphBindings, GraphSlots};

use crate::{errors::ConstructionError, identifiers::validate_name};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyString};
use std::sync::Arc;

#[derive(Clone, Debug)]
#[pyclass(name = "EntityGraph", module = "pgorm", frozen, from_py_object)]
pub struct PyEntityGraph {
    pub(crate) factory: Arc<dyn backend::Factory>,
}

#[pymethods]
impl PyEntityGraph {
    #[getter]
    fn name(&self) -> &str {
        &self.factory.info().name
    }

    fn describe<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        py.import("json")?
            .call_method1("loads", (self.factory.info().describe().to_string(),))
    }

    #[pyo3(signature = (*, aliases=None))]
    fn find(&self, aliases: Option<&Bound<'_, PyAny>>) -> PyResult<PyEntityGraphQuery> {
        let sources = &self.factory.info().bindings.sources;
        let root = &sources[0].entity.table;
        let aliases = match aliases.filter(|value| !value.is_none()) {
            Some(value) if !value.is_instance_of::<PyString>() => {
                value.extract::<Vec<String>>().map_err(|_| {
                    PyTypeError::new_err("aliases require a sequence of joined-source names")
                })?
            }
            Some(_) => {
                return Err(PyTypeError::new_err(
                    "aliases require a sequence, not one string",
                ));
            }
            None => (1..sources.len())
                .map(|at| {
                    let alias = format!("g{at}");
                    if &alias == root {
                        format!("_g{at}")
                    } else {
                        alias
                    }
                })
                .collect(),
        };
        if aliases.len() + 1 != sources.len() {
            return Err(ConstructionError::new_err(
                "alias count must match the registered joined-slot count",
            ));
        }
        let mut seen = std::collections::HashSet::from([root.clone()]);
        for alias in &aliases {
            validate_name(alias)?;
            if !seen.insert(alias.clone()) {
                return Err(ConstructionError::new_err(
                    "graph source aliases must be distinct",
                ));
            }
        }
        Ok(PyEntityGraphQuery {
            inner: self.factory.find(aliases),
        })
    }
}

pub(crate) fn install(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyEntityGraph>()?;
    module.add_class::<PyEntityGraphQuery>()?;
    module.add_class::<PyEntityGraphCursor>()?;
    module.add_function(wrap_pyfunction!(registration::graph, module)?)?;
    Ok(())
}

pub(crate) fn capabilities() -> serde_json::Map<String, serde_json::Value> {
    [
        ("graph", "pgorm_python::entities::Registry::graph"),
        ("graphs", "pgorm_python::entities::Registry::graph"),
        ("graph.find", "application SelectGraph<E, S> factory"),
        ("graph.column", "pgorm::pgorm_query::Expr::col"),
        ("graph.where", "pgorm::QueryFilter"),
        ("graph.order_by", "pgorm::QueryOrder"),
        ("graph.inspect", "pgorm::QueryTrait::build"),
        ("graph.all", "pgorm::SelectGraph<E, S>::all"),
        ("graph.optional", "pgorm::SelectGraph<E, S>::one_opt"),
        ("graph.cursor", "pgorm::SelectGraph<E, S>::cursor_by"),
        (
            "graph.cursor_by_on",
            "pgorm::SelectGraph<E, S>::cursor_by_on",
        ),
        (
            "graph.cursor.bounds",
            "pgorm::Cursor::{before,after,before_with,after_with,first,last,asc,desc}",
        ),
        ("graph.cursor.all", "pgorm::Cursor::all"),
        ("graph.cursor.inspect", "pgorm::Cursor::statement"),
        ("graph.stream", "pgorm::SelectGraph<E, S>::stream"),
        (
            "graph.all_grouped",
            "pgorm::SelectGraph<E, (Opt<F>,)>::all_grouped",
        ),
        ("graph.count", "pgorm::PaginatorTrait::count"),
        (
            "graph.paginate",
            "pgorm::PaginatorTrait::paginate, Paginator::{fetch_page,num_items_and_pages}",
        ),
    ]
    .into_iter()
    .map(|(name, api)| {
        (
            name.to_owned(),
            serde_json::json!({"rust_api":api, "features":[], "registration_required":true}),
        )
    })
    .collect()
}
