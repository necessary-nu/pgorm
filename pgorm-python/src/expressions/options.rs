use pgorm::pgorm_query::LikeExpr;
use pyo3::{exceptions::PyTypeError, prelude::*};

use super::{PyExpr, construct::require_expr};
use crate::{
    errors::ConstructionError,
    identifiers::{Direction, Identifier, Nulls},
};

/// A LIKE, ILIKE or SIMILAR TO pattern and its `ESCAPE`, which the grammar
/// admits only there, as Rust's `LikeExpr` holds them: the pattern is bound,
/// and the escape, a backslash until `escape` or `no_escape` replaces it, is
/// written as a literal.
// [spec:pgorm:req:bindings.expressions+3]
#[pyclass(name = "LikeExpr", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyLikeExpr {
    pub inner: LikeExpr,
}

#[pymethods]
impl PyLikeExpr {
    #[new]
    fn new(pattern: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: LikeExpr::new(pattern_text(pattern)?),
        })
    }

    /// Escape with `character`, `ESCAPE 'c'`, in place of the default
    /// backslash.
    fn escape(&self, character: &Bound<'_, PyAny>) -> PyResult<Self> {
        let character: String = character
            .extract()
            .map_err(|_| PyTypeError::new_err("a LIKE escape is a string"))?;
        let mut chars = character.chars();
        let character = match (chars.next(), chars.next()) {
            (Some(character), None) if character != '\0' => character,
            _ => {
                return Err(ConstructionError::new_err(
                    "a LIKE escape is one character, not NUL; no_escape() turns escaping off",
                ));
            }
        };
        Ok(Self {
            inner: self.inner.clone().escape(character),
        })
    }

    /// Turn escaping off, `ESCAPE ''`: every character of the pattern, a
    /// backslash included, is then literal or a wildcard.
    fn no_escape(&self) -> Self {
        Self {
            inner: self.inner.clone().no_escape(),
        }
    }

    fn __repr__(&self) -> &'static str {
        "LikeExpr(...)"
    }

    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "LIKE patterns cannot be tested as Python booleans",
        ))
    }
}

/// A projection expression with a separate, validated output identifier.
#[pyclass(name = "Aliased", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct Aliased {
    #[pyo3(get)]
    pub expr: PyExpr,
    pub alias: Identifier,
}

#[pymethods]
impl Aliased {
    #[getter(alias)]
    fn alias_name(&self) -> &str {
        &self.alias.name
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL projections cannot be tested as Python booleans",
        ))
    }
}

/// An expression and typed direction/NULL placement for a statement's ORDER BY.
#[pyclass(name = "OrderBy", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct OrderBy {
    #[pyo3(get)]
    pub expr: PyExpr,
    #[pyo3(get)]
    pub direction: Direction,
    #[pyo3(get)]
    pub nulls: Option<Nulls>,
}

#[pymethods]
impl OrderBy {
    #[new]
    #[pyo3(signature = (expr, direction, *, nulls=None))]
    fn new(expr: &Bound<'_, PyAny>, direction: Direction, nulls: Option<Nulls>) -> PyResult<Self> {
        Ok(Self {
            expr: require_expr(expr)?,
            direction,
            nulls,
        })
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL orderings cannot be tested as Python booleans",
        ))
    }
}

fn pattern_text(value: &Bound<'_, PyAny>) -> PyResult<String> {
    value
        .extract()
        .map_err(|_| PyTypeError::new_err("a LIKE pattern is a string or a LikeExpr"))
}

/// The pattern `like` and its kin take: a `LikeExpr`, or a string, which is
/// the pattern with the default escape, as Rust's `IntoLikeExpr` reads one.
pub(super) fn pattern(value: &Bound<'_, PyAny>) -> PyResult<LikeExpr> {
    match value.extract::<PyRef<'_, PyLikeExpr>>() {
        Ok(pattern) => Ok(pattern.inner.clone()),
        Err(_) => pattern_text(value).map(LikeExpr::new),
    }
}
