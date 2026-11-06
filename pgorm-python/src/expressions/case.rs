//! `CASE`, searched and simple, through pgorm-query's CASE builder. Each form
//! is an `Expr` once it has an arm, its statement held by the `Expr` it
//! extends, so it goes wherever an expression goes; a simple CASE's operand
//! alone is no expression, as PostgreSQL's grammar requires a `WHEN`.

use pgorm::pgorm_query::{CaseOperand, CaseStatement, Expr, SimpleCaseStatement, SimpleExpr};
use pyo3::{PyClass, prelude::*};

use super::{PyExpr, coerce};
use crate::errors::{ConstructionError, InternalError};

fn object<T>(py: Python<'_>, statement: SimpleExpr, class: T) -> PyResult<Bound<'_, T>>
where
    T: PyClass<BaseType = PyExpr>,
{
    Bound::new(
        py,
        PyClassInitializer::from(PyExpr::from_rust(statement)).add_subclass(class),
    )
}

/// The refusal of a CASE whose base holds another expression, which only
/// this module's constructors make, so no input causes it.
fn mismatch(form: &str) -> PyErr {
    InternalError::new_err(format!("a {form} CASE holds another expression"))
}

/// A searched `CASE`: `CASE WHEN condition THEN result .. END`. It takes
/// further `when` arms, tried in order, and one `else_`.
// [spec:pgorm:req:bindings.case]
#[pyclass(name = "SearchedCase", module = "pgorm", extends = PyExpr, frozen)]
#[derive(Debug)]
pub struct PySearchedCase;

impl PySearchedCase {
    fn statement(slf: &Bound<'_, Self>) -> PyResult<CaseStatement> {
        match &slf.as_super().get().inner {
            SimpleExpr::Case(statement) => Ok((**statement).clone()),
            _ => Err(mismatch("searched")),
        }
    }
}

#[pymethods]
impl PySearchedCase {
    /// A further `WHEN condition THEN result` arm. The condition is an
    /// expression or a `Condition`; the result is bound unless it is an
    /// expression.
    fn when<'py>(
        slf: &Bound<'py, Self>,
        condition: &Bound<'py, PyAny>,
        result: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let statement = Self::statement(slf)?.case(
            crate::statements::condition(condition)?,
            coerce(result)?.inner,
        );
        object(slf.py(), statement.into(), Self)
    }

    /// The `ELSE` result, which ends the CASE: what it gives is an `Expr`
    /// that takes no further arm.
    fn else_(slf: &Bound<'_, Self>, result: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        let statement = Self::statement(slf)?.finally(coerce(result)?.inner);
        Ok(PyExpr::from_rust(statement.into()))
    }
}

/// A simple `CASE`: `CASE operand WHEN value THEN result .. END`, which
/// compares its operand with each arm's value by `=`, so a NULL operand
/// matches no arm. It takes further `when` arms and one `else_`.
// [spec:pgorm:req:bindings.case]
#[pyclass(name = "SimpleCase", module = "pgorm", extends = PyExpr, frozen)]
#[derive(Debug)]
pub struct PySimpleCase;

impl PySimpleCase {
    fn statement(slf: &Bound<'_, Self>) -> PyResult<SimpleCaseStatement> {
        match &slf.as_super().get().inner {
            SimpleExpr::SimpleCase(statement) => Ok((**statement).clone()),
            _ => Err(mismatch("simple")),
        }
    }
}

#[pymethods]
impl PySimpleCase {
    /// A further `WHEN value THEN result` arm, each bound unless it is an
    /// expression.
    fn when<'py>(
        slf: &Bound<'py, Self>,
        value: &Bound<'py, PyAny>,
        result: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let statement = Self::statement(slf)?.when(coerce(value)?.inner, coerce(result)?.inner);
        object(slf.py(), statement.into(), Self)
    }

    fn else_(slf: &Bound<'_, Self>, result: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        let statement = Self::statement(slf)?.finally(coerce(result)?.inner);
        Ok(PyExpr::from_rust(statement.into()))
    }
}

/// The operand of a simple `CASE`, which is no expression until `when` gives
/// it its first arm.
// [spec:pgorm:req:bindings.case]
#[pyclass(name = "CaseOperand", module = "pgorm", frozen)]
#[derive(Debug)]
pub struct PyCaseOperand {
    inner: CaseOperand,
}

#[pymethods]
impl PyCaseOperand {
    /// The first `WHEN value THEN result` arm, which makes the CASE an
    /// expression.
    fn when<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
        result: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PySimpleCase>> {
        let statement = self
            .inner
            .clone()
            .when(coerce(value)?.inner, coerce(result)?.inner);
        object(py, statement.into(), PySimpleCase)
    }

    // [spec:pgorm:req:python.truth-testing+5]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "a CASE operand cannot be tested as a Python boolean",
        ))
    }

    fn __repr__(&self) -> &'static str {
        "CaseOperand(...)"
    }
}

/// `case_when(condition, result)`: a searched `CASE` with its first arm,
/// through Rust's `Expr::case`.
// [spec:pgorm:req:bindings.case]
#[pyfunction]
pub(crate) fn case_when<'py>(
    condition: &Bound<'py, PyAny>,
    result: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PySearchedCase>> {
    let statement = Expr::case(
        crate::statements::condition(condition)?,
        coerce(result)?.inner,
    );
    object(condition.py(), statement.into(), PySearchedCase)
}

/// `case_of(operand)`: a simple `CASE` over the operand, bound unless it is
/// an expression, through Rust's `Expr::case_of`.
// [spec:pgorm:req:bindings.case]
#[pyfunction]
pub(crate) fn case_of(operand: &Bound<'_, PyAny>) -> PyResult<PyCaseOperand> {
    let inner = Expr::case_of(coerce(operand)?.inner);
    Ok(PyCaseOperand { inner })
}
