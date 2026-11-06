//! A disjoint model's or registration's kind changes, as pgorm's
//! [`DisjointShape`] writes them, each step's statements as Python's DDL
//! classes.

use pgorm::pgorm_query::TableName;
use pgorm::{DdlStatement, DisjointChange, DisjointShape};
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyList, PyTuple},
};

use crate::{
    errors::ConstructionError,
    schema::{self, PyMigrationFile},
};

/// `statements` as Python's DDL classes, in order.
pub(super) fn ddl_tuple(
    py: Python<'_>,
    statements: Vec<DdlStatement>,
) -> PyResult<Bound<'_, PyTuple>> {
    let objects = statements
        .into_iter()
        .map(|statement| schema::ddl_object(py, statement))
        .collect::<PyResult<Vec<_>>>()?;
    PyTuple::new(py, objects)
}

/// The change `change` names, of the kind `value`, which `table` names the
/// table of where the kind loses it: `DisjointShape`'s refusal is a
/// `ConstructionError`.
// [spec:pgorm:req:bindings.disjoint-evolve]
fn written(
    shape: &DisjointShape,
    change: &str,
    value: &str,
    table: Option<TableName>,
) -> PyResult<DisjointChange> {
    let changed = match (change, table) {
        ("add_kind", None) => shape.add_kind(value),
        ("add_kind_recreating", None) => shape.add_kind_recreating(value),
        ("remove_kind", None) => shape.remove_kind(value),
        ("add_kind_table", None) => shape.add_kind_table(value),
        ("remove_kind_table", Some(table)) => shape.remove_kind_table(value, table),
        _ => return Err(PyTypeError::new_err("unknown disjoint kind change")),
    };
    changed.map_err(|error| ConstructionError::new_err(error.to_string()))
}

/// A change as pgorm wrote it, which the Python `DisjointChange` keeps so
/// that `migration_files` freezes it as pgorm does.
// [spec:pgorm:req:bindings.migration-file+1]
#[derive(Debug)]
#[pyclass(name = "_DisjointChange", module = "pgorm._native", frozen)]
pub struct PyDisjointChange {
    change: DisjointChange,
}

#[pymethods]
impl PyDisjointChange {
    /// pgorm's `DisjointChange::migration_files`: a file per step.
    // [spec:pgorm:req:bindings.migration-file+1]
    fn migration_files(&self, name: &str) -> Vec<PyMigrationFile> {
        self.change
            .migration_files(name)
            .into_iter()
            .map(PyMigrationFile::from)
            .collect()
    }
}

/// The steps of the change `change` names, each `(statements, gap)`, the
/// gap `None` or `(kind, schema, table)`, and the change itself.
// [spec:pgorm:req:bindings.disjoint-evolve]
pub(super) fn change<'py>(
    py: Python<'py>,
    shape: &DisjointShape,
    change: &str,
    value: &str,
    table: Option<&Bound<'py, PyAny>>,
) -> PyResult<(Bound<'py, PyList>, PyDisjointChange)> {
    let table = table.map(schema::relation).transpose()?;
    let change = written(shape, change, value, table)?;
    let steps = change
        .steps()
        .iter()
        .map(|step| -> PyResult<Bound<'py, PyTuple>> {
            let gap = step.gap().map(|gap| {
                (
                    gap.kind().to_owned(),
                    gap.table().schema().map(|schema| schema.to_string()),
                    gap.table().table().to_string(),
                )
            });
            let statements = ddl_tuple(py, step.statements().to_vec())?;
            (statements, gap).into_pyobject(py)
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok((PyList::new(py, steps)?, PyDisjointChange { change }))
}
