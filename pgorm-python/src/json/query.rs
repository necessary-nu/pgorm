//! `JSON_EXISTS`, `JSON_VALUE` and `JSON_QUERY`. The path is a Python `str`
//! the Rust builder binds as text cast to `jsonpath`; it never reaches the
//! statement's text.

use pgorm::pgorm_query::{Func, JsonValueType};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyDict};

use super::{
    column_type, input,
    kinds::{PyJsonExistsBehavior, query_behavior, value_behavior},
    variables,
};
use crate::{errors::ConstructionError, expressions::PyExpr};

// [spec:pgorm:req:bindings.sql-json]
#[pyfunction]
#[pyo3(signature = (context, path, *, passing=None, on_error=None))]
pub(crate) fn json_exists(
    context: &Bound<'_, PyAny>,
    path: String,
    passing: Option<&Bound<'_, PyDict>>,
    on_error: Option<PyJsonExistsBehavior>,
) -> PyResult<PyExpr> {
    let mut json = Func::json_exists(input(context)?, path);
    for (value, name) in variables(passing)? {
        json = json.passing(value, name);
    }
    if let Some(behavior) = on_error {
        json = json.on_error(behavior.rust());
    }
    Ok(PyExpr::from_rust(json.into()))
}

#[pyfunction]
#[pyo3(signature = (context, path, *, passing=None, returning=None, on_empty=None, on_error=None))]
pub(crate) fn json_value(
    context: &Bound<'_, PyAny>,
    path: String,
    passing: Option<&Bound<'_, PyDict>>,
    returning: Option<&Bound<'_, PyAny>>,
    on_empty: Option<&Bound<'_, PyAny>>,
    on_error: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyExpr> {
    let mut json = Func::json_value(input(context)?, path);
    for (value, name) in variables(passing)? {
        json = json.passing(value, name);
    }
    if let Some(ty) = column_type(returning)? {
        let ty = JsonValueType::try_from(ty).map_err(|_| {
            ConstructionError::new_err(
                "JSON_VALUE cannot return json or jsonb, which were left out for PostgreSQL \
                 18's bug #19695; use json_query",
            )
        })?;
        json = json.returning(ty);
    }
    if let Some(behavior) = on_empty {
        json = json.on_empty(value_behavior(behavior)?);
    }
    if let Some(behavior) = on_error {
        json = json.on_error(value_behavior(behavior)?);
    }
    Ok(PyExpr::from_rust(json.into()))
}

#[pyfunction]
#[pyo3(signature = (
    context, path, *, passing=None, returning=None, shaping=None, on_empty=None, on_error=None
))]
pub(crate) fn json_query(
    context: &Bound<'_, PyAny>,
    path: String,
    passing: Option<&Bound<'_, PyDict>>,
    returning: Option<&Bound<'_, PyAny>>,
    shaping: Option<&str>,
    on_empty: Option<&Bound<'_, PyAny>>,
    on_error: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyExpr> {
    let mut json = Func::json_query(input(context)?, path);
    for (value, name) in variables(passing)? {
        json = json.passing(value, name);
    }
    if let Some(ty) = column_type(returning)? {
        json = json.returning(ty);
    }
    json = match shaping {
        None => json,
        Some("with_wrapper") => json.with_wrapper(),
        Some("with_conditional_wrapper") => json.with_conditional_wrapper(),
        Some("omit_quotes") => json.omit_quotes(),
        Some(_) => {
            return Err(PyTypeError::new_err(
                "shaping requires 'with_wrapper', 'with_conditional_wrapper' or 'omit_quotes'",
            ));
        }
    };
    if let Some(behavior) = on_empty {
        json = json.on_empty(query_behavior(behavior)?);
    }
    if let Some(behavior) = on_error {
        json = json.on_error(query_behavior(behavior)?);
    }
    Ok(PyExpr::from_rust(json.into()))
}
