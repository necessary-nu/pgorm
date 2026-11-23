//! Views, recursive views and materialized views over a `Select`, the
//! refresh and drops a materialized view takes, and `CREATE TABLE AS`. Each
//! body is the `Select` builder's statement, written by pgorm-query with its
//! values inlined: a definition is stored as SQL text and binds nothing.
//!
//! Nothing can order a query or declare its windows once it is a definition,
//! so a tie limit or a WINDOW clause PostgreSQL would refuse is refused as
//! the statement is built. The Rust constructor runs the two checks
//! `PySelect::embedded` runs, in the same order and with the same errors, on
//! the query it takes, so the query is handed over unchecked and the
//! constructor's refusal is raised as the `ConstructionError` an embedded
//! read would raise.

use super::{
    args::{self, columns, pick, relation, relations},
    drops::drop_list,
    statement::{PyDDL, Statement, ddl_class},
};
use crate::{errors::ConstructionError, statements::PySelect};
use pgorm::pgorm_query::{
    MaterializedView, MaterializedViewCreateStatement, RecursiveViewCreateStatement, Table,
    TableCreateAsStatement, View, ViewCheckOption, ViewCreateStatement, error::Result,
};
use pyo3::prelude::*;

/// The statement `built` gives, or the `ConstructionError` its refusal is.
fn construct<T>(built: Result<T>) -> PyResult<T> {
    built.map_err(|error| ConstructionError::new_err(error.to_string()))
}

/// The options a plain view and a recursive one share, each written only
/// when set.
macro_rules! view_options {
    ($class:ident) => {
        #[pymethods]
        impl $class {
            /// `OR REPLACE`: the new query keeps the view's columns in place
            /// and may add columns after them.
            fn or_replace<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
                let mut view = Self::statement(slf)?;
                view.or_replace();
                Self::object(slf.py(), view)
            }

            /// Names appended to the column list, renaming the query's leading
            /// columns.
            // [spec:pgorm:req:bindings.schema-views+3]
            fn columns<'py>(
                slf: &Bound<'py, Self>,
                names: &Bound<'_, PyAny>,
            ) -> PyResult<Bound<'py, Self>> {
                let names = args::names(names, "columns")?;
                let mut view = Self::statement(slf)?;
                view.columns(names);
                Self::object(slf.py(), view)
            }

            fn security_barrier<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
                let mut view = Self::statement(slf)?;
                view.security_barrier();
                Self::object(slf.py(), view)
            }

            fn security_invoker<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
                let mut view = Self::statement(slf)?;
                view.security_invoker();
                Self::object(slf.py(), view)
            }
        }
    };
}

/// The clauses a materialized view and `CREATE TABLE AS` share.
macro_rules! relation_options {
    ($class:ident) => {
        #[pymethods]
        impl $class {
            /// `IF NOT EXISTS`: an existing relation of the name is left
            /// alone, its rows with it.
            fn if_not_exists<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
                let mut create = Self::statement(slf)?;
                create.if_not_exists();
                Self::object(slf.py(), create)
            }

            /// Names appended to the column list, renaming the query's leading
            /// columns.
            fn columns<'py>(
                slf: &Bound<'py, Self>,
                names: &Bound<'_, PyAny>,
            ) -> PyResult<Bound<'py, Self>> {
                let names = args::names(names, "columns")?;
                let mut create = Self::statement(slf)?;
                create.columns(names);
                Self::object(slf.py(), create)
            }

            /// `WITH NO DATA`: the query's columns and none of its rows.
            fn with_no_data<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
                let mut create = Self::statement(slf)?;
                create.with_no_data();
                Self::object(slf.py(), create)
            }
        }
    };
}

/// `CREATE VIEW`; every method gives a new statement.
#[derive(Debug)]
#[pyclass(name = "CreateView", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyCreateView;

ddl_class!(PyCreateView, CreateView, Box<ViewCreateStatement>);

// [spec:pgorm:req:bindings.schema-views+3]
#[pyfunction]
pub(super) fn create_view<'py>(
    py: Python<'py>,
    name: &Bound<'_, PyAny>,
    query: PyRef<'_, PySelect>,
) -> PyResult<Bound<'py, PyCreateView>> {
    let view = construct(View::create(relation(name)?, query.inner.clone()))?;
    PyCreateView::object(py, Box::new(view))
}

#[pymethods]
impl PyCreateView {
    /// `WITH LOCAL CHECK OPTION` or `WITH CASCADED CHECK OPTION`: a row
    /// written through the view must be one it shows. The bare form means
    /// `CASCADED`, so it has no spelling of its own.
    // [spec:pgorm:req:bindings.schema-views+3]
    fn check_option<'py>(slf: &Bound<'py, Self>, option: &str) -> PyResult<Bound<'py, Self>> {
        let option = pick(
            option,
            "a check option",
            &[
                ("local", ViewCheckOption::Local),
                ("cascaded", ViewCheckOption::Cascaded),
            ],
        )?;
        let mut view = Self::statement(slf)?;
        view.check_option(option);
        Self::object(slf.py(), view)
    }
}

view_options!(PyCreateView);

