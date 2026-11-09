//! One-of references over pgorm's [`OneOfShape`]: a reference declared over
//! models Python describes, or a downstream crate's registered
//! `#[derive(OneOf)]` enum.
//!
//! Either is one native `_OneOf`, the shape and its description. Every
//! decision about a reference is the shape's — which target a row
//! references, which columns a write sets, the condition a target's rows
//! are loaded by, the error for a key its target does not hold, and the
//! `CHECK` — so a model's reference is the Rust declaration's; Python only
//! reads the records' fields and runs the reads.

mod described;
mod registered;

use std::{fmt, sync::Arc};

use pgorm::{DdlStatement, pgorm_query::Value};
use pgorm_bindings::entity::OneOfCore;
use pyo3::prelude::*;

use crate::{
    errors::{Raise, raised},
    expressions::PyExpr,
    values::PyValue,
};

pub(crate) use registered::{one_of_entities, one_of_entity};

/// A one-of reference or a registered one-of enum, as Python's `OneOf` and
/// `OneOfEntity` hold it.
// [spec:pgorm:req:bindings.one-of+1]
#[pyclass(name = "_OneOf", module = "pgorm._native", frozen)]
pub struct NativeOneOf {
    pub(crate) core: Arc<OneOfCore>,
}

impl fmt::Debug for NativeOneOf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeOneOf")
            .field("description", &self.core.description)
            .finish()
    }
}

/// The values `values` hold, in order.
fn values(values: &[PyRef<'_, PyValue>]) -> Vec<Value> {
    values.iter().map(|value| value.inner.clone()).collect()
}

#[pymethods]
impl NativeOneOf {
    /// The reference or registration as data, under the keys `describe()`
    /// reports.
    // [spec:pgorm:req:bindings.one-of+1]
    fn describe<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        py.import("json")?
            .call_method1("loads", (self.core.description.to_string(),))
    }

    /// Where the target `value` stands among the reference's.
    // [spec:pgorm:req:bindings.one-of+1]
    fn target(&self, value: &str) -> PyResult<usize> {
        self.core.target(value).py()
    }

    /// The target a row references, given its key and, for each target,
    /// whether its column is set, or `None` for a row referencing none.
    // [spec:pgorm:req:bindings.one-of+1]
    fn referenced(&self, row: Vec<PyRef<'_, PyValue>>, set: Vec<bool>) -> PyResult<Option<usize>> {
        self.core.referenced(&values(&row), &set).py()
    }

    /// For each target, whether a write of a reference to `target` sets its
    /// column to the key, the rest null.
    // [spec:pgorm:req:bindings.one-of+1]
    #[pyo3(signature = (target=None))]
    fn assigned(&self, target: Option<usize>) -> PyResult<Vec<bool>> {
        self.core.assigned(target).py()
    }

    /// The condition the rows of the target at `target` are loaded by.
    // [spec:pgorm:req:bindings.one-of+1]
    fn load_condition(&self, target: usize, keys: Vec<PyRef<'_, PyValue>>) -> PyResult<PyExpr> {
        self.core
            .load_condition(target, values(&keys))
            .map(PyExpr::from_rust)
            .py()
    }

    /// The `DecodeError` for the row of key `row` referencing a row of the
    /// target at `target`, of key `key`, which its table does not hold.
    // [spec:pgorm:req:bindings.one-of+1]
    fn absent(
        &self,
        target: usize,
        row: Vec<PyRef<'_, PyValue>>,
        key: PyRef<'_, PyValue>,
    ) -> PyErr {
        raised(self.core.absent(target, &values(&row), &key.inner))
    }

    /// The `ALTER TABLE` adding the reference's `CHECK`.
    // [spec:pgorm:req:bindings.one-of+1]
    fn statement<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        crate::schema::ddl_object(py, DdlStatement::AlterTable(self.core.check()))
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeOneOf>()?;
    module.add_function(wrap_pyfunction!(described::one_of_model, module)?)?;
    module.add_function(wrap_pyfunction!(one_of_entity, module)?)?;
    module.add_function(wrap_pyfunction!(one_of_entities, module)?)?;
    Ok(())
}

/// Each one-of operation beside the Rust API that answers it; only
/// `one_of_entity` needs a registration.
pub(crate) const OPERATIONS: [(&str, &str); 6] = [
    ("one_of.declare", "pgorm::OneOfShape::new"),
    ("one_of.key", "pgorm::OneOfShape::referenced"),
    ("one_of.set", "pgorm::OneOfShape::assigned"),
    ("one_of.load", "pgorm::OneOfShape::{load_condition, absent}"),
    ("one_of.schema", "pgorm::OneOfShape::check"),
    (
        "one_of_entity",
        "pgorm_python::entities::Registry::one_of / pgorm::OneOfShape::of",
    ),
];
