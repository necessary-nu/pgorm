//! What the builder exports read from their JavaScript arguments: the node a
//! builder object boxes, an identifier, an operand that is an expression or a
//! value to bind, a predicate, an ordering.
//!
//! Every refusal is thrown as the builder method is called, so a statement
//! that cannot be built never exists to be run: a `TypeError` for an
//! argument of a type the method does not take, and a `ConstructionError`
//! for one whose value cannot become what pgorm sends.

use neon::{prelude::*, types::JsBigInt};
use pgorm::pgorm_query::{
    Condition, Expr, IntoCondition, Name, SelectStatement, SimpleExpr, TypeName,
};

use super::{Node, Ordering};
pub(crate) use crate::values::read::refuse;
use crate::{
    codec::Codec,
    values::{self, Tag, read},
};

/// A copy of the node `value` boxes, if it is a builder object's native
/// half.
pub(crate) fn node<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> Option<Node> {
    value
        .downcast::<JsBox<Node>, _>(cx)
        .ok()
        .map(|boxed| (**boxed).clone())
}

/// The node at `index`: the receiver of a method, or an argument that must
/// be a builder object.
pub(crate) fn this<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<Node> {
    let value = arg(cx, index);
    match node(cx, value) {
        Some(node) => Ok(node),
        None => cx.throw_type_error("expected a statement or expression this module built"),
    }
}

/// The argument at `index`, `undefined` past the end.
pub(crate) fn arg<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> Handle<'cx, JsValue> {
    cx.argument_opt(index)
        .unwrap_or_else(|| cx.undefined().upcast())
}

/// Whether `value` is `undefined` or `null`, an option left out.
pub(crate) fn absent<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> bool {
    value.is_a::<JsUndefined, _>(cx) || value.is_a::<JsNull, _>(cx)
}

/// The items of the array at `index`.
pub(crate) fn list<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<Vec<Handle<'cx, JsValue>>> {
    let value = arg(cx, index);
    items(cx, value)
}

/// The items of `value`, which must be an array.
pub(crate) fn items<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<Vec<Handle<'cx, JsValue>>> {
    match value.downcast::<JsArray, _>(cx) {
        Ok(array) => array.to_vec(cx),
        Err(_) => cx.throw_type_error("expected an array"),
    }
}

/// An identifier: a JavaScript string of 1–63 UTF-8 bytes without NUL,
/// minted into a `Name`, which renders quoted — never SQL.
// [spec:pgorm:req:bindings.identifiers+2]
pub(crate) fn name<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<Name> {
    let text = read::text(cx, value, "an identifier")?;
    if text.is_empty() || text.len() > 63 || text.contains('\0') {
        return refuse(cx, "an identifier is 1–63 UTF-8 bytes without NUL");
    }
    Ok(Name::runtime(text))
}

/// The identifier at `index`.
pub(crate) fn name_at<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<Name> {
    let value = arg(cx, index);
    name(cx, value)
}

/// An identifier or nothing: `undefined` and `null` are no name.
pub(crate) fn optional_name<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<Option<Name>> {
    if absent(cx, value) {
        Ok(None)
    } else {
        name(cx, value).map(Some)
    }
}

/// A type named by identifier, for a cast: a string, or the module's
/// `TypeName`, schema-qualified.
pub(crate) fn type_name<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<TypeName> {
    if value.is_a::<JsString, _>(cx) {
        return Ok(TypeName::new(name(cx, value)?));
    }
    let codec = Codec::get(cx)?;
    match read::named_type(cx, codec, value)? {
        Some(named) => Ok(rust_type(&named)),
        None => cx.throw_type_error("a type to cast to is a string or a TypeName"),
    }
}

/// The Rust `TypeName` of a binding `TypeName`.
pub(crate) fn rust_type(named: &values::TypeName) -> TypeName {
    let mut name = TypeName::new(Name::runtime(&named.name));
    name.schema = named.schema.as_ref().map(Name::runtime);
    name
}

/// A value to bind: anything a parameter takes, inferred or declared with
/// `Value`, written as its kind needs — an enum's label cast to the enum, a
/// created range's text cast to its range type through `Expr::as_range`.
/// Nothing here interpolates: the value is a placeholder's.
// [spec:pgorm:req:bindings.expressions+2]
pub(crate) fn bound<'cx>(cx: &mut Cx<'cx>, data: Handle<'cx, JsValue>) -> NeonResult<SimpleExpr> {
    let codec = Codec::get(cx)?;
    let Some(tagged) = read::infer(cx, codec, data)? else {
        return refuse(
            cx,
            "null has no kind to bind in a built statement: test with isNull(), or bind \
             Value.null(kind) for a typed NULL",
        );
    };
    Ok(typed(Expr::value(tagged.datum), tagged.tag))
}