/// `CREATE RECURSIVE VIEW`, whose query reads the view by name. It has no
/// check option, which PostgreSQL refuses on a recursive view.
#[derive(Debug)]
#[pyclass(name = "CreateRecursiveView", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyCreateRecursiveView;

ddl_class!(
    PyCreateRecursiveView,
    CreateRecursiveView,
    Box<RecursiveViewCreateStatement>
);

/// The column list is what the recursive query's rows are called, so it is
/// one name or a non-empty list: the grammar refuses a recursive view without
/// one.
// [spec:pgorm:req:bindings.schema-views+3]
#[pyfunction]
pub(super) fn create_recursive_view<'py>(
    py: Python<'py>,
    name: &Bound<'_, PyAny>,
    columns: &Bound<'_, PyAny>,
    query: PyRef<'_, PySelect>,
) -> PyResult<Bound<'py, PyCreateRecursiveView>> {
    let name = relation(name)?;
    let (first, rest) = self::columns(columns, "a recursive view's column list")?;
    let mut view = construct(View::create_recursive(name, first, query.inner.clone()))?;
    view.columns(rest);
    PyCreateRecursiveView::object(py, Box::new(view))
}

view_options!(PyCreateRecursiveView);

/// `CREATE MATERIALIZED VIEW`: the rows the query returned, held until a
/// refresh.
#[derive(Debug)]
#[pyclass(name = "CreateMaterializedView", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyCreateMaterializedView;

ddl_class!(
    PyCreateMaterializedView,
    CreateMaterializedView,
    Box<MaterializedViewCreateStatement>
);

// [spec:pgorm:req:bindings.schema-views+3]
#[pyfunction]
pub(super) fn create_materialized_view<'py>(
    py: Python<'py>,
    name: &Bound<'_, PyAny>,
    query: PyRef<'_, PySelect>,
) -> PyResult<Bound<'py, PyCreateMaterializedView>> {
    let create = construct(MaterializedView::create(
        relation(name)?,
        query.inner.clone(),
    ))?;
    PyCreateMaterializedView::object(py, Box::new(create))
}

relation_options!(PyCreateMaterializedView);

/// `CREATE TABLE AS`: a table of the query's columns and rows, tied to it no
/// further.
#[derive(Debug)]
#[pyclass(name = "CreateTableAs", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyCreateTableAs;

ddl_class!(PyCreateTableAs, CreateTableAs, Box<TableCreateAsStatement>);

// [spec:pgorm:req:bindings.schema-views+3]
#[pyfunction]
pub(super) fn create_table_as<'py>(
    py: Python<'py>,
    name: &Bound<'_, PyAny>,
    query: PyRef<'_, PySelect>,
) -> PyResult<Bound<'py, PyCreateTableAs>> {
    let create = construct(Table::create_as(relation(name)?, query.inner.clone()))?;
    PyCreateTableAs::object(py, Box::new(create))
}

relation_options!(PyCreateTableAs);

/// `DROP VIEW` over one view or a non-empty list of them.
// [spec:pgorm:req:bindings.schema-views+3]
#[pyfunction]
#[pyo3(signature=(names, *, if_exists=false, behavior=None))]
pub(super) fn drop_view(
    names: &Bound<'_, PyAny>,
    if_exists: bool,
    behavior: Option<&str>,
) -> PyResult<PyDDL> {
    let drop = drop_list(relations(names)?, if_exists, behavior)?;
    Ok(PyDDL::new(Statement::DropView(drop)))
}

/// `REFRESH MATERIALIZED VIEW`. PostgreSQL refuses `CONCURRENTLY` beside
/// `WITH NO DATA`, and the Rust statement holds the two in one slot, so both
/// together are refused here rather than one silently kept.
// [spec:pgorm:req:bindings.schema-views+3]
#[pyfunction]
#[pyo3(signature=(name, *, concurrently=false, with_no_data=false))]
pub(super) fn refresh_materialized_view(
    name: &Bound<'_, PyAny>,
    concurrently: bool,
    with_no_data: bool,
) -> PyResult<PyDDL> {
    let mut refresh = MaterializedView::refresh(relation(name)?);
    match (concurrently, with_no_data) {
        (true, true) => {
            return Err(ConstructionError::new_err(
                "a refresh is concurrent or with no data, never both",
            ));
        }
        (true, false) => {
            refresh.concurrently();
        }
        (false, true) => {
            refresh.with_no_data();
        }
        (false, false) => {}
    }
    Ok(PyDDL::new(Statement::RefreshMaterializedView(refresh)))
}

/// `DROP MATERIALIZED VIEW` over one view or a non-empty list of them.
// [spec:pgorm:req:bindings.schema-views+3]
#[pyfunction]
#[pyo3(signature=(names, *, if_exists=false, behavior=None))]
pub(super) fn drop_materialized_view(
    names: &Bound<'_, PyAny>,
    if_exists: bool,
    behavior: Option<&str>,
) -> PyResult<PyDDL> {
    let drop = drop_list(relations(names)?, if_exists, behavior)?;
    Ok(PyDDL::new(Statement::DropMaterializedView(drop)))
}
