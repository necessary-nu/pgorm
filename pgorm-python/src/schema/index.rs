//! `CREATE INDEX` and `DROP INDEX`. An index's entries are columns or
//! expressions, each with an operator class and an order; the index takes
//! uniqueness, `NULLS NOT DISTINCT`, an access method, `INCLUDE`d columns and
//! a partial-index predicate. `CONCURRENTLY` is not offered: PostgreSQL
//! refuses it inside a transaction, where DDL runs as often as not.

use super::{
    args::{self, choice, relation},
    statement::{PyDDL, Statement, ddl_class},
};
use crate::{expressions::PyExpr, statements::condition};
use pgorm::pgorm_query::{Index, IndexColumn, IndexCreateStatement, IndexOrder, IndexType};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyString};

/// An index entry: a column's name or an expression, with the order and
/// operator class given.
// [spec:pgorm:req:bindings.schema-indexes]
fn entry(
    on: &Bound<'_, PyAny>,
    order: Option<&str>,
    operator_class: Option<&Bound<'_, PyAny>>,
) -> PyResult<IndexColumn> {
    let mut column = if let Ok(expr) = on.extract::<PyRef<'_, PyExpr>>() {
        IndexColumn::expr(expr.inner.clone())
    } else if on.is_instance_of::<PyString>() {
        IndexColumn::name(args::name(on)?)
    } else {
        return Err(PyTypeError::new_err(
            "an index entry is a column's name or an expression",
        ));
    };
    if let Some(class) = operator_class {
        column = column.operator_class(args::name(class)?);
    }
    let order = choice(
        order,
        "an index entry's order",
        &[("asc", IndexOrder::Asc), ("desc", IndexOrder::Desc)],
    )?;
    if let Some(order) = order {
        column = column.order(order);
    }
    Ok(column)
}

/// `CREATE INDEX`; every method gives a new statement.
#[derive(Debug)]
#[pyclass(name = "CreateIndex", module = "pgorm.schema", frozen, extends = PyDDL)]
pub struct PyCreateIndex;

ddl_class!(PyCreateIndex, CreateIndex, IndexCreateStatement, with);

/// An index over its first entry, named as asked and by PostgreSQL
/// otherwise.
// [spec:pgorm:req:bindings.schema-indexes]
#[pyfunction]
#[pyo3(signature=(table, on, *, name=None, order=None, operator_class=None))]
pub(super) fn create_index<'py>(
    py: Python<'py>,
    table: &Bound<'_, PyAny>,
    on: &Bound<'_, PyAny>,
    name: Option<&Bound<'_, PyAny>>,
    order: Option<&str>,
    operator_class: Option<&Bound<'_, PyAny>>,
) -> PyResult<Bound<'py, PyCreateIndex>> {
    let mut created = Index::create(relation(table)?, entry(on, order, operator_class)?);
    if let Some(name) = name {
        created.name(args::name(name)?);
    }
    PyCreateIndex::object(py, created)
}

#[pymethods]
impl PyCreateIndex {
    #[pyo3(signature=(on, *, order=None, operator_class=None))]
    fn column<'py>(
        slf: &Bound<'py, Self>,
        on: &Bound<'_, PyAny>,
        order: Option<&str>,
        operator_class: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, Self>> {
        let column = entry(on, order, operator_class)?;
        Self::with(slf, |statement| {
            statement.col(column);
        })
    }

    fn unique<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let mut statement = Self::statement(slf)?;
        statement.unique();
        Self::object(slf.py(), statement)
    }

    /// `NULLS NOT DISTINCT`, which PostgreSQL defines for a unique index
    /// alone, and so makes the index unique rather than be written for
    /// nothing.
    // [spec:pgorm:req:bindings.schema-indexes]
    fn nulls_not_distinct<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let mut statement = Self::statement(slf)?;
        statement.unique().nulls_not_distinct();
        Self::object(slf.py(), statement)
    }

    fn if_not_exists<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let mut statement = Self::statement(slf)?;
        statement.if_not_exists();
        Self::object(slf.py(), statement)
    }

    /// The access method, any the server has — `btree`, `hash` and `gin` by
    /// pgorm-query's own variants, another by its quoted name.
    // [spec:pgorm:req:bindings.schema-indexes]
    fn using<'py>(slf: &Bound<'py, Self>, method: &Bound<'_, PyAny>) -> PyResult<Bound<'py, Self>> {
        let name = args::name(method)?;
        let method = match method.extract::<&str>() {
            Ok("btree") => IndexType::BTree,
            Ok("hash") => IndexType::Hash,
            Ok("gin") => IndexType::Gin,
            _ => IndexType::Named(name),
        };
        Self::with(slf, |statement| {
            statement.index_type(method);
        })
    }

    /// Columns carried in the index's leaves without being indexed; appends.
    fn include<'py>(
        slf: &Bound<'py, Self>,
        columns: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let columns = args::names(columns, "an index's include list")?;
        Self::with(slf, |statement| {
            statement.include(columns);
        })
    }

    /// The partial index's predicate, ANDed to one already there.
    // [spec:pgorm:req:bindings.schema-indexes]
    #[pyo3(name = "where")]
    fn where_<'py>(
        slf: &Bound<'py, Self>,
        predicate: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let predicate = condition(predicate)?;
        Self::with(slf, |statement| {
            statement.cond_where(predicate);
        })
    }
}

/// `DROP INDEX`, the index in its table's schema.
// [spec:pgorm:req:bindings.schema-indexes]
#[pyfunction]
#[pyo3(signature=(table, name, *, if_exists=false))]
pub(super) fn drop_index(
    table: &Bound<'_, PyAny>,
    name: &Bound<'_, PyAny>,
    if_exists: bool,
) -> PyResult<PyDDL> {
    let mut drop = Index::drop(args::name(name)?);
    drop.table(relation(table)?);
    if if_exists {
        drop.if_exists();
    }
    Ok(PyDDL::new(Statement::DropIndex(drop)))
}
