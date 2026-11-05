//! `Expr`'s NULL tests, membership in a list or a subquery's rows, and
//! `[NOT] BETWEEN [SYMMETRIC]`.

use pgorm::pgorm_query::{Expr, ExprTrait};
use pyo3::prelude::*;

use super::{PyExpr, coerce, construct::sequence};
use crate::statements::subquery;

// [spec:pgorm:req:bindings.expressions+3]
#[pymethods]
impl PyExpr {
    fn is_null(&self) -> Self {
        Self::from_rust(Expr::expr(self.inner.clone()).is_null())
    }
    fn is_not_null(&self) -> Self {
        Self::from_rust(Expr::expr(self.inner.clone()).is_not_null())
    }

    /// `IN`: membership in a list or tuple of operands, Rust's `is_in`, an
    /// empty one holding nothing, or in a `Select`'s rows, Rust's
    /// `in_subquery`, the subquery checked as any embedded one is.
    pub(crate) fn is_in(&self, values: &Bound<'_, PyAny>) -> PyResult<Self> {
        let expr = Expr::expr(self.inner.clone());
        Ok(Self::from_rust(match subquery(values)? {
            Some(select) => expr.in_subquery(select),
            None => expr.is_in(sequence(values)?),
        }))
    }
    pub(crate) fn is_not_in(&self, values: &Bound<'_, PyAny>) -> PyResult<Self> {
        let expr = Expr::expr(self.inner.clone());
        Ok(Self::from_rust(match subquery(values)? {
            Some(select) => expr.not_in_subquery(select),
            None => expr.is_not_in(sequence(values)?),
        }))
    }
    /// `BETWEEN`, or with `symmetric` `BETWEEN SYMMETRIC`, which sorts its
    /// bounds first, so they hold in whichever order they are given.
    #[pyo3(signature = (lower, upper, *, symmetric=false))]
    pub(crate) fn between(
        &self,
        lower: &Bound<'_, PyAny>,
        upper: &Bound<'_, PyAny>,
        symmetric: bool,
    ) -> PyResult<Self> {
        let (lower, upper) = (coerce(lower)?.inner, coerce(upper)?.inner);
        let expr = Expr::expr(self.inner.clone());
        Ok(Self::from_rust(if symmetric {
            expr.between_symmetric(lower, upper)
        } else {
            expr.between(lower, upper)
        }))
    }
    #[pyo3(signature = (lower, upper, *, symmetric=false))]
    pub(crate) fn not_between(
        &self,
        lower: &Bound<'_, PyAny>,
        upper: &Bound<'_, PyAny>,
        symmetric: bool,
    ) -> PyResult<Self> {
        let (lower, upper) = (coerce(lower)?.inner, coerce(upper)?.inner);
        let expr = Expr::expr(self.inner.clone());
        Ok(Self::from_rust(if symmetric {
            expr.not_between_symmetric(lower, upper)
        } else {
            expr.not_between(lower, upper)
        }))
    }
}
