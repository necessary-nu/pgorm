//! Composable, owned wrappers of the existing Rust expression builders.
//!
//! `Expr`'s methods are one `#[pymethods]` block per operator family, each
//! in its own module (PyO3's `multiple-pymethods`), as its stub is one mixin
//! per family.

mod arithmetic;
mod capabilities;
mod case;
mod comparison;
mod compiled;
mod construct;
mod declared;
mod logic;
mod options;
mod patterns;
mod predicates;
mod ranges;
mod search;
mod shaping;
mod subscripts;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod variants;

use pgorm::pgorm_query::{BinOper, SimpleExpr};
use pyo3::prelude::*;

use crate::statements::PyQuantified;

pub(crate) use capabilities::operations as capabilities;
pub use compiled::Compiled;
pub use construct::PyCondition;
pub(crate) use construct::{coerce, require_expr};
pub use options::{Aliased, LikePattern, OrderBy};

// [spec:pgorm:req:bindings.expressions+2]
// [spec:pgorm:req:python.expressions+7]
// [spec:pgorm:req:bindings.statements]
/// An immutable owned Rust SimpleExpr. Every composition clones its inputs.
#[pyclass(name = "Expr", module = "pgorm", frozen, from_py_object, subclass)]
#[derive(Clone, Debug)]
pub struct PyExpr {
    pub inner: SimpleExpr,
}

impl PyExpr {
    pub fn from_rust(inner: SimpleExpr) -> Self {
        Self { inner }
    }

    /// `operator` with `other`, bound unless it is an expression, or with
    /// `ANY (..)` / `ALL (..)` when PostgreSQL reads one after `operator`.
    pub(crate) fn binary(&self, operator: BinOper, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        let right = match other.extract::<PyRef<'_, PyQuantified>>() {
            Ok(quantified) if takes_quantifier(operator) => quantified.inner.clone(),
            _ => coerce(other)?.inner,
        };
        Ok(Self::from_rust(self.inner.clone().binary(operator, right)))
    }
}

/// The operators PostgreSQL's grammar takes before `ANY` or `ALL` whose
/// result is a boolean, as the server requires of one (`42809`). `SIMILAR
/// TO`, `IS DISTINCT FROM`, `BETWEEN` and `IN` are read by the grammar with
/// no quantifier at all.
// [spec:pgorm:req:bindings.quantifiers+1]
fn takes_quantifier(operator: BinOper) -> bool {
    matches!(
        operator,
        BinOper::Equal
            | BinOper::NotEqual
            | BinOper::SmallerThan
            | BinOper::SmallerThanOrEqual
            | BinOper::GreaterThan
            | BinOper::GreaterThanOrEqual
            | BinOper::Like
            | BinOper::NotLike
            | BinOper::ILike
            | BinOper::NotILike
            | BinOper::Regex
            | BinOper::NotRegex
            | BinOper::RegexCaseInsensitive
            | BinOper::NotRegexCaseInsensitive
            | BinOper::Contains
            | BinOper::Contained
            | BinOper::Overlap
            | BinOper::LShift
            | BinOper::RShift
            | BinOper::NotExtendRightOf
            | BinOper::NotExtendLeftOf
            | BinOper::AdjacentTo
            | BinOper::Matches
            | BinOper::JsonPathExists
    )
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyExpr>()?;
    module.add_class::<declared::DeclaredColumn>()?;
    module.add_class::<PyCondition>()?;
    module.add_class::<LikePattern>()?;
    module.add_class::<Aliased>()?;
    module.add_class::<OrderBy>()?;
    module.add_class::<Compiled>()?;
    module.add_class::<case::PySearchedCase>()?;
    module.add_class::<case::PySimpleCase>()?;
    module.add_class::<case::PyCaseOperand>()?;
    module.add_function(wrap_pyfunction!(case::case_when, module)?)?;
    module.add_function(wrap_pyfunction!(case::case_of, module)?)?;
    module.add_function(wrap_pyfunction!(construct::col, module)?)?;
    module.add_function(wrap_pyfunction!(construct::bind, module)?)?;
    module.add_function(wrap_pyfunction!(construct::literal, module)?)?;
    module.add_function(wrap_pyfunction!(construct::call, module)?)?;
    module.add_function(wrap_pyfunction!(construct::tuple_expr, module)?)?;
    module.add_function(wrap_pyfunction!(construct::domain_value, module)?)?;
    Ok(())
}
