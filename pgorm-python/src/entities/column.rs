use pgorm::pgorm_query::Value;
use pyo3::{
    basic::CompareOp,
    exceptions::PyTypeError,
    prelude::*,
    types::{PyList, PyTuple},
};
use std::sync::Arc;

use super::{
    backend::{Between, Comparison, EntityBackend, Members},
    metadata::EntityInfo,
};
use crate::{
    errors::{ConstructionError, LifecycleError},
    expressions::PyExpr,
    identifiers::PyIdentifier,
};

/// A registered entity's column, itself the column's expression: every
/// `Expr` method reads it as written, and its comparisons, `[NOT] BETWEEN
/// [SYMMETRIC]`, `IS [NOT] DISTINCT FROM` and membership are the column's own
/// `ColumnTrait` methods.
// [spec:pgorm:req:bindings.column-operators+2]
#[derive(Clone, Debug)]
#[pyclass(name = "EntityColumn", module = "pgorm", frozen, from_py_object, extends = PyExpr)]
pub struct PyEntityColumn {
    pub(crate) entity: Arc<dyn EntityBackend>,
    #[pyo3(get)]
    pub(crate) name: String,
}

impl PyEntityColumn {
    pub(crate) fn new(
        entity: Arc<dyn EntityBackend>,
        name: String,
    ) -> PyResult<PyClassInitializer<Self>> {
        let expression = entity
            .expression(&name)
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        Ok(PyClassInitializer::from(PyExpr::from_rust(expression))
            .add_subclass(Self { entity, name }))
    }

    /// `[NOT] BETWEEN [SYMMETRIC]` through the column's `ColumnTrait`, each
    /// bound converted to the column's declared kind as a compared value is.
    /// A bound that is an expression leaves the test to `Expr`'s, both bounds
    /// taken as written.
    // [spec:pgorm:req:bindings.column-operators+2]
    fn bounded(
        &self,
        lower: &Bound<'_, PyAny>,
        upper: &Bound<'_, PyAny>,
        between: Between,
    ) -> PyResult<PyExpr> {
        if lower.is_instance_of::<PyExpr>() || upper.is_instance_of::<PyExpr>() {
            let expr = self.expr()?;
            return match between.negated {
                false => expr.between(lower, upper, between.symmetric),
                true => expr.not_between(lower, upper, between.symmetric),
            };
        }
        let (lower, upper) = (self.compared(lower)?, self.compared(upper)?);
        self.entity
            .between(&self.name, between, lower, upper)
            .map(PyExpr::from_rust)
            .map_err(|error| ConstructionError::new_err(error.to_string()))
    }

    /// `IS [NOT] DISTINCT FROM` through the column's `ColumnTrait`, a value
    /// converted as a compared value is and an expression taken as written.
    // [spec:pgorm:req:bindings.column-operators+2]
    fn distinct(&self, other: &Bound<'_, PyAny>, negated: bool) -> PyResult<PyExpr> {
        if !other.is_instance_of::<PyExpr>() {
            let op = match negated {
                false => Comparison::IsDistinctFrom,
                true => Comparison::IsNotDistinctFrom,
            };
            return self.comparison(other, op);
        }
        let expr = self.expr()?;
        match negated {
            false => expr.is_distinct_from(other),
            true => expr.is_not_distinct_from(other),
        }
    }

    /// `value` converted to the column's declared kind as a comparison takes
    /// it. `None` is refused: no comparison with NULL holds.
    fn compared(&self, value: &Bound<'_, PyAny>) -> PyResult<Value> {
        if value.is_none() {
            return Err(ConstructionError::new_err(format!(
                "{} compared with None is never true: test it with is_null()",
                self.name
            )));
        }
        Ok(self
            .entity
            .info()
            .column(&self.name)?
            .input
            .coerce(value)?
            .inner)
    }

    fn comparison(&self, value: &Bound<'_, PyAny>, op: Comparison) -> PyResult<PyExpr> {
        let value = self.compared(value)?;
        self.entity
            .compare(&self.name, op, value)
            .map(PyExpr::from_rust)
            .map_err(|error| crate::errors::ConstructionError::new_err(error.to_string()))
    }

