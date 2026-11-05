use pgorm::pgorm_query::{ColumnRef, Expr, FromItem};
use pyo3::{exceptions::PyTypeError, prelude::*};

use super::table::PyTable;
use crate::{errors::ConstructionError, expressions::PyExpr, identifiers::PyIdentifier};

// [spec:pgorm:req:bindings.select+4]
/// A FROM item that is not a plain named table, owned by Rust's `FromItem`.
/// A value-producing item is always aliased, as PostgreSQL requires, so its
/// columns are qualified by a name the caller chose; a sampled table is
/// qualified by its alias, or by its unqualified name when it has none.
/// `json_table`, `TableFunction.alias` and `Table.tablesample` build one.
#[pyclass(name = "FromItem", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyFromItem {
    pub(crate) inner: FromItem,
}

#[pymethods]
impl PyFromItem {
    fn col(&self, name: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        Ok(PyExpr::from_rust(
            Expr::col(ColumnRef::TableColumn(
                self.inner.qualifier().clone(),
                PyIdentifier::new(name)?.name(),
            ))
            .into(),
        ))
    }

    fn star(&self) -> PyExpr {
        PyExpr::from_rust(
            Expr::col(ColumnRef::TableAsterisk(self.inner.qualifier().clone())).into(),
        )
    }

    #[getter]
    fn alias(&self) -> String {
        self.inner.qualifier().to_string()
    }

    fn __repr__(&self) -> String {
        format!("FromItem(alias={:?})", self.alias())
    }

    // [spec:pgorm:req:python.truth-testing+3]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "FROM items cannot be tested as Python booleans",
        ))
    }
}

/// The FROM item a `Table` or `FromItem` argument stands for.
pub(super) fn from_item(value: &Bound<'_, PyAny>) -> PyResult<FromItem> {
    if let Ok(table) = value.extract::<PyRef<'_, PyTable>>() {
        Ok(FromItem::Table(table.inner.clone()))
    } else if let Ok(item) = value.extract::<PyRef<'_, PyFromItem>>() {
        Ok(item.inner.clone())
    } else {
        Err(PyTypeError::new_err(
            "a FROM item requires a Table or FromItem",
        ))
    }
}
