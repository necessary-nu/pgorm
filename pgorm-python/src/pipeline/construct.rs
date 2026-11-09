use pgorm::pgorm_query::Value;
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyBool, PyFloat, PyInt, PyList, PyString, PyTuple},
};

use super::{
    expression::{PyPipelineExpr, alias_name},
    recipe::{Function, Literal, Recipe},
};
use crate::{
    UnsupportedCapabilityError,
    entities::PyEntityColumn,
    errors::ConstructionError,
    schema::{PyModelColumnCasts, column_type},
    values::{PyValue, Tag, TagPy},
};
use std::sync::Arc;

pub(super) fn integer(value: &Bound<'_, PyAny>) -> PyResult<i64> {
    if !value.is_exact_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err(
            "pipeline integer argument requires an exact int",
        ));
    }
    value
        .extract::<i64>()
        .map_err(|_| ConstructionError::new_err("pipeline integer argument exceeds i64"))
}

/// `literal(value)`: a value written into the SQL as pgorm's pipeline
/// writes a literal — `None`, a `bool`, an integer within `bigint`, a
/// finite `float` or a `str`, plain or as a `Value` of that kind.
// [spec:pgorm:req:bindings.pipeline-expressions+3]
#[pyfunction]
pub(super) fn pipeline_literal(value: &Bound<'_, PyAny>) -> PyResult<PyPipelineExpr> {
    let literal = if value.is_none() {
        Literal::Null
    } else {
        let plain = [
            value.is_instance_of::<PyValue>(),
            value.is_exact_instance_of::<PyBool>(),
            value.is_exact_instance_of::<PyInt>(),
            value.is_exact_instance_of::<PyFloat>(),
            value.is_exact_instance_of::<PyString>(),
        ];
        if !plain.contains(&true) {
            return Err(not_literal());
        }
        let value = PyValue::coerce(value)?;
        match value.rust_value() {
            _ if value.has_named_type() => return Err(not_literal()),
            Value::Bool(Some(value)) => Literal::Bool(*value),
            Value::Int(Some(value)) => Literal::Integer(i64::from(*value)),
            Value::BigInt(Some(value)) => Literal::Integer(*value),
            Value::Double(Some(value)) if value.is_finite() => Literal::Float(*value),
            Value::String(Some(value)) => Literal::Text(value.to_string()),
            _ => return Err(not_literal()),
        }
    };
    PyPipelineExpr::compose(Recipe::Literal(literal), &[])
}

fn not_literal() -> PyErr {
    ConstructionError::new_err(
        "a pipeline literal is None, a bool, an integer within bigint, a finite float or a str; bind any other value",
    )
}

#[pyfunction]
pub(super) fn pipeline_alias(name: &Bound<'_, PyAny>) -> PyResult<PyPipelineExpr> {
    PyPipelineExpr::compose(Recipe::Alias(alias_name(name)?), &[])
}

/// `pl.col(table, column)`: a column qualified by the relation it is read
/// from. A registered entity's column (`entity.col(name)`) is the entity's
/// own, compared as the entity types it; any other name only qualifies.
// [spec:pgorm:req:bindings.pipeline-expressions+3]
// [spec:pgorm:req:bindings.pipeline-sources+3]
#[pyfunction]
pub(super) fn pipeline_col(
    table: &Bound<'_, PyAny>,
    column: &Bound<'_, PyAny>,
) -> PyResult<PyPipelineExpr> {
    let table = alias_name(table)?;
    let recipe = match column.extract::<PyRef<'_, PyEntityColumn>>() {
        Ok(registered) => Recipe::Registered(
            table,
            registered
                .entity
                .pipeline_column(&registered.name)
                .map_err(|error| ConstructionError::new_err(error.to_string()))?,
        ),
        Err(_) => Recipe::Column(table, alias_name(column)?),
    };
    PyPipelineExpr::compose(recipe, &[])
}

