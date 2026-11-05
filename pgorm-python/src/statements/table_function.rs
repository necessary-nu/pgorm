//! A set-returning function read as a FROM item, over Rust's
//! `TableFunction`.

use pgorm::pgorm_query::{FunctionCall, SimpleExpr, TableFunction};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyTuple};

use super::from_item::PyFromItem;
use crate::{
    errors::ConstructionError, expressions::PyExpr, identifiers::PyIdentifier, schema::PyDataType,
};

/// The call an expression holds; any other expression cannot stand in FROM,
/// and anything but an expression is of another type.
fn function_call(value: &Bound<'_, PyAny>) -> PyResult<FunctionCall> {
    let expr = value.extract::<PyRef<'_, PyExpr>>().map_err(|_| {
        PyTypeError::new_err("a table function requires a function call built by call()")
    })?;
    match &expr.inner {
        SimpleExpr::FunctionCall(call) => Ok(call.clone()),
        _ => Err(ConstructionError::new_err(
            "a table function reads a function call built by call(), not another expression",
        )),
    }
}

// [spec:pgorm:req:bindings.select+3]
/// A function in FROM: one call, or several read side by side under
/// `ROWS FROM (..)`, the shorter padded with NULLs; `with_ordinality` adds a
/// final `bigint` column numbering the rows from 1, `columns` names the
/// columns in the alias, and `column_definition` defines a column of the
/// `record` the call added last returns. `alias` finishes it as the
/// `FromItem` that `from_` and the joins take.
#[pyclass(name = "TableFunction", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyTableFunction {
    inner: TableFunction,
}

#[pymethods]
impl PyTableFunction {
    #[new]
    fn new(call: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: TableFunction::new(function_call(call)?),
        })
    }

    /// `ROWS FROM (f(..))`, to which `and_` adds calls. Over one call it
    /// differs from the constructor only for a function returning a
    /// composite, whose columns `ROWS FROM` expands.
    #[staticmethod]
    fn rows_from(call: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: TableFunction::rows_from(function_call(call)?),
        })
    }

    /// Read another call beside the others; several calls are `ROWS FROM`.
    fn and_(&self, call: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.clone().and(function_call(call)?),
        })
    }

    fn with_ordinality(&self) -> Self {
        Self {
            inner: self.inner.clone().with_ordinality(),
        }
    }

    /// Name the relation's columns in its alias, the ordinal column last,
    /// replacing any names given before.
    #[pyo3(signature = (*names))]
    fn columns(&self, names: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let names = names
            .iter()
            .map(|name| PyIdentifier::new(&name).map(|name| name.name()))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Self {
            inner: self.inner.clone().columns(names),
        })
    }

    /// Define a column, its name and type, of the `record` the call added
    /// last returns, as Rust's `column_definition` does: a lone call writes
    /// the list in its alias, and each call of a `ROWS FROM` carries its own.
    fn column_definition(
        &self,
        name: &Bound<'_, PyAny>,
        data_type: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.clone().column_definition(
                PyIdentifier::new(name)?.name(),
                PyDataType::coerce(data_type)?.inner,
            ),
        })
    }

    fn alias(&self, name: &Bound<'_, PyAny>) -> PyResult<PyFromItem> {
        Ok(PyFromItem {
            inner: self.inner.clone().alias(PyIdentifier::new(name)?.name()),
        })
    }

    fn __repr__(&self) -> &'static str {
        "TableFunction(...)"
    }

    // [spec:pgorm:req:python.truth-testing+3]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "FROM items cannot be tested as Python booleans",
        ))
    }
}
