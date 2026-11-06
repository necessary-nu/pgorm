//! Pipeline expressions from JavaScript: columns, introduced names, literals,
//! operators, casts, `CASE`, and the aggregate and window functions.

use std::sync::Arc;

use neon::{prelude::*, types::JsBigInt};
use pgorm::{pgorm_query::Name, pipeline as pl};

use super::{
    super::{
        Node,
        args::{arg, items, name, node, refuse},
    },
    Part, PlExpr, compose,
    recipe::{BINARY, FUNCTIONS, Function, Literal, Recipe, UNARY},
    scope::bindable,
};
use crate::{
    codec::Codec,
    disjoint::column_type,
    errors::throw,
    values::{Tag, read},
};

pub(super) const EXPORTS: &[(&str, super::super::Build)] = &[
    ("pipelineCol", col),
    ("pipelineRegisteredCol", registered_col),
    ("pipelineModelCol", model_col),
    ("pipelineDeclaredValue", declared_value),
    ("pipelineComparesAs", compares_as),
    ("pipelineAlias", alias),
    ("pipelineRole", role),
    ("pipelineLiteral", literal),
    ("pipelineBinary", binary),
    ("pipelineUnary", unary),
    ("pipelineAs", named),
    ("pipelineCast", cast),
    ("pipelineIn", membership),
    ("pipelineFunction", function),
    ("pipelineCase", case),
];

/// A pipeline expression an argument is, if it is one.
pub(super) fn expression<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> Option<PlExpr> {
    match node(cx, value) {
        Some(Node::Pipeline(part)) => match *part {
            Part::Expr(expr) => Some(expr),
            _ => None,
        },
        _ => None,
    }
}

/// An operand: a pipeline expression, or a value the stage that takes it
/// binds. A statement builder's expression is no pipeline expression.
// [spec:pgorm:req:bindings.pipeline-expressions+3]
// [spec:pgorm:req:bindings.operands]
pub(super) fn operand<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<PlExpr> {
    if let Some(expr) = expression(cx, value) {
        return Ok(expr);
    }
    if let Some(other) = node(cx, value) {
        let what = match other {
            Node::Pipeline(part) => part.describe(),
            other => other.describe(),
        };
        return refuse(
            cx,
            format!("expected a pipeline expression or a value, got {what}"),
        );
    }
    let value = bindable(cx, value)?;
    Ok(PlExpr {
        recipe: Arc::new(Recipe::Value(value)),
        scope: None,
    })
}

fn operand_at<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<PlExpr> {
    let value = arg(cx, index);
    operand(cx, value)
}

/// A name a pipeline writes: an identifier, or an `alias()` token's.
fn pipeline_name<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<Name> {
    if let Some(expr) = expression(cx, value) {
        if let Recipe::Alias(name) = expr.recipe.as_ref() {
            return Ok(name.clone());
        }
        return refuse(cx, "a name is an identifier or an alias()");
    }
    name(cx, value)
}

fn pipeline_name_at<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<Name> {
    let value = arg(cx, index);
    pipeline_name(cx, value)
}

/// `pipelineCol(table, column)`: a column qualified by the relation it is
/// read from, as prqlc, which has no catalog, needs.
// [spec:pgorm:req:bindings.pipeline-expressions+3]
fn col(cx: &mut FunctionContext) -> NeonResult<Node> {
    let table = pipeline_name_at(cx, 0)?;
    let column = pipeline_name_at(cx, 1)?;
    compose(cx, Recipe::Column(table, column), &[])
}

/// `pipelineRegisteredCol(table, entity, column)`: a registered entity's
/// column read under `table` — its own table, or the name a source was read
/// by — and compared as the entity types it, through `Named::col`.
// [spec:pgorm:req:bindings.pipeline-sources+3]
fn registered_col(cx: &mut FunctionContext) -> NeonResult<Node> {
    let table = pipeline_name_at(cx, 0)?;
    let column = crate::entities::pipeline_column(cx, 1, 2)?;
    compose(cx, Recipe::Registered(table, column), &[])
}

