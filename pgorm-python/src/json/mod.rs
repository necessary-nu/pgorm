//! SQL/JSON's query functions, constructors, `IS JSON` and `JSON_TABLE` over
//! the Rust builders in `pgorm_query`. Each function takes its clauses as keyword
//! arguments, applies them through the builder's own methods and returns the
//! finished expression or FROM item, so the SQL is the Rust renderer's.

mod capabilities;
mod construct;
mod kinds;
mod query;
mod table;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod variants;

use pgorm::pgorm_query::{ColumnType, JsonInput, Name};
use pyo3::{prelude::*, types::PyDict};

use crate::{expressions::coerce, identifiers::Identifier, schema::PyDataType};
pub(crate) use capabilities::operations as capabilities;
pub use kinds::{
    PyJsonDefault, PyJsonExistsBehavior, PyJsonInput, PyJsonKind, PyJsonQueryBehavior,
    PyJsonValueBehavior,
};
pub use table::{PyJsonTableBehavior, PyJsonTableColumn};

/// An expression in a position SQL/JSON reads as JSON: a `JsonInput` from
/// `format_json`, or anything an expression accepts, unformatted.
fn input(value: &Bound<'_, PyAny>) -> PyResult<JsonInput> {
    if let Ok(input) = value.extract::<PyRef<'_, PyJsonInput>>() {
        Ok(input.inner.clone())
    } else {
        Ok(coerce(value)?.inner.into())
    }
}

/// The `RETURNING` type: a `DataType` or one of its built-in names.
fn column_type(value: Option<&Bound<'_, PyAny>>) -> PyResult<Option<ColumnType>> {
    value
        .map(|value| PyDataType::coerce(value).map(|kind| kind.inner))
        .transpose()
}

/// `PASSING` variables in the mapping's order, each name an identifier.
fn variables(passing: Option<&Bound<'_, PyDict>>) -> PyResult<Vec<(JsonInput, Name)>> {
    let Some(passing) = passing else {
        return Ok(Vec::new());
    };
    passing
        .iter()
        .map(|(name, value)| Ok((input(&value)?, Identifier::new(&name)?.name())))
        .collect()
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyJsonInput>()?;
    module.add_class::<PyJsonKind>()?;
    module.add_class::<PyJsonExistsBehavior>()?;
    module.add_class::<PyJsonValueBehavior>()?;
    module.add_class::<PyJsonQueryBehavior>()?;
    module.add_class::<PyJsonDefault>()?;
    module.add_class::<PyJsonTableBehavior>()?;
    module.add_class::<PyJsonTableColumn>()?;
    module.add_function(wrap_pyfunction!(query::json_exists, module)?)?;
    module.add_function(wrap_pyfunction!(query::json_value, module)?)?;
    module.add_function(wrap_pyfunction!(query::json_query, module)?)?;
    module.add_function(wrap_pyfunction!(table::json_table, module)?)?;
    module.add_function(wrap_pyfunction!(construct::json_object, module)?)?;
    module.add_function(wrap_pyfunction!(construct::json_array, module)?)?;
    module.add_function(wrap_pyfunction!(construct::json_array_query, module)?)?;
    module.add_function(wrap_pyfunction!(construct::json_objectagg, module)?)?;
    module.add_function(wrap_pyfunction!(construct::json_arrayagg, module)?)?;
    module.add_function(wrap_pyfunction!(construct::json_parse, module)?)?;
    module.add_function(wrap_pyfunction!(construct::json_scalar, module)?)?;
    module.add_function(wrap_pyfunction!(construct::json_serialize, module)?)?;
    module.add_function(wrap_pyfunction!(kinds::format_json, module)?)?;
    module.add_function(wrap_pyfunction!(kinds::is_json, module)?)?;
    module.add_function(wrap_pyfunction!(kinds::is_not_json, module)?)?;
    Ok(())
}
