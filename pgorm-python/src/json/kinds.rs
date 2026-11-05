//! The closed choices SQL/JSON's clauses take, and the two expressions built
//! straight from an operand: `FORMAT JSON` and `IS [NOT] JSON`.

use pgorm::pgorm_query::{
    Expr, JsonExistsBehavior, JsonInput, JsonKind, JsonQueryBehavior, JsonTest, JsonValueBehavior,
    SimpleExpr, Value,
};
use pyo3::{exceptions::PyTypeError, prelude::*};

use crate::{
    errors::ConstructionError,
    expressions::{PyExpr, coerce},
    values::PyValue,
};

// [spec:pgorm:req:bindings.sql-json]
/// An operand marked as JSON text for an SQL/JSON position: `FORMAT JSON`.
/// It is accepted only where SQL/JSON reads JSON, never as an expression.
#[pyclass(name = "JsonInput", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyJsonInput {
    pub(crate) inner: JsonInput,
}

#[pymethods]
impl PyJsonInput {
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL/JSON operands cannot be tested as Python booleans",
        ))
    }

    fn __repr__(&self) -> &'static str {
        "JsonInput(...)"
    }
}

#[pyfunction]
pub(crate) fn format_json(operand: &Bound<'_, PyAny>) -> PyResult<PyJsonInput> {
    Ok(PyJsonInput {
        inner: Expr::expr(coerce(operand)?.inner).format_json(),
    })
}

/// Which JSON an `IS JSON` test accepts.
#[pyclass(name = "JsonKind", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyJsonKind {
    Value,
    Scalar,
    Array,
    Object,
}

#[pymethods]
impl PyJsonKind {
    // [spec:pgorm:req:python.truth-testing+5]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL/JSON clauses cannot be tested as Python booleans",
        ))
    }
}

fn json_test(kind: &PyJsonKind, unique_keys: bool) -> JsonTest {
    let kind = match kind {
        PyJsonKind::Value => JsonKind::Value,
        PyJsonKind::Scalar => JsonKind::Scalar,
        PyJsonKind::Array => JsonKind::Array,
        PyJsonKind::Object => JsonKind::Object,
    };
    if unique_keys {
        kind.with_unique_keys()
    } else {
        kind.into()
    }
}

fn json_predicate(operand: &Bound<'_, PyAny>, test: JsonTest, negated: bool) -> PyResult<PyExpr> {
    let operand = Expr::expr(coerce(operand)?.inner);
    let inner: SimpleExpr = if negated {
        operand.is_not_json(test)
    } else {
        operand.is_json(test)
    };
    Ok(PyExpr::from_rust(inner))
}

#[pyfunction]
#[pyo3(signature = (operand, kind=PyJsonKind::Value, *, unique_keys=false))]
pub(crate) fn is_json(
    operand: &Bound<'_, PyAny>,
    kind: PyJsonKind,
    unique_keys: bool,
) -> PyResult<PyExpr> {
    json_predicate(operand, json_test(&kind, unique_keys), false)
}

#[pyfunction]
#[pyo3(signature = (operand, kind=PyJsonKind::Value, *, unique_keys=false))]
pub(crate) fn is_not_json(
    operand: &Bound<'_, PyAny>,
    kind: PyJsonKind,
    unique_keys: bool,
) -> PyResult<PyExpr> {
    json_predicate(operand, json_test(&kind, unique_keys), true)
}

/// What `JSON_EXISTS` answers when its path fails: `ON ERROR`. `True_` and
/// `False_` carry the underscore Python's keywords need.
#[pyclass(name = "JsonExistsBehavior", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyJsonExistsBehavior {
    #[pyo3(name = "True_")]
    True,
    #[pyo3(name = "False_")]
    False,
    Unknown,
    Error,
}

#[pymethods]
impl PyJsonExistsBehavior {
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL/JSON clauses cannot be tested as Python booleans",
        ))
    }
}