/// A value written into the SQL text by pgorm-query's own rendering of a
/// constant, read as `bound` reads it and cast as its kind needs. Only
/// `literal()` asks for one; every other operand is bound.
// [spec:pgorm:req:bindings.operands]
pub(crate) fn literal<'cx>(cx: &mut Cx<'cx>, data: Handle<'cx, JsValue>) -> NeonResult<SimpleExpr> {
    let codec = Codec::get(cx)?;
    let Some(tagged) = read::infer(cx, codec, data)? else {
        return refuse(
            cx,
            "null has no kind to write: write literal(Value.null(kind)) for a typed NULL",
        );
    };
    Ok(typed(SimpleExpr::Constant(tagged.datum), tagged.tag))
}

/// `expr` cast as a value of `tag` must be: an enum's label to the enum, a
/// created range's text to its range type through `Expr::as_range`, and an
/// array of either to the type's array.
fn typed(expr: SimpleExpr, tag: Tag) -> SimpleExpr {
    match tag {
        Tag::Enum(named) => expr.cast_as_type(rust_type(&named)),
        Tag::Array(element) => match *element {
            Tag::Enum(named) => expr.cast_as_type(rust_type(&named).array()),
            Tag::Created(kind) => Expr::expr(expr).as_range(rust_type(&kind.name).array()),
            _ => expr,
        },
        Tag::Created(kind) => Expr::expr(expr).as_range(rust_type(&kind.name)),
        Tag::Scalar(_) => expr,
    }
}

/// An operand: an expression a builder made, or a value to bind.
pub(crate) fn operand<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<SimpleExpr> {
    match node(cx, value) {
        Some(node) => match node {
            Node::Expr(expr) => Ok(expr),
            other => {
                let what = other.describe();
                refuse(cx, format!("expected an expression or a value, got {what}"))
            }
        },
        None => bound(cx, value),
    }
}

/// The operand at `index`.
pub(crate) fn operand_at<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<SimpleExpr> {
    let value = arg(cx, index);
    operand(cx, value)
}

/// The operands of the array at `index`.
pub(crate) fn operands_at<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<Vec<SimpleExpr>> {
    let values = list(cx, index)?;
    values.into_iter().map(|value| operand(cx, value)).collect()
}

/// An expression a builder made; a plain value is refused.
pub(crate) fn expression<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<SimpleExpr> {
    match node(cx, value) {
        Some(Node::Expr(expr)) => Ok(expr),
        Some(other) => {
            let what = other.describe();
            cx.throw_type_error(format!("expected an expression, got {what}"))
        }
        None => cx.throw_type_error("expected an expression"),
    }
}

/// The expressions of the array at `index`.
pub(crate) fn expressions_at<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<Vec<SimpleExpr>> {
    let values = list(cx, index)?;
    values
        .into_iter()
        .map(|value| expression(cx, value))
        .collect()
}

/// A predicate: a `Condition`, or an expression.
pub(crate) fn predicate<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<Condition> {
    match node(cx, value) {
        Some(Node::Condition(condition)) => Ok(condition),
        Some(Node::Expr(expr)) => Ok(expr.into_condition()),
        Some(other) => {
            let what = other.describe();
            cx.throw_type_error(format!("expected an expression or a Condition, got {what}"))
        }
        None => cx.throw_type_error("expected an expression or a Condition"),
    }
}

/// The predicate at `index`.
pub(crate) fn predicate_at<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<Condition> {
    let value = arg(cx, index);
    predicate(cx, value)
}

/// An ordering from `asc()` or `desc()`.
pub(crate) fn ordering<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<Ordering> {
    match node(cx, value) {
        Some(Node::Ordering(ordering)) => Ok(ordering),
        _ => cx.throw_type_error("an ordering is an expression's asc() or desc()"),
    }
}

/// The orderings of the array at `index`.
pub(crate) fn orderings_at<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<Vec<Ordering>> {
    let values = list(cx, index)?;
    values
        .into_iter()
        .map(|value| ordering(cx, value))
        .collect()
}

/// A `Select`, for a subquery or a set operation's operand.
pub(crate) fn select<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<SelectStatement> {
    let select = unchecked(cx, value)?;
    embedded(cx, select)
}

