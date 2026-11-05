use std::sync::Arc;

use pgorm::pipeline as pl;
use pyo3::{
    basic::CompareOp,
    exceptions::PyTypeError,
    prelude::*,
    types::{PyList, PyTuple},
};

use super::{
    recipe::{Binary, Recipe, Unary},
    scope::{Scope, bindable},
};
use crate::{
    UnsupportedCapabilityError,
    errors::{ConstructionError, LifecycleError},
    identifiers::PyIdentifier,
    values::PyTypeName,
};

/// A native expression recipe with an optional, checked Python callback scope.
#[pyclass(
    name = "PipelineExpr",
    module = "pgorm.pipeline",
    frozen,
    from_py_object
)]
#[derive(Clone, Debug)]
pub struct PyPipelineExpr {
    pub(super) recipe: Arc<Recipe>,
    pub(super) scope: Option<Arc<Scope>>,
}

impl PyPipelineExpr {
    pub(super) fn check(&self) -> PyResult<()> {
        self.scope.as_ref().map_or(Ok(()), |scope| scope.check())
    }

    pub(super) fn compose(recipe: Recipe, operands: &[Self]) -> PyResult<Self> {
        let mut owner: Option<Arc<Scope>> = None;
        for operand in operands {
            operand.check()?;
            if let Some(scope) = &operand.scope {
                if owner
                    .as_ref()
                    .is_some_and(|owner| !Arc::ptr_eq(owner, scope))
                {
                    return Err(LifecycleError::new_err(
                        "pipeline expression combines different binder scopes",
                    ));
                }
                owner = Some(scope.clone());
            }
        }
        Ok(Self {
            recipe: Arc::new(recipe),
            scope: owner,
        })
    }

    /// An operand: a pipeline expression, or a value the stage that takes it
    /// binds. Only `literal()` writes a value into the SQL.
    // [spec:pgorm:req:bindings.pipeline-expressions+3]
    // [spec:pgorm:req:bindings.operands]
    pub(super) fn coerce(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(expression) = value.extract::<PyRef<'_, Self>>() {
            expression.check()?;
            return Ok(expression.clone());
        }
        Ok(Self {
            recipe: Arc::new(Recipe::Value(bindable(value)?)),
            scope: None,
        })
    }

    fn binary(&self, operation: Binary, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let right = Self::coerce(value)?;
        Self::compose(
            Recipe::Binary(operation, self.recipe.clone(), right.recipe.clone()),
            &[self.clone(), right],
        )
    }

    fn unary(&self, operation: Unary) -> PyResult<Self> {
        Self::compose(
            Recipe::Unary(operation, self.recipe.clone()),
            std::slice::from_ref(self),
        )
    }
}

#[pymethods]
impl PyPipelineExpr {
    fn eq(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Eq, value)
    }
    fn ne(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Ne, value)
    }
    fn gt(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Gt, value)
    }
    fn gte(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Gte, value)
    }
    fn lt(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Lt, value)
    }
    fn lte(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Lte, value)
    }
    fn coalesce(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Coalesce, value)
    }

    fn __richcmp__(&self, value: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Self> {
        self.binary(
            match operation {
                CompareOp::Eq => Binary::Eq,
                CompareOp::Ne => Binary::Ne,
                CompareOp::Gt => Binary::Gt,
                CompareOp::Ge => Binary::Gte,
                CompareOp::Lt => Binary::Lt,
                CompareOp::Le => Binary::Lte,
            },
            value,
        )
    }
    fn __add__(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Add, value)
    }
    fn __sub__(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Sub, value)
    }
    fn __mul__(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Mul, value)
    }
    fn __truediv__(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Div, value)
    }
    fn __mod__(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Rem, value)
    }
    fn __and__(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::And, value)
    }
    fn __or__(&self, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.binary(Binary::Or, value)
    }
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "pipeline expressions cannot be tested as Python booleans",
        ))
    }
    fn __invert__(&self) -> PyResult<Self> {
        self.unary(Unary::Not)
    }
    fn __neg__(&self) -> PyResult<Self> {
        self.unary(Unary::Neg)
    }
    fn asc(&self) -> PyResult<Self> {
        self.unary(Unary::Asc)
    }
    fn desc(&self) -> PyResult<Self> {
        self.unary(Unary::Desc)
    }
    fn is_null(&self) -> PyResult<Self> {
        self.unary(Unary::IsNull)
    }
    fn is_not_null(&self) -> PyResult<Self> {
        self.unary(Unary::IsNotNull)
    }

    fn as_(&self, name: &Bound<'_, PyAny>) -> PyResult<Self> {
        Self::compose(
            Recipe::Named(self.recipe.clone(), alias_name(name)?),
            std::slice::from_ref(self),
        )
    }

    fn cast(&self, kind: &str) -> PyResult<Self> {
        let kind = match kind {
            "smallint" => pl::CastType::SmallInt,
            "integer" => pl::CastType::Integer,
            "bigint" => pl::CastType::BigInt,
            "real" => pl::CastType::Real,
            "double" | "float8" => pl::CastType::Double,
            "numeric" => pl::CastType::Numeric,
            "text" => pl::CastType::Text,
            "boolean" => pl::CastType::Boolean,
            "date" => pl::CastType::Date,
            "timestamp" => pl::CastType::Timestamp,
            "timestamptz" => pl::CastType::Timestamptz,
            "interval" => pl::CastType::Interval,
            "uuid" => pl::CastType::Uuid,
            "json" => pl::CastType::Json,
            "jsonb" => pl::CastType::Jsonb,
            _ => {
                return Err(UnsupportedCapabilityError::new_err(
                    "unsupported pipeline CastType",
                ));
            }
        };
        Self::compose(
            Recipe::Cast(self.recipe.clone(), kind),
            std::slice::from_ref(self),
        )
    }

    /// Compare this expression as the type `type_name`, through Rust's
    /// `ExprOps::compares_as`: the column and each value compared with it
    /// cast to the type, which a column whose domain is over an enum, or a
    /// `money` column compared with a `Decimal`, needs where no registration
    /// types it.
    // [spec:pgorm:req:bindings.pipeline-expressions+3]
    fn compares_as(&self, type_name: PyRef<'_, PyTypeName>) -> PyResult<Self> {
        Self::compose(
            Recipe::ComparesAs(self.recipe.clone(), type_name.rust_type()),
            std::slice::from_ref(self),
        )
    }

    fn in_array(&self, values: &Bound<'_, PyAny>) -> PyResult<Self> {
        if !values.is_instance_of::<PyList>() && !values.is_instance_of::<PyTuple>() {
            return Err(PyTypeError::new_err("pipeline IN requires a list or tuple"));
        }
        let mut members = values
            .try_iter()?
            .map(|value| Self::coerce(&value?))
            .collect::<PyResult<Vec<_>>>()?;
        let recipe = Recipe::In(
            self.recipe.clone(),
            members.iter().map(|value| value.recipe.clone()).collect(),
        );
        members.push(self.clone());
        Self::compose(recipe, &members)
    }

    fn __repr__(&self) -> &'static str {
        "PipelineExpr(...)"
    }
}

pub(super) fn alias_name(value: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(expression) = value.extract::<PyRef<'_, PyPipelineExpr>>() {
        expression.check()?;
        if let Recipe::Alias(name) = expression.recipe.as_ref() {
            return Ok(name.clone());
        }
        return Err(PyTypeError::new_err(
            "pipeline names require identifiers or alias tokens",
        ));
    }
    Ok(PyIdentifier::new(value)?.name)
}
