//! A Python callback owns recipes; only its Rust closure owns branded Exprs.

use std::sync::{Arc, Mutex};

use pgorm::{pgorm_query::Value, pipeline as pl};
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyList, PyTuple},
};

use super::{
    expression::PyPipelineExpr,
    recipe::{Lowering, Recipe},
};
use crate::{
    errors::{ConstructionError, InternalError, LifecycleError},
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
    pub(super) fn check(&self) -> PyResult<()> {
        if self
            .0
            .lock()
            .map_err(|_| InternalError::new_err("pipeline binder state is poisoned"))?
            .active
        {
            Ok(())
        } else {
            Err(LifecycleError::new_err("pipeline binder scope has ended"))
        }
    }

    fn bind(&self, value: Value) -> PyResult<usize> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| InternalError::new_err("pipeline binder state is poisoned"))?;
        if !state.active {
            return Err(LifecycleError::new_err("pipeline binder scope has ended"));
        }
        let index = state.values.len();
        if index >= 65535 {
            return Err(ConstructionError::new_err(
                "PostgreSQL supports at most 65535 query parameters",
            ));
        }
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
        self.scope.check()?;
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

/// A stage's expressions and the values its binder function bound, if it had
/// one.
pub(super) struct Plan {
    pub(super) nodes: Vec<Arc<Recipe>>,
    values: Vec<Value>,
}

impl Plan {
    /// The plan of a stage with no binder function: a placeholder in any of
    /// its expressions belongs to another stage, as pgorm's brand rules.
    // [spec:pgorm:req:bindings.pipeline-binder]
    pub(super) fn plain(expressions: Vec<PyPipelineExpr>) -> PyResult<Self> {
        for expression in &expressions {
            expression.check()?;
            if expression.scope.is_some() {
                return Err(LifecycleError::new_err(
                    "a placeholder belongs to the stage its binder's function returns it to, not a plain stage",
                ));
            }
        }
        Ok(Self {
            nodes: expressions
                .into_iter()
                .map(|expression| expression.recipe)
                .collect(),
            values: Vec::new(),
        })
    }

    /// Whether lowering the plan binds anything, so the stage needs its
    /// binder form.
    pub(super) fn binds(&self) -> bool {
        !self.values.is_empty() || self.nodes.iter().any(|node| node.binds())
    }

    /// The expressions lowered with no binder, for a plan that binds nothing.
    pub(super) fn unbound(&self) -> PyResult<Vec<pl::Expr<'static>>> {
        let mut with = Lowering {
            binder: None,
            bound: &[],
        };
        self.nodes
            .iter()
            .map(|node| node.lower(&mut with))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(unlowered)
    }

    /// The expressions lowered in a stage's binder closure: the function's
    /// values bound first, in the order it bound them, then each operand's
    /// own value as the expression reaches it. `failed` notes a lowering that
    /// could not finish, which one scope minting both rules out.
    // [spec:pgorm:req:bindings.pipeline-binder]
    // [spec:pgorm:req:bindings.operands]
    pub(super) fn lower<'brand, const N: usize>(
        &self,
        binder: &mut pl::Binder<'brand>,
        failed: &mut bool,
    ) -> [pl::Expr<'brand>; N] {
        let bound: Vec<_> = self
            .values
            .iter()
            .cloned()
            .map(|value| binder.bind(value))
            .collect();
        let mut with = Lowering {
            binder: Some(binder),
            bound: &bound,
        };
        let lowered = self
            .nodes
            .iter()
            .map(|node| node.lower(&mut with))
            .collect::<Option<Vec<_>>>()
            .and_then(|all| <[_; N]>::try_from(all).ok());
        lowered.unwrap_or_else(|| {
            *failed = true;
            std::array::from_fn(|_| pl::null())
        })
    }
}

/// A lowering that could not finish, which no input should cause.
pub(super) fn unlowered() -> PyErr {
    InternalError::new_err("a pipeline expression could not be lowered")
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
    for expression in &expressions {
        expression.check()?;
        if expression
            .scope
            .as_ref()
            .is_some_and(|owner| !Arc::ptr_eq(owner, &scope))
        {
            return Err(LifecycleError::new_err(
                "pipeline expression belongs to another binder scope",
            ));
        }
    }
    let mut state = scope
        .0
        .lock()
        .map_err(|_| InternalError::new_err("pipeline binder state is poisoned"))?;
    state.active = false;
    Ok(Plan {
        nodes: expressions
            .into_iter()
            .map(|expression| expression.recipe)
            .collect(),
        values: std::mem::take(&mut state.values),
    })
}