/// `pipelineModelCol(table, column, kind, array, values)`: a declared model's
/// field, the column its declaration names under the name its relation is
/// read by, typed by the declaration through pgorm's `col_typed`.
// [spec:pgorm:req:bindings.pipeline-models]
fn model_col(cx: &mut FunctionContext) -> NeonResult<Node> {
    let table = pipeline_name_at(cx, 0)?;
    let column = pipeline_name_at(cx, 1)?;
    let kind = arg(cx, 2);
    let codec = Codec::get(cx)?;
    let tag = read::kind(cx, codec, kind)?;
    let tag = match cx.argument::<JsBoolean>(3)?.value(cx) {
        true => Tag::Array(Box::new(tag)),
        false => tag,
    };
    let values = arg(cx, 4);
    let values = match values.is_a::<JsNull, _>(cx) {
        true => Vec::new(),
        false => items(cx, values)?
            .into_iter()
            .map(|label| read::string(cx, label))
            .collect::<NeonResult<Vec<_>>>()?,
    };
    let kind = column_type(&tag, &values).or_else(|failure| throw(cx, failure))?;
    compose(cx, Recipe::Typed(table, column, kind), &[])
}

/// `pipelineDeclaredValue(value)`: a value a declared model's column has
/// converted through its declaration, bound as the payload it holds, the
/// column's own cast typing it as `col_typed` casts a compared value.
// [spec:pgorm:req:bindings.pipeline-models]
fn declared_value(cx: &mut FunctionContext) -> NeonResult<Node> {
    let value = arg(cx, 0);
    let codec = Codec::get(cx)?;
    let Some(tagged) = read::infer(cx, codec, value)? else {
        return refuse(
            cx,
            "a model column's value is never null: test it with isNull()",
        );
    };
    compose(cx, Recipe::Value(tagged.datum), &[])
}

/// `pipelineComparesAs(expr, type)`: the expression compared as the type
/// `type` names, a string or a `TypeName`, through `ExprOps::compares_as`,
/// the column and each value compared with it cast to the type — what a
/// column whose domain is over an enum, or a `money` column compared with a
/// `Decimal`, needs where no registration types it.
// [spec:pgorm:req:bindings.pipeline-expressions+3]
fn compares_as(cx: &mut FunctionContext) -> NeonResult<Node> {
    let inner = operand_at(cx, 0)?;
    let named = arg(cx, 1);
    let named = super::super::args::type_name(cx, named)?;
    compose(
        cx,
        Recipe::ComparesAs(inner.recipe.clone(), named),
        &[&inner],
    )
}

/// `pipelineAlias(name)`: a name a stage introduces, read back unqualified.
fn alias(cx: &mut FunctionContext) -> NeonResult<Node> {
    let alias = pipeline_name_at(cx, 0)?;
    compose(cx, Recipe::Alias(alias), &[])
}

/// `pipelineRole("this" | "that", column)`: a join condition's side by role.
fn role(cx: &mut FunctionContext) -> NeonResult<Node> {
    let side = super::super::args::choice(cx, 0, "a side", &["this", "that"])?;
    let column = pipeline_name_at(cx, 1)?;
    let recipe = if side == "this" {
        Recipe::This(column)
    } else {
        Recipe::That(column)
    };
    compose(cx, recipe, &[])
}

