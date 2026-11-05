//! The items of a GROUP BY list beyond a plain expression, over Rust's
//! `GroupingElement`.

use pgorm::pgorm_query::{GroupingElement, SimpleExpr};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyTuple};

use crate::{errors::ConstructionError, expressions::PyExpr};

// [spec:pgorm:req:bindings.select+4]
/// One GROUP BY item that is not a bare expression: `()`, a parenthesised
/// set, `ROLLUP`, `CUBE` or `GROUPING SETS`, each built by the Rust
/// constructor of the same name. `sets` takes its first element, so the
/// empty `GROUPING SETS` PostgreSQL refuses cannot be built.
#[pyclass(name = "GroupingElement", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyGroupingElement {
    inner: GroupingElement,
}

/// What one argument of `group_by` or `GroupingElement.sets` stands for.
pub(super) enum Grouped {
    Expr(SimpleExpr),
    Element(GroupingElement),
}

impl Grouped {
    fn into_element(self) -> GroupingElement {
        match self {
            Self::Expr(expr) => GroupingElement::set([expr]),
            Self::Element(element) => element,
        }
    }
}

/// A `GroupingElement` as itself, and an expression as the set of it alone.
pub(super) fn element(value: &Bound<'_, PyAny>) -> PyResult<Grouped> {
    if let Ok(element) = value.extract::<PyRef<'_, PyGroupingElement>>() {
        Ok(Grouped::Element(element.inner.clone()))
    } else if let Ok(expr) = value.extract::<PyRef<'_, PyExpr>>() {
        Ok(Grouped::Expr(expr.inner.clone()))
    } else {
        Err(PyTypeError::new_err(
            "a grouping item requires an expression or GroupingElement",
        ))
    }
}

fn expressions(exprs: &Bound<'_, PyTuple>) -> PyResult<Vec<SimpleExpr>> {
    exprs
        .iter()
        .map(|expr| {
            expr.extract::<PyRef<'_, PyExpr>>()
                .map(|expr| expr.inner.clone())
                .map_err(|_| PyTypeError::new_err("a grouping set requires expressions"))
        })
        .collect()
}

#[pymethods]
impl PyGroupingElement {
    /// One grouping set, `(a, b)`; one expression renders bare and none
    /// renders `()`.
    #[staticmethod]
    #[pyo3(signature = (*exprs))]
    fn set(exprs: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            inner: GroupingElement::set(expressions(exprs)?),
        })
    }

    /// The empty grouping set, `()`: the whole input as one group.
    #[staticmethod]
    fn empty() -> Self {
        Self {
            inner: GroupingElement::empty(),
        }
    }

    /// `ROLLUP (..)`: every leading prefix of the list, down to `()`.
    #[staticmethod]
    #[pyo3(signature = (*exprs))]
    fn rollup(exprs: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            inner: GroupingElement::rollup(expressions(exprs)?),
        })
    }

    /// `CUBE (..)`: every subset of the list, `()` included.
    #[staticmethod]
    #[pyo3(signature = (*exprs))]
    fn cube(exprs: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            inner: GroupingElement::cube(expressions(exprs)?),
        })
    }

    /// `GROUPING SETS (..)` over its elements in order; an expression is the
    /// set of it alone.
    #[staticmethod]
    #[pyo3(signature = (first, *rest))]
    fn sets(first: &Bound<'_, PyAny>, rest: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut sets = GroupingElement::sets(element(first)?.into_element());
        for item in rest.iter() {
            sets = sets.add(element(&item)?.into_element());
        }
        Ok(Self { inner: sets.into() })
    }

    fn __repr__(&self) -> &'static str {
        "GroupingElement(...)"
    }

    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "grouping elements cannot be tested as Python booleans",
        ))
    }
}