    /// Membership through the column's `ColumnTrait`: `is_in` / `is_not_in`
    /// over a list or tuple of values, each converted to the column's
    /// declared kind as a compared value is, or `in_subquery` /
    /// `not_in_subquery` over a `Select`'s rows, so a column whose domain is
    /// over an enum is a member as the enum, and so is what the subquery
    /// projects.
    // [spec:pgorm:req:bindings.entity-reads+2]
    fn membership(&self, values: &Bound<'_, PyAny>, negated: bool) -> PyResult<PyExpr> {
        let members = if let Some(select) = crate::statements::subquery(values)? {
            Members::Subquery(Box::new(select))
        } else if values.is_exact_instance_of::<PyList>()
            || values.is_exact_instance_of::<PyTuple>()
        {
            let input = &self.entity.info().column(&self.name)?.input;
            let values = values
                .try_iter()?
                .map(|value| Ok(input.coerce(&value?)?.inner));
            Members::Values(values.collect::<PyResult<_>>()?)
        } else {
            return Err(PyTypeError::new_err(
                "membership requires a list or tuple of values, or a Select",
            ));
        };
        self.entity
            .membership(&self.name, negated, members)
            .map(PyExpr::from_rust)
            .map_err(|error| ConstructionError::new_err(error.to_string()))
    }
}

#[pymethods]
impl PyEntityColumn {
    #[getter]
    fn entity_name(&self) -> &str {
        &self.entity.info().name
    }

    fn expr(&self) -> PyResult<PyExpr> {
        self.entity
            .expression(&self.name)
            .map(PyExpr::from_rust)
            .map_err(|error| crate::errors::ConstructionError::new_err(error.to_string()))
    }

    fn eq(&self, other: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        self.comparison(other, Comparison::Eq)
    }
    fn ne(&self, other: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        self.comparison(other, Comparison::Ne)
    }
    fn gt(&self, other: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        self.comparison(other, Comparison::Gt)
    }
    fn gte(&self, other: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        self.comparison(other, Comparison::Ge)
    }
    fn lt(&self, other: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        self.comparison(other, Comparison::Lt)
    }
    fn lte(&self, other: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        self.comparison(other, Comparison::Le)
    }

    #[pyo3(signature = (lower, upper, *, symmetric=false))]
    fn between(
        &self,
        lower: &Bound<'_, PyAny>,
        upper: &Bound<'_, PyAny>,
        symmetric: bool,
    ) -> PyResult<PyExpr> {
        let negated = false;
        self.bounded(lower, upper, Between { negated, symmetric })
    }
    #[pyo3(signature = (lower, upper, *, symmetric=false))]
    fn not_between(
        &self,
        lower: &Bound<'_, PyAny>,
        upper: &Bound<'_, PyAny>,
        symmetric: bool,
    ) -> PyResult<PyExpr> {
        let negated = true;
        self.bounded(lower, upper, Between { negated, symmetric })
    }
    fn is_distinct_from(&self, other: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        self.distinct(other, false)
    }
    fn is_not_distinct_from(&self, other: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        self.distinct(other, true)
    }

    fn is_in(&self, values: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        self.membership(values, false)
    }
    fn is_not_in(&self, values: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
        self.membership(values, true)
    }

    fn __richcmp__(&self, value: &Bound<'_, PyAny>, op: CompareOp) -> PyResult<PyExpr> {
        self.comparison(
            value,
            match op {
                CompareOp::Eq => Comparison::Eq,
                CompareOp::Ne => Comparison::Ne,
                CompareOp::Gt => Comparison::Gt,
                CompareOp::Ge => Comparison::Ge,
                CompareOp::Lt => Comparison::Lt,
                CompareOp::Le => Comparison::Le,
            },
        )
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "entity columns cannot be tested as Python booleans",
        ))
    }
    fn __repr__(&self) -> String {
        format!("EntityColumn({:?}, {:?})", self.entity_name(), self.name)
    }
}

pub(crate) fn name(input: &Bound<'_, PyAny>, owner: &Arc<EntityInfo>) -> PyResult<String> {
    let name = if let Ok(column) = input.extract::<PyRef<'_, PyEntityColumn>>() {
        if !Arc::ptr_eq(column.entity.info(), owner) {
            return Err(LifecycleError::new_err(
                "column belongs to a different entity registration",
            ));
        }
        column.name.clone()
    } else {
        PyIdentifier::new(input)?.name
    };
    owner.column(&name)?;
    Ok(name)
}
