use pgorm::pgorm_query::{ColumnRef, NamedTable, TableName};
use pyo3::prelude::*;

use super::{
    from_item::PyFromItem,
    table_sample::{PySampleMethod, sample},
};
use crate::{errors::ConstructionError, expressions::PyExpr, identifiers::PyIdentifier};

/// A runtime table name and optional alias, owned by Rust's NamedTable.
#[pyclass(name = "Table", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyTable {
    pub inner: NamedTable,
}

#[pymethods]
impl PyTable {
    #[new]
    #[pyo3(signature = (name, *, schema=None, alias=None))]
    fn new(
        name: &Bound<'_, PyAny>,
        schema: Option<&Bound<'_, PyAny>>,
        alias: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let name = PyIdentifier::new(name)?.name();
        let name = match schema {
            Some(schema) => TableName::SchemaTable(PyIdentifier::new(schema)?.name(), name),
            None => TableName::Table(name),
        };
        let mut inner = NamedTable::from(name);
        if let Some(alias) = alias {
            inner = inner.alias(PyIdentifier::new(alias)?.name());
        }
        Ok(Self { inner })
    }

    fn as_(&self, alias: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.clone().alias(PyIdentifier::new(alias)?.name()),
        })
    }

    fn col(&self, name: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        let name = PyIdentifier::new(name)?.name();
        let column = match (&self.inner.alias, &self.inner.name) {
            (Some(alias), _) => ColumnRef::TableColumn(alias.clone(), name),
            (None, TableName::Table(table)) => ColumnRef::TableColumn(table.clone(), name),
            (None, TableName::SchemaTable(schema, table)) => {
                ColumnRef::SchemaTableColumn(schema.clone(), table.clone(), name)
            }
        };
        Ok(PyExpr::from_rust(
            pgorm::pgorm_query::Expr::col(column).into(),
        ))
    }

    fn star(&self) -> PyExpr {
        PyExpr::from_rust(
            pgorm::pgorm_query::Expr::col(ColumnRef::TableAsterisk(self.inner.qualifier().clone()))
                .into(),
        )
    }

    // [spec:pgorm:req:bindings.select+4]
    /// This table read through `TABLESAMPLE`, a `FromItem` qualified by the
    /// table's alias, or by its unqualified name: `method` draws each row
    /// (or page) with probability `percentage` out of 100, and `repeatable`
    /// seeds the draw.
    #[pyo3(signature = (method, percentage, *, repeatable=None))]
    fn tablesample(
        &self,
        method: PySampleMethod,
        percentage: &Bound<'_, PyAny>,
        repeatable: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyFromItem> {
        Ok(PyFromItem {
            inner: sample(&self.inner, &method, percentage, repeatable)?,
        })
    }

    #[getter]
    fn name(&self) -> String {
        self.inner.name.table().to_string()
    }
    #[getter]
    fn schema(&self) -> Option<String> {
        self.inner.name.schema().map(|name| name.to_string())
    }
    #[getter]
    fn alias(&self) -> Option<String> {
        self.inner.alias.as_ref().map(|name| name.to_string())
    }

    fn __repr__(&self) -> String {
        format!(
            "Table({:?}, schema={:?}, alias={:?})",
            self.name(),
            self.schema(),
            self.alias()
        )
    }

    // [spec:pgorm:req:python.truth-testing+3]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL tables cannot be tested as Python booleans",
        ))
    }
}
