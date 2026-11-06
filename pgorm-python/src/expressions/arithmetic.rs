//! `Expr`'s arithmetic, as Python's numeric operators, and `||`.

use pgorm::pgorm_query::BinOper;
use pyo3::prelude::*;

use super::PyExpr;

// [spec:pgorm:req:bindings.expressions+4]
// [spec:pgorm:req:python.naming+4]
#[pymethods]
impl PyExpr {
    fn __add__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(BinOper::Add, other)
    }
    fn __sub__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(BinOper::Sub, other)
    }
    fn __mul__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(BinOper::Mul, other)
    }
    fn __truediv__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(BinOper::Div, other)
    }
    fn __mod__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(BinOper::Mod, other)
    }
    /// `||`: text, array and `jsonb` concatenation, one operator for all
    /// three. Python's `+` stays arithmetic.
    fn concat(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(BinOper::Concatenate, other)
    }
}
