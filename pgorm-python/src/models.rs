//! The native half of the models Python declares.
//!
//! A model is Python data lowered into the statement builders. What it needs
//! from Rust is small: the name each column of a decoded source is projected
//! and found under, composed by pgorm's own `result_column_name` so a model
//! graph's aliases are the ones pgorm's graph writer mints, and the statement
//! counting a query's rows.

use pgorm::pgorm_query::{Asterisk, Expr, Func, Name, Query};
use pyo3::prelude::*;

use crate::statements::PySelect;

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(result_names, module)?)?;
    module.add_function(wrap_pyfunction!(count, module)?)?;
    Ok(())
}

/// The result-set name of each of `columns` under `prefix`, the plain
/// concatenation below PostgreSQL's identifier bound and the bounded spelling
/// with its hash past it.
// [spec:pgorm:req:bindings.graphs+2]
#[pyfunction(name = "_model_result_names")]
fn result_names(prefix: &str, columns: Vec<String>) -> Vec<String> {
    columns
        .iter()
        .map(|column| pgorm::result_column_name(prefix, column))
        .collect()
}

/// The statement counting the rows `select` reads: its limit, offset and
/// ordering dropped, and the rest read as a subquery,
/// `SELECT COUNT(*) AS "num_items" FROM (..) AS "sub_query"`.
// [spec:pgorm:req:bindings.pagination]
#[pyfunction(name = "_model_count")]
fn count(select: PyRef<'_, PySelect>) -> PyResult<PySelect> {
    let mut query = select.embedded()?;
    query.reset_limit().reset_offset().clear_order_by();
    Ok(PySelect {
        inner: Query::select()
            .expr_as(Func::count(Expr::col(Asterisk)), Name::runtime("num_items"))
            .from_subquery(query, Name::runtime("sub_query"))
            .to_owned(),
    })
}