/// `pipelineLiteral(value)`: a value written into the SQL as pgorm's
/// pipeline writes a literal, which only `literal()` asks for.
// [spec:pgorm:req:bindings.pipeline-expressions+3]
fn literal(cx: &mut FunctionContext) -> NeonResult<Node> {
    let value = arg(cx, 0);
    let literal = if value.is_a::<JsNull, _>(cx) {
        Some(Literal::Null)
    } else if let Ok(flag) = value.downcast::<JsBoolean, _>(cx) {
        Some(Literal::Bool(flag.value(cx)))
    } else if let Ok(number) = value.downcast::<JsNumber, _>(cx) {
        let number = number.value(cx);
        #[allow(clippy::cast_possible_truncation)]
        if number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0 {
            Some(Literal::Integer(number as i64))
        } else {
            number.is_finite().then_some(Literal::Float(number))
        }
    } else if let Ok(big) = value.downcast::<JsBigInt, _>(cx) {
        big.to_i64(cx).ok().map(Literal::Integer)
    } else if value.is_a::<JsString, _>(cx) {
        Some(Literal::Text(read::sql(cx, value)?))
    } else {
        return cx.throw_type_error(
            "a literal is null, a boolean, a number, a bigint or a string; bind any other value",
        );
    };
    match literal {
        Some(literal) => compose(cx, Recipe::Literal(literal), &[]),
        None => refuse(
            cx,
            "a literal number is finite, and a literal bigint within PostgreSQL's bigint",
        ),
    }
}

/// `pipelineBinary(expr, operator, operand)`.
fn binary(cx: &mut FunctionContext) -> NeonResult<Node> {
    let left = operand_at(cx, 0)?;
    let operator = arg(cx, 1);
    let operator = read::string(cx, operator)?;
    let right = operand_at(cx, 2)?;
    match BINARY.iter().find(|(spelled, _)| *spelled == operator) {
        Some((_, operator)) => compose(
            cx,
            Recipe::Binary(*operator, left.recipe.clone(), right.recipe.clone()),
            &[&left, &right],
        ),
        None => refuse(cx, format!("{operator:?} is no pipeline operator")),
    }
}

/// `pipelineUnary(expr, operator)`.
fn unary(cx: &mut FunctionContext) -> NeonResult<Node> {
    let inner = operand_at(cx, 0)?;
    let operator = arg(cx, 1);
    let operator = read::string(cx, operator)?;
    match UNARY.iter().find(|(spelled, _)| *spelled == operator) {
        Some((_, operator)) => compose(
            cx,
            Recipe::Unary(*operator, inner.recipe.clone()),
            &[&inner],
        ),
        None => refuse(cx, format!("{operator:?} is no pipeline operator")),
    }
}

/// `pipelineAs(expr, name)`: a projected expression's name, an identifier or
/// an `alias()`.
fn named(cx: &mut FunctionContext) -> NeonResult<Node> {
    let inner = operand_at(cx, 0)?;
    let name = pipeline_name_at(cx, 1)?;
    compose(cx, Recipe::Named(inner.recipe.clone(), name), &[&inner])
}

/// The types pgorm's pipeline casts to, a closed set because the name reaches
/// the SQL as written.
const CASTS: &[(&str, pl::CastType)] = &[
    ("smallint", pl::CastType::SmallInt),
    ("integer", pl::CastType::Integer),
    ("bigint", pl::CastType::BigInt),
    ("real", pl::CastType::Real),
    ("double", pl::CastType::Double),
    ("float8", pl::CastType::Double),
    ("numeric", pl::CastType::Numeric),
    ("text", pl::CastType::Text),
    ("boolean", pl::CastType::Boolean),
    ("date", pl::CastType::Date),
    ("timestamp", pl::CastType::Timestamp),
    ("timestamptz", pl::CastType::Timestamptz),
    ("interval", pl::CastType::Interval),
    ("uuid", pl::CastType::Uuid),
    ("json", pl::CastType::Json),
    ("jsonb", pl::CastType::Jsonb),
];

/// `pipelineCast(expr, type)`.
// [spec:pgorm:req:bindings.pipeline-expressions+3]
fn cast(cx: &mut FunctionContext) -> NeonResult<Node> {
    let inner = operand_at(cx, 0)?;
    let kind = arg(cx, 1);
    let names = || {
        let names: Vec<&str> = CASTS.iter().map(|(name, _)| *name).collect();
        format!("a pipeline casts to one of {}", names.join(", "))
    };
    if !kind.is_a::<JsString, _>(cx) {
        return cx.throw_type_error(names());
    }
    let spelled = read::string(cx, kind)?;
    match CASTS.iter().find(|(name, _)| *name == spelled) {
        Some((_, kind)) => compose(cx, Recipe::Cast(inner.recipe.clone(), *kind), &[&inner]),
        None => refuse(cx, names()),
    }
}

