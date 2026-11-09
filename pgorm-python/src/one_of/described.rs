//! A one-of reference's shape, described from the models Python declares.

use std::sync::Arc;

use pgorm_bindings::entity::{DescribedTarget, OneOfCore};
use pyo3::prelude::*;

use super::NativeOneOf;
use crate::{disjoint::table_of, errors::Raise};

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
    let targets = targets
        .into_iter()
        .map(|(value, field, column, (model, table))| {
            Ok(DescribedTarget {
                value,
                field,
                column,
                model,
                table: table_of(&table)?,
            })
        })
        .collect::<PyResult<Vec<_>>>()?;
    let core = OneOfCore::model(name, (model_name, entity), targets, optional).py()?;
    Ok(NativeOneOf {
        core: Arc::new(core),
    })
}
