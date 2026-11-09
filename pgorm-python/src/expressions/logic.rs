//! `Expr`'s logic: `&`, `|` and `~` as SQL's AND, OR and NOT, and the
//! refusal of Python truth testing.

use pgorm::pgorm_query::ExprTrait;
use pyo3::prelude::*;

use super::{PyExpr, require_expr};
use crate::errors::ConstructionError;

// [spec:pgorm:req:bindings.expressions+4]
// [spec:pgorm:req:python.naming+4]
#[pymethods]
impl PyExpr {
    fn __and__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_rust(
            self.inner.clone().and(require_expr(other)?.inner),
        ))
    }
    fn __or__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_rust(
            self.inner.clone().or(require_expr(other)?.inner),
        ))
    }
    fn __invert__(&self) -> Self {
        Self::from_rust(self.inner.clone().not())
    }
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL expressions cannot be tested as Python booleans; use &, | and ~",
        ))
    }
}
