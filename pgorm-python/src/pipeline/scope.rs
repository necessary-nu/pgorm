//! A Python callback owns recipes; only its Rust closure owns branded Exprs.

use std::sync::{Arc, Mutex};

use pgorm::pgorm_query::Value;
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyList, PyTuple},
};

pub(super) use pgorm_bindings::pipeline::Plan;

use super::{expression::PyPipelineExpr, recipe::Recipe};
use crate::{
    errors::{ConstructionError, InternalError, LifecycleError, Raise},
    values::PyValue,
};

#[derive(Debug)]
struct State {
    active: bool,
    values: Vec<Value>,
}

#[derive(Debug)]
pub(super) struct Scope(Mutex<State>);

impl Scope {
    /// Whether the scope's function is still running.
    pub(super) fn active(&self) -> PyResult<bool> {
        Ok(self
            .0
            .lock()
            .map_err(|_| InternalError::new_err("pipeline binder state is poisoned"))?
            .active)
    }

    fn bind(&self, value: Value) -> PyResult<usize> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| InternalError::new_err("pipeline binder state is poisoned"))?;
        if !state.active {
            return Err(LifecycleError::new_err(
                pgorm_bindings::pipeline::BINDER_ENDED,
            ));
        }
        let index = state.values.len();
        pgorm_bindings::parameter_count(index + 1).py()?;
        state.values.push(value);
        Ok(index)
    }
}

/// A value to bind: any parameter, inferred or declared with `Value`, but none
/// whose kind needs a cast pgorm's pipeline cannot write, nor `None`, which has
/// no kind.
// [spec:pgorm:req:bindings.pipeline-expressions+3]
// [spec:pgorm:req:bindings.operands]
pub(super) fn bindable(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    if value.is_none() {
        return Err(ConstructionError::new_err(
            "None has no kind to bind in a pipeline: write literal(None) or is_null(), or bind Value.null(kind)",
        ));
    }
    let value = PyValue::coerce(value)?;
    if value.has_named_type() {
        return Err(ConstructionError::new_err(
            "pgorm's pipeline has no cast to an enum or a created range: bind the text for the server to infer, or compare through compares_as",
        ));
    }
    Ok(value.rust_value().clone())
}

struct CloseScope(Arc<Scope>);

impl Drop for CloseScope {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.0.lock() {
            state.active = false;
            state.values.clear();
        }
    }
}

#[pyclass(name = "PipelineBinder", module = "pgorm.pipeline", frozen)]
#[derive(Debug)]
pub struct PyPipelineBinder {
    scope: Arc<Scope>,
}

#[pymethods]
impl PyPipelineBinder {
    fn bind(&self, value: &Bound<'_, PyAny>) -> PyResult<PyPipelineExpr> {
        if !self.scope.active()? {
            return Err(LifecycleError::new_err(
                pgorm_bindings::pipeline::BINDER_ENDED,
            ));
        }
        let index = self.scope.bind(bindable(value)?)?;
        Ok(PyPipelineExpr {
            recipe: Arc::new(Recipe::Bound(index)),
            scope: Some(self.scope.clone()),
            declared: None,
        })
    }

    fn __repr__(&self) -> &'static str {
        "PipelineBinder(...)"
    }
}

/// The plan of a stage with no binder function: a placeholder in any of its
/// expressions belongs to another stage, as pgorm's brand rules.
// [spec:pgorm:req:bindings.pipeline-binder]
pub(super) fn plain(expressions: Vec<PyPipelineExpr>) -> PyResult<Plan> {
    if expressions
        .iter()
        .any(|expression| expression.scope.is_some())
    {
        return Err(LifecycleError::new_err(pgorm_bindings::pipeline::MISPLACED));
    }
    Ok(Plan {
        recipes: expressions
            .into_iter()
            .map(|expression| expression.recipe)
            .collect(),
        values: Vec::new(),
    })
}

// [spec:pgorm:req:bindings.pipeline-binder]
pub(super) fn callback(function: &Bound<'_, PyAny>, single: bool) -> PyResult<Plan> {
    if !function.is_callable() {
        return Err(PyTypeError::new_err(
            "pipeline bound stages require a synchronous callable",
        ));
    }
    let scope = Arc::new(Scope(Mutex::new(State {
        active: true,
        values: Vec::new(),
    })));
    let _close = CloseScope(scope.clone());
    let binder = Py::new(
        function.py(),
        PyPipelineBinder {
            scope: scope.clone(),
        },
    )?;
    let result = function.call1((binder,))?;
    if result.hasattr("__await__")? {
        if result.hasattr("close")? {
            result.call_method0("close")?;
        }
        return Err(PyTypeError::new_err(
            "a binder function returns its expressions directly, never an awaitable",
        ));
    }
    let expressions = if single || result.is_instance_of::<PyPipelineExpr>() {
        vec![PyPipelineExpr::coerce(&result)?]
    } else {
        if !result.is_instance_of::<PyList>() && !result.is_instance_of::<PyTuple>() {
            return Err(PyTypeError::new_err(
                "bound projection callback must return a pipeline expression, list or tuple",
            ));
        }
        result
            .try_iter()?
            .map(|value| PyPipelineExpr::coerce(&value?))
            .collect::<PyResult<Vec<_>>>()?
    };
    let misplaced = expressions.iter().any(|expression| {
        expression
            .scope
            .as_ref()
            .is_some_and(|owner| !Arc::ptr_eq(owner, &scope))
    });
    if misplaced {
        return Err(LifecycleError::new_err(pgorm_bindings::pipeline::MISPLACED));
    }
    let mut state = scope
        .0
        .lock()
        .map_err(|_| InternalError::new_err("pipeline binder state is poisoned"))?;
    state.active = false;
    Ok(Plan {
        recipes: expressions
            .into_iter()
            .map(|expression| expression.recipe)
            .collect(),
        values: std::mem::take(&mut state.values),
    })
}