/// `pipelineIn(expr, items)`: membership in an explicit list.
fn membership(cx: &mut FunctionContext) -> NeonResult<Node> {
    let inner = operand_at(cx, 0)?;
    let list = arg(cx, 1);
    let list = items(cx, list)?;
    let mut members = Vec::with_capacity(list.len());
    for item in list {
        members.push(operand(cx, item)?);
    }
    let recipe = Recipe::In(
        inner.recipe.clone(),
        members.iter().map(|member| member.recipe.clone()).collect(),
    );
    let mut operands: Vec<&PlExpr> = members.iter().collect();
    operands.push(&inner);
    compose(cx, recipe, &operands)
}

/// An offset for `lag` or `lead`: a safe integer.
fn offset(cx: &mut FunctionContext, index: usize) -> NeonResult<i64> {
    let value = arg(cx, index);
    let Ok(offset) = value.downcast::<JsNumber, _>(cx) else {
        return cx.throw_type_error("an offset is a number");
    };
    let offset = offset.value(cx);
    if offset.fract() == 0.0 && offset.abs() <= 9_007_199_254_740_991.0 {
        #[allow(clippy::cast_possible_truncation)]
        Ok(offset as i64)
    } else {
        refuse(cx, "an offset is a whole number")
    }
}

/// `pipelineFunction(name, ..operands)`: the aggregates and window functions
/// pgorm's pipeline has, at the argument counts each takes.
// [spec:pgorm:req:bindings.pipeline-expressions+3]
fn function(cx: &mut FunctionContext) -> NeonResult<Node> {
    let name = arg(cx, 0);
    let name = read::string(cx, name)?;
    let arity = cx.len().saturating_sub(1);
    let (function, operand_index) = match (name.as_str(), arity) {
        ("countRows", 0) => return compose(cx, Recipe::CountRows, &[]),
        ("rowNumber", 0) => return compose(cx, Recipe::RowNumber, &[]),
        ("lag", 2) => (Function::Lag(offset(cx, 1)?), 2),
        ("lead", 2) => (Function::Lead(offset(cx, 1)?), 2),
        (name, 1) => match FUNCTIONS.iter().find(|(spelled, _)| *spelled == name) {
            Some((_, function)) => (*function, 1),
            None => {
                return refuse(
                    cx,
                    format!("{name:?} is no pipeline function of one argument"),
                );
            }
        },
        (name, count) => {
            return refuse(
                cx,
                format!("{name:?} is no pipeline function of {count} argument(s)"),
            );
        }
    };
    let inner = operand_at(cx, operand_index)?;
    compose(
        cx,
        Recipe::Function(function, inner.recipe.clone()),
        &[&inner],
    )
}

/// `pipelineCase(arms, otherwise)`: `[condition, value]` arms tried in
/// order, and the fallback pgorm's pipeline requires.
fn case(cx: &mut FunctionContext) -> NeonResult<Node> {
    let arms = arg(cx, 0);
    let arms = items(cx, arms)?;
    let otherwise = operand_at(cx, 1)?;
    let mut parts = Vec::with_capacity(arms.len());
    for arm in arms {
        let pair = items(cx, arm)?;
        let [condition, value] = <[_; 2]>::try_from(pair)
            .or_else(|_| cx.throw_type_error("a CASE arm is [condition, value]"))?;
        parts.push((operand(cx, condition)?, operand(cx, value)?));
    }
    let recipe = Recipe::Case(
        parts
            .iter()
            .map(|(condition, value)| (condition.recipe.clone(), value.recipe.clone()))
            .collect(),
        otherwise.recipe.clone(),
    );
    let mut operands: Vec<&PlExpr> = parts
        .iter()
        .flat_map(|(condition, value)| [condition, value])
        .collect();
    operands.push(&otherwise);
    compose(cx, recipe, &operands)
}
