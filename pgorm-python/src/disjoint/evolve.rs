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

use crate::{errors::ConstructionError, schema};

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

/// The steps of the change `change` names, each `(statements, gap)`, the
/// gap `None` or `(kind, schema, table)`.
// [spec:pgorm:req:bindings.disjoint-evolve]
pub(super) fn change<'py>(
    py: Python<'py>,
    shape: &DisjointShape,
    change: &str,
    value: &str,
    table: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyList>> {
    let table = table.map(schema::relation).transpose()?;
    let steps = written(shape, change, value, table)?
        .into_steps()
        .into_iter()
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
    PyList::new(py, steps)
}