/// `pl.col(model, field)`: a declared model's field, the column its
/// declaration names under the qualifier the model is read by, typed by the
/// declaration through pgorm's `col_typed`, or through `col_shaped` by the
/// casts of a column declaring its own; `convert` is the declaration's
/// conversion of a value compared with it.
// [spec:pgorm:req:bindings.pipeline-models+1]
#[pyfunction]
#[pyo3(signature = (table, column, kind, array, values, convert, casts=None))]
pub(super) fn pipeline_model_col(
    table: &Bound<'_, PyAny>,
    column: &Bound<'_, PyAny>,
    kind: &Bound<'_, PyAny>,
    array: bool,
    values: Option<Vec<String>>,
    convert: Py<PyAny>,
    casts: Option<PyRef<'_, PyModelColumnCasts>>,
) -> PyResult<PyPipelineExpr> {
    let table = alias_name(table)?;
    let column = alias_name(column)?;
    let recipe = match casts {
        Some(casts) => Recipe::Shaped(table, column, Box::new(casts.column().clone())),
        None => {
            let tag = Tag::parse(kind)?;
            let tag = match array {
                true => Tag::Array(Box::new(tag)),
                false => tag,
            };
            Recipe::Typed(table, column, column_type(&tag, values.as_deref(), None)?)
        }
    };
    let mut expression = PyPipelineExpr::compose(recipe, &[])?;
    expression.declared = Some(Arc::new(convert));
    Ok(expression)
}

#[pyfunction]
pub(super) fn pipeline_role(role: &str, name: &Bound<'_, PyAny>) -> PyResult<PyPipelineExpr> {
    let name = alias_name(name)?;
    PyPipelineExpr::compose(
        match role {
            "this" => Recipe::This(name),
            "that" => Recipe::That(name),
            _ => {
                return Err(UnsupportedCapabilityError::new_err(
                    "unsupported pipeline relation role",
                ));
            }
        },
        &[],
    )
}

#[pyfunction]
#[pyo3(signature = (name, *arguments))]
pub(super) fn pipeline_function(
    name: &str,
    arguments: &Bound<'_, PyTuple>,
) -> PyResult<PyPipelineExpr> {
    if matches!(name, "count_rows" | "row_number") {
        if !arguments.is_empty() {
            return Err(ConstructionError::new_err("wrong pipeline function arity"));
        }
        return PyPipelineExpr::compose(
            if name == "count_rows" {
                Recipe::CountRows
            } else {
                Recipe::RowNumber
            },
            &[],
        );
    }
    let (function, arity) = match name {
        "sum" => (Function::Sum, 1),
        "min" => (Function::Min, 1),
        "max" => (Function::Max, 1),
        "average" => (Function::Average, 1),
        "stddev" => (Function::Stddev, 1),
        "count" => (Function::Count, 1),
        "count_distinct" => (Function::CountDistinct, 1),
        "rank" => (Function::Rank, 1),
        "rank_dense" => (Function::RankDense, 1),
        "first" => (Function::First, 1),
        "last" => (Function::Last, 1),
        "lag" | "lead" => {
            if arguments.len() != 2 {
                return Err(ConstructionError::new_err(
                    "lag/lead require an offset and an expression",
                ));
            }
            let offset = integer(&arguments.get_item(0)?)?;
            (
                if name == "lag" {
                    Function::Lag(offset)
                } else {
                    Function::Lead(offset)
                },
                2,
            )
        }
        _ => {
            return Err(UnsupportedCapabilityError::new_err(
                "unsupported pipeline function",
            ));
        }
    };
    if arguments.len() != arity {
        return Err(ConstructionError::new_err("wrong pipeline function arity"));
    }
    let operand = PyPipelineExpr::coerce(&arguments.get_item(arity - 1)?)?;
    PyPipelineExpr::compose(
        Recipe::Function(function, operand.recipe.clone()),
        &[operand],
    )
}

#[pyfunction]
pub(super) fn pipeline_case(
    arms: &Bound<'_, PyAny>,
    otherwise: &Bound<'_, PyAny>,
) -> PyResult<PyPipelineExpr> {
    if !arms.is_instance_of::<PyList>() && !arms.is_instance_of::<PyTuple>() {
        return Err(PyTypeError::new_err(
            "pipeline CASE arms require a list or tuple",
        ));
    }
    let fallback = PyPipelineExpr::coerce(otherwise)?;
    let mut operands = vec![fallback.clone()];
    let mut pairs = Vec::new();
    for arm in arms.try_iter()? {
        let arm = arm?;
        if (!arm.is_instance_of::<PyList>() && !arm.is_instance_of::<PyTuple>()) || arm.len()? != 2
        {
            return Err(PyTypeError::new_err(
                "each CASE arm is a (condition, value) pair",
            ));
        }
        let condition = PyPipelineExpr::coerce(&arm.get_item(0)?)?;
        let value = PyPipelineExpr::coerce(&arm.get_item(1)?)?;
        pairs.push((condition.recipe.clone(), value.recipe.clone()));
        operands.extend([condition, value]);
    }
    PyPipelineExpr::compose(Recipe::Case(pairs, fallback.recipe), &operands)
}