impl PyJsonExistsBehavior {
    pub(super) fn rust(&self) -> JsonExistsBehavior {
        match self {
            Self::True => JsonExistsBehavior::True,
            Self::False => JsonExistsBehavior::False,
            Self::Unknown => JsonExistsBehavior::Unknown,
            Self::Error => JsonExistsBehavior::Error,
        }
    }
}

/// `JSON_VALUE`'s `ON EMPTY` / `ON ERROR` keywords; a value is a `JsonDefault`.
#[pyclass(name = "JsonValueBehavior", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyJsonValueBehavior {
    Null,
    Error,
}

#[pymethods]
impl PyJsonValueBehavior {
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL/JSON clauses cannot be tested as Python booleans",
        ))
    }
}

/// `JSON_QUERY`'s `ON EMPTY` / `ON ERROR` keywords; a value is a `JsonDefault`.
#[pyclass(name = "JsonQueryBehavior", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyJsonQueryBehavior {
    Null,
    Error,
    EmptyArray,
    EmptyObject,
}

#[pymethods]
impl PyJsonQueryBehavior {
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL/JSON clauses cannot be tested as Python booleans",
        ))
    }
}

/// `DEFAULT value ON EMPTY | ON ERROR`. PostgreSQL refuses a parameter there,
/// so the Rust renderer writes the value as an escaped literal in both render
/// paths; it is never bound and never spliced unescaped.
#[pyclass(name = "JsonDefault", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyJsonDefault {
    value: Value,
}

#[pymethods]
impl PyJsonDefault {
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let value = PyValue::coerce(value)?;
        if value.has_named_type() {
            return Err(ConstructionError::new_err(
                "a JSON DEFAULT is written as a plain literal; it cannot carry an enum's or a created range's cast",
            ));
        }
        Ok(Self {
            value: value.rust_value().clone(),
        })
    }

    fn __repr__(&self) -> String {
        format!("JsonDefault({:?})", self.value)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL/JSON clauses cannot be tested as Python booleans",
        ))
    }
}

/// `JSON_VALUE`'s behaviour from a keyword or a `JsonDefault`.
pub(super) fn value_behavior(value: &Bound<'_, PyAny>) -> PyResult<JsonValueBehavior> {
    if let Ok(default) = value.extract::<PyRef<'_, PyJsonDefault>>() {
        return Ok(JsonValueBehavior::Default(default.value.clone()));
    }
    match value.extract::<PyJsonValueBehavior>() {
        Ok(PyJsonValueBehavior::Null) => Ok(JsonValueBehavior::Null),
        Ok(PyJsonValueBehavior::Error) => Ok(JsonValueBehavior::Error),
        Err(_) => Err(PyTypeError::new_err(
            "JSON_VALUE's behaviour requires a JsonValueBehavior or JsonDefault",
        )),
    }
}

/// `JSON_QUERY`'s behaviour from a keyword or a `JsonDefault`.
pub(super) fn query_behavior(value: &Bound<'_, PyAny>) -> PyResult<JsonQueryBehavior> {
    if let Ok(default) = value.extract::<PyRef<'_, PyJsonDefault>>() {
        return Ok(JsonQueryBehavior::Default(default.value.clone()));
    }
    match value.extract::<PyJsonQueryBehavior>() {
        Ok(PyJsonQueryBehavior::Null) => Ok(JsonQueryBehavior::Null),
        Ok(PyJsonQueryBehavior::Error) => Ok(JsonQueryBehavior::Error),
        Ok(PyJsonQueryBehavior::EmptyArray) => Ok(JsonQueryBehavior::EmptyArray),
        Ok(PyJsonQueryBehavior::EmptyObject) => Ok(JsonQueryBehavior::EmptyObject),
        Err(_) => Err(PyTypeError::new_err(
            "JSON_QUERY's behaviour requires a JsonQueryBehavior or JsonDefault",
        )),
    }
}
