//! `Expr`'s array subscripts: one element, `a[i]`, and a slice, `a[l:u]`.

use pgorm::pgorm_query::{Expr, Subscript};
use pyo3::prelude::*;

use super::{PyExpr, coerce};

// [spec:pgorm:req:bindings.expressions+3]
// [spec:pgorm:req:python.expressions+7]
#[pymethods]
impl PyExpr {
    /// `a[index]`: one element, through Rust's `Expr::index`. Arrays count
    /// from 1, and an index outside the array's bounds is NULL rather than an
    /// error. The index is bound unless it is an expression.
    fn at(&self, index: &Bound<'_, PyAny>) -> PyResult<Self> {
        let index = coerce(index)?.inner;
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).index(index).into(),
        ))
    }
    /// `a[lower:upper]`, both bounds inclusive, through Rust's
    /// `Subscript::Slice`; `None` leaves that end open, so `slice()` is
    /// `a[:]`. A slice is an array, cut to the array's bounds.
    #[pyo3(signature = (lower=None, upper=None))]
    fn slice(
        &self,
        lower: Option<&Bound<'_, PyAny>>,
        upper: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let bound = |end: Option<&Bound<'_, PyAny>>| -> PyResult<_> {
            end.map(|end| coerce(end).map(|end| end.inner)).transpose()
        };
        let slice = Subscript::Slice(bound(lower)?, bound(upper)?);
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).subscript(slice).into(),
        ))
    }
}
