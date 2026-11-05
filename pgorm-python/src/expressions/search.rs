//! `Expr`'s document search: `@?`, a `jsonpath` against a `jsonb`, and
//! `@@`, which is both full-text search and a `jsonpath` predicate.

use pgorm::pgorm_query::{BinOper, Expr};
use pyo3::{exceptions::PyTypeError, prelude::*};

use super::PyExpr;
use crate::statements::PyQuantified;

// [spec:pgorm:req:bindings.expressions+2]
// [spec:pgorm:req:bindings.quantifiers+2]
#[pymethods]
impl PyExpr {
    /// `@?`: whether a `jsonpath` returns any item for this `jsonb`. The path
    /// is a `str`, bound as text cast to `jsonpath` by Rust's
    /// `json_path_exists`, so it never becomes statement text, or `ANY (..)`
    /// / `ALL (..)` over `jsonpath` values, which a bound text array is not.
    fn path_exists(&self, path: &Bound<'_, PyAny>) -> PyResult<Self> {
        if path.is_instance_of::<PyQuantified>() {
            return self.binary(BinOper::JsonPathExists, path);
        }
        let path: String = path
            .extract()
            .map_err(|_| PyTypeError::new_err("a JSON path is a str, or any_ or all_"))?;
        Ok(Self::from_rust(
            Expr::expr(self.inner.clone()).json_path_exists(path),
        ))
    }
    /// `@@`: a `tsvector` against a `tsquery`, text against text as
    /// PostgreSQL's own full-text conversion reads it, or a `jsonb` against
    /// a `jsonpath` predicate. The operand is bound unless it is an
    /// expression; a `jsonpath` is written cast, since a bound `str` cannot
    /// be sent as one.
    fn matches(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(BinOper::Matches, other)
    }
}
