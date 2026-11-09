use super::{
    PyCreateIndex, PyCreateTable, PyCreateType,
    statement::{PyDDL, Statement},
};
use crate::{entities::PyEntity, errors::ConstructionError};
use pgorm::pgorm_query::{
    CommentStatement, IndexCreateStatement, TableCreateStatement, extension::TypeCreateStatement,
};
use pyo3::{exceptions::PyTypeError, prelude::*};

/// A detached result of Rust's Schema methods for a compiled entity: each
/// read gives new statement objects over the statements pgorm wrote.
#[derive(Debug)]
#[pyclass(name = "EntitySchema", module = "pgorm.schema", frozen)]
pub struct PyEntitySchema {
    pub(crate) table: TableCreateStatement,
    pub(crate) enums: Vec<TypeCreateStatement>,
    pub(crate) indexes: Vec<IndexCreateStatement>,
    pub(crate) comments: Vec<CommentStatement>,
}

#[pymethods]
impl PyEntitySchema {
    /// `CREATE TABLE`, with its primary key, unique keys and foreign keys.
    #[getter]
    fn table<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyCreateTable>> {
        PyCreateTable::object(py, self.table.clone())
    }

    /// `CREATE TYPE .. AS ENUM` for each enum type its columns name.
    #[getter]
    fn enums<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyCreateType>>> {
        self.enums
            .iter()
            .map(|statement| PyCreateType::object(py, statement.clone()))
            .collect()
    }

    /// `CREATE INDEX` for each column it indexes.
    #[getter]
    fn indexes<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyCreateIndex>>> {
        self.indexes
            .iter()
            .map(|statement| PyCreateIndex::object(py, statement.clone()))
            .collect()
    }

    /// `COMMENT ON` its table and its columns.
    #[getter]
    fn comments(&self) -> Vec<PyDDL> {
        self.comments
            .iter()
            .map(|statement| PyDDL::new(Statement::Comment(statement.clone())))
            .collect()
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity schemas cannot be tested as Python booleans",
        ))
    }
}

// [spec:pgorm:req:bindings.schema-from-entity]
#[pyfunction]
pub(super) fn schema_from_entity(entity: &Bound<'_, PyAny>) -> PyResult<PyEntitySchema> {
    let entity = entity.extract::<PyRef<'_, PyEntity>>().map_err(|_| {
        PyTypeError::new_err("schema.from_entity requires a registered native Entity; runtime models use explicit DDL builders")
    })?;
    entity
        .backend
        .schema()
        .map(|schema| PyEntitySchema {
            table: schema.table,
            enums: schema.enums,
            indexes: schema.indexes,
            comments: schema.comments,
        })
        .map_err(|error| ConstructionError::new_err(error.to_string()))
}