/// A `Select` as it was built, with neither of `embedded`'s checks run: only
/// for a taker whose pgorm-query constructor runs both itself, in the same
/// order and with the same errors, so the refusal is the one `embedded`
/// would give.
fn unchecked<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<SelectStatement> {
    match node(cx, value) {
        Some(Node::Select(select)) => Ok(select),
        Some(other) => {
            let what = other.describe();
            cx.throw_type_error(format!("expected a Select, got {what}"))
        }
        None => cx.throw_type_error("expected a Select"),
    }
}

/// `select`, read into another statement: final from here on, so a tie
/// limit PostgreSQL's grammar refuses — with no ORDER BY, or beside SKIP
/// LOCKED — and a WINDOW clause it refuses are refused here, as `compile`
/// refuses them in a statement run on its own, rather than by the server
/// once the outer statement is sent.
// [spec:pgorm:req:bindings.select+3]
// [spec:pgorm:req:bindings.windows]
pub(crate) fn embedded<'cx>(
    cx: &mut Cx<'cx>,
    select: SelectStatement,
) -> NeonResult<SelectStatement> {
    match select.check_clauses() {
        Ok(()) => Ok(select),
        Err(error) => refuse(cx, error.to_string()),
    }
}

/// The `Select` at `index`.
pub(crate) fn select_at<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<SelectStatement> {
    let value = arg(cx, index);
    select(cx, value)
}

/// The `Select` at `index` as a stored definition's body: a view's, a
/// materialized view's or a `CREATE TABLE AS`'s, whose pgorm-query
/// constructor checks it.
pub(crate) fn definition_at<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<SelectStatement> {
    let value = arg(cx, index);
    unchecked(cx, value)
}

/// The string at `index`, one of `choices`.
pub(crate) fn choice<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
    what: &str,
    choices: &[&'static str],
) -> NeonResult<&'static str> {
    let value = arg(cx, index);
    let spelled: Vec<(&str, &'static str)> =
        choices.iter().map(|choice| (*choice, *choice)).collect();
    pick(cx, value, what, &spelled)
}

/// `value`, a string that is one of `choices`, given as `(spelling,
/// meaning)`. A closed choice is a type, a union of string literals as
/// Python's is a class of constants, so anything outside it is a
/// `TypeError`.
// [spec:pgorm:req:bindings.argument-errors+1]
pub(crate) fn pick<'cx, T: Clone>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
    what: &str,
    choices: &[(&str, T)],
) -> NeonResult<T> {
    let text = if value.is_a::<JsString, _>(cx) {
        Some(read::string(cx, value)?)
    } else {
        None
    };
    match text.and_then(|text| choices.iter().find(|(spelling, _)| *spelling == text)) {
        Some((_, meaning)) => Ok(meaning.clone()),
        None => {
            let quoted: Vec<String> = choices
                .iter()
                .map(|(spelling, _)| format!("{spelling:?}"))
                .collect();
            cx.throw_type_error(format!("{what} is one of {}", quoted.join(", ")))
        }
    }
}

/// The boolean at `index`.
pub(crate) fn flag<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<bool> {
    let value = arg(cx, index);
    match value.downcast::<JsBoolean, _>(cx) {
        Ok(flag) => Ok(flag.value(cx)),
        Err(_) => cx.throw_type_error("expected a boolean"),
    }
}

/// A row count for LIMIT or OFFSET: a non-negative safe-integer number or a
/// `bigint`, within PostgreSQL's signed `bigint`.
pub(crate) fn count<'cx>(cx: &mut FunctionContext<'cx>, index: usize) -> NeonResult<u64> {
    let value = arg(cx, index);
    if !value.is_a::<JsNumber, _>(cx) && !value.is_a::<JsBigInt, _>(cx) {
        return cx.throw_type_error("a row count is a number or a bigint");
    }
    let counted = if let Ok(number) = value.downcast::<JsNumber, _>(cx) {
        let number = number.value(cx);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        (number.fract() == 0.0 && (0.0..=9_007_199_254_740_991.0).contains(&number))
            .then_some(number as u64)
    } else if let Ok(big) = value.downcast::<JsBigInt, _>(cx) {
        big.to_u64(cx)
            .ok()
            .filter(|count| i64::try_from(*count).is_ok())
    } else {
        None
    };
    match counted {
        Some(count) => Ok(count),
        None => refuse(
            cx,
            "a row count is a non-negative integer number or bigint within PostgreSQL's bigint",
        ),
    }
}
