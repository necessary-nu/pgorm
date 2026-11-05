//! The native base of a model's declared column: an `Expr` a Python subclass
//! builds from the column's expression, so the column is itself an
//! expression and every `Expr` method reads it, while `Expr` keeps no
//! constructor of its own.

use pyo3::prelude::*;

use super::PyExpr;

// [spec:pgorm:req:bindings.column-operators+3]
#[pyclass(
    name = "_DeclaredColumn",
    module = "pgorm._native",
    extends = PyExpr,
    subclass,
    frozen
)]
#[derive(Debug)]
pub(crate) struct DeclaredColumn;

#[pymethods]
impl DeclaredColumn {
    #[new]
    fn new(expression: PyRef<'_, PyExpr>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(expression.clone()).add_subclass(Self)
    }
}
