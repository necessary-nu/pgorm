//! A one-of reference's shape, described from the models Python declares.

use std::sync::Arc;

use pgorm::OneOfShape;
use pyo3::prelude::*;
use serde_json::json;

use super::{Core, NativeOneOf};
use crate::{disjoint::table_of, errors::ConstructionError};

/// A model's name beside the description of its table.
type Described<'py> = (String, Bound<'py, PyAny>);

/// `_one_of_model(name, model, targets, optional)`: the reference `name` from
/// the table `model` describes to each target in order, its value beside the
/// field holding its key, that field's column, and its model's name and
/// table. What the shape refuses is a `ConstructionError`, raised as the
/// reference is declared.
// [spec:pgorm:req:bindings.one-of+1]
#[pyfunction(name = "_one_of_model")]
pub(crate) fn one_of_model(
    name: &str,
    model: Described<'_>,
    targets: Vec<(String, String, String, Described<'_>)>,
    optional: bool,
) -> PyResult<NativeOneOf> {
    let (model_name, model) = model;
    let entity = table_of(&model)?;
    let mut described = Vec::with_capacity(targets.len());
    let mut tables = Vec::with_capacity(targets.len());
    for (value, field, column, (target, table)) in targets {
        described.push(json!({"value": value, "field": field, "target": target}));
        tables.push((value, column, table_of(&table)?));
    }
    let named: Vec<(&str, &str, &pgorm::DisjointTable)> = tables
        .iter()
        .map(|(value, column, table)| (value.as_str(), column.as_str(), table))
        .collect();
    let shape = OneOfShape::new(name, &entity, &named, optional)
        .map_err(|error| ConstructionError::new_err(error.to_string()))?;
    Ok(NativeOneOf {
        core: Arc::new(Core {
            shape,
            description: json!({
                "name": name,
                "model": model_name,
                "targets": described,
                "optional": optional,
            }),
            rust: None,
        }),
    })
}
