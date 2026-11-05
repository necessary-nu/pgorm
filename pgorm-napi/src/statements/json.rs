//! SQL/JSON's query functions, constructors, `FORMAT JSON` and `IS JSON`
//! over pgorm-query's builders. Each option is applied through the builder's
//! own method, and each function's behaviours are its own, so a choice
//! PostgreSQL refuses for a function cannot be given to it.

use neon::prelude::*;
use pgorm::pgorm_query::{
    Expr, Func, JsonExistsBehavior, JsonInput, JsonKind, JsonQueryBehavior, JsonTest,
    JsonValueBehavior, JsonValueType, Name,
};

use super::{
    Node,
    args::{
        absent, arg, choice, flag, items, name, node, operand, operand_at, orderings_at, pick,
        predicate, refuse, select_at,
    },
    data_type,
};
use crate::{
    codec::Codec,
    values::{Tag, read},
};

pub(super) const EXPORTS: &[(&str, super::Build)] = &[
    ("jsonFormat", format_json),
    ("jsonDefault", json_default),
    ("jsonIs", is_json),
    ("jsonExists", json_exists),
    ("jsonValue", json_value),
    ("jsonQuery", json_query),
    ("jsonObject", json_object),
    ("jsonArray", json_array),
    ("jsonArrayQuery", json_array_query),
    ("jsonObjectagg", json_object_agg),
    ("jsonArrayagg", json_array_agg),
    ("jsonParse", json_parse),
    ("jsonScalar", json_scalar),
    ("jsonSerialize", json_serialize),
];

/// An operand in a position SQL/JSON reads as JSON: a `formatJson(..)`, or
/// any operand, unformatted.
pub(super) fn input<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<JsonInput> {
    match node(cx, value) {
        Some(Node::JsonInput(input)) => Ok(input),
        _ => Ok(operand(cx, value)?.into()),
    }
}

fn input_at(cx: &mut FunctionContext, index: usize) -> NeonResult<JsonInput> {
    let value = arg(cx, index);
    input(cx, value)
}

/// `PASSING` variables: `[name, value]` pairs, each name an identifier.
pub(super) fn variables(
    cx: &mut FunctionContext,
    index: usize,
) -> NeonResult<Vec<(JsonInput, Name)>> {
    let value = arg(cx, index);
    if absent(cx, value) {
        return Ok(Vec::new());
    }
    let mut passing = Vec::new();
    for pair in items(cx, value)? {
        let pair = items(cx, pair)?;
        let (Some(variable), Some(value)) = (pair.first().copied(), pair.get(1).copied()) else {
            return cx.throw_type_error("a PASSING variable is a name and a value");
        };
        let variable = name(cx, variable)?;
        passing.push((input(cx, value)?, variable));
    }
    Ok(passing)
}

/// The path at `index`: a string, which pgorm-query binds.
pub(super) fn path(cx: &mut FunctionContext, index: usize) -> NeonResult<String> {
    let value = arg(cx, index);
    read::text(cx, value, "a JSON path")
}

fn returning(
    cx: &mut FunctionContext,
    index: usize,
) -> NeonResult<Option<pgorm::pgorm_query::ColumnType>> {
    let value = arg(cx, index);
    if absent(cx, value) {
        Ok(None)
    } else {
        data_type::column_type(cx, value).map(Some)
    }
}

/// `jsonFormat(operand)`: the operand marked as JSON text, `FORMAT JSON`,
/// accepted only where SQL/JSON reads JSON.
// [spec:pgorm:req:bindings.sql-json]
fn format_json(cx: &mut FunctionContext) -> NeonResult<Node> {
    let operand = operand_at(cx, 0)?;
    Ok(Node::JsonInput(Expr::expr(operand).format_json()))
}

/// `jsonDefault(value)`: `DEFAULT value`, which pgorm-query writes as an
/// escaped literal, PostgreSQL refusing a parameter there. A value whose type
/// a cast would carry cannot keep it in a literal, and is refused.
// [spec:pgorm:req:bindings.sql-json]
fn json_default(cx: &mut FunctionContext) -> NeonResult<Node> {
    let data = arg(cx, 0);
    let codec = Codec::get(cx)?;
    let Some(tagged) = read::infer(cx, codec, data)? else {
        return refuse(cx, "a JSON DEFAULT is a value: null has no kind to write");
    };
    let cast = match &tagged.tag {
        Tag::Enum(_) | Tag::Created(_) => true,
        Tag::Array(element) => matches!(**element, Tag::Created(_)),
        Tag::Scalar(_) => false,
    };
    if cast {
        return refuse(
            cx,
            "a JSON DEFAULT is written as a plain literal, which cannot carry an enum's or a \
             created range's cast",
        );
    }
    Ok(Node::JsonDefault(tagged.datum))
}

/// `jsonIs(operand, negated, kind, uniqueKeys)`: `IS [NOT] JSON [VALUE |
/// SCALAR | ARRAY | OBJECT] [WITH UNIQUE KEYS]`.
// [spec:pgorm:req:bindings.sql-json]
fn is_json(cx: &mut FunctionContext) -> NeonResult<Node> {
    let operand = Expr::expr(operand_at(cx, 0)?);
    let negated = flag(cx, 1)?;
    let kind = match choice(
        cx,
        2,
        "a JSON kind",
        &["value", "scalar", "array", "object"],
    )? {
        "value" => JsonKind::Value,
        "scalar" => JsonKind::Scalar,
        "array" => JsonKind::Array,
        _ => JsonKind::Object,
    };
    let test: JsonTest = if flag(cx, 3)? {
        kind.with_unique_keys()
    } else {
        kind.into()
    };
    Ok(Node::Expr(if negated {
        operand.is_not_json(test)
    } else {
        operand.is_json(test)
    }))
}

/// `JSON_EXISTS`'s behaviours by the names JavaScript spells them, which a
/// `JSON_TABLE`'s EXISTS column takes too.
pub(super) const EXISTS_BEHAVIORS: &[(&str, JsonExistsBehavior)] = &[
    ("true", JsonExistsBehavior::True),
    ("false", JsonExistsBehavior::False),
    ("unknown", JsonExistsBehavior::Unknown),
    ("error", JsonExistsBehavior::Error),
];

/// `JSON_VALUE`'s keyword behaviours; a `jsonDefault` is its DEFAULT.
pub(super) const VALUE_BEHAVIORS: &[(&str, JsonValueBehavior)] = &[
    ("null", JsonValueBehavior::Null),
    ("error", JsonValueBehavior::Error),
];

/// `JSON_QUERY`'s keyword behaviours; a `jsonDefault` is its DEFAULT.
pub(super) const QUERY_BEHAVIORS: &[(&str, JsonQueryBehavior)] = &[
    ("null", JsonQueryBehavior::Null),
    ("error", JsonQueryBehavior::Error),
    ("emptyArray", JsonQueryBehavior::EmptyArray),
    ("emptyObject", JsonQueryBehavior::EmptyObject),
];

/// `JSON_EXISTS`'s `ON ERROR`, which has no DEFAULT.
fn exists_behavior(
    cx: &mut FunctionContext,
    index: usize,
) -> NeonResult<Option<JsonExistsBehavior>> {
    let value = arg(cx, index);
    if absent(cx, value) {
        return Ok(None);
    }
    pick(cx, value, "JSON_EXISTS's behaviour", EXISTS_BEHAVIORS).map(Some)
}

/// `JSON_VALUE`'s `ON EMPTY` or `ON ERROR`: a keyword or a `jsonDefault`.
pub(super) fn value_behavior(
    cx: &mut FunctionContext,
    index: usize,
) -> NeonResult<Option<JsonValueBehavior>> {
    let value = arg(cx, index);
    if absent(cx, value) {
        return Ok(None);
    }
    if let Some(Node::JsonDefault(default)) = node(cx, value) {
        return Ok(Some(JsonValueBehavior::Default(default)));
    }
    pick(cx, value, "JSON_VALUE's behaviour", VALUE_BEHAVIORS).map(Some)
}

/// `JSON_QUERY`'s `ON EMPTY` or `ON ERROR`: a keyword or a `jsonDefault`.
pub(super) fn query_behavior(
    cx: &mut FunctionContext,
    index: usize,
) -> NeonResult<Option<JsonQueryBehavior>> {
    let value = arg(cx, index);
    if absent(cx, value) {
        return Ok(None);
    }
    if let Some(Node::JsonDefault(default)) = node(cx, value) {
        return Ok(Some(JsonQueryBehavior::Default(default)));
    }
    pick(cx, value, "JSON_QUERY's behaviour", QUERY_BEHAVIORS).map(Some)
}

/// `JSON_QUERY`'s one shaping slot: PostgreSQL refuses `OMIT QUOTES` beside a
/// wrapper.
pub(super) fn shaping(cx: &mut FunctionContext, index: usize) -> NeonResult<Option<&'static str>> {
    let value = arg(cx, index);
    if absent(cx, value) {
        return Ok(None);
    }
    choice(
        cx,
        index,
        "shaping",
        &["withWrapper", "withConditionalWrapper", "omitQuotes"],
    )
    .map(Some)
}

/// `jsonExists(context, path, passing, onError)`.
// [spec:pgorm:req:bindings.sql-json]
fn json_exists(cx: &mut FunctionContext) -> NeonResult<Node> {
    let context = input_at(cx, 0)?;
    let path = path(cx, 1)?;
    let mut json = Func::json_exists(context, path);
    for (value, name) in variables(cx, 2)? {
        json = json.passing(value, name);
    }
    if let Some(behavior) = exists_behavior(cx, 3)? {
        json = json.on_error(behavior);
    }
    Ok(Node::Expr(json.into()))
}

/// `jsonValue(context, path, passing, returning, onEmpty, onError)`.
// [spec:pgorm:req:bindings.sql-json]
fn json_value(cx: &mut FunctionContext) -> NeonResult<Node> {
    let context = input_at(cx, 0)?;
    let path = path(cx, 1)?;
    let mut json = Func::json_value(context, path);
    for (value, name) in variables(cx, 2)? {
        json = json.passing(value, name);
    }
    if let Some(column) = returning(cx, 3)? {
        let Ok(scalar) = JsonValueType::try_from(column) else {
            return refuse(
                cx,
                "JSON_VALUE cannot return json or jsonb: PostgreSQL 18.6 returns NULL for every \
                 later row once one is NULL (bug #19695); use jsonQuery",
            );
        };
        json = json.returning(scalar);
    }
    if let Some(behavior) = value_behavior(cx, 4)? {
        json = json.on_empty(behavior);
    }
    if let Some(behavior) = value_behavior(cx, 5)? {
        json = json.on_error(behavior);
    }
    Ok(Node::Expr(json.into()))
}

/// `jsonQuery(context, path, passing, returning, shaping, onEmpty, onError)`.
// [spec:pgorm:req:bindings.sql-json]
fn json_query(cx: &mut FunctionContext) -> NeonResult<Node> {
    let context = input_at(cx, 0)?;
    let path = path(cx, 1)?;
    let mut json = Func::json_query(context, path);
    for (value, name) in variables(cx, 2)? {
        json = json.passing(value, name);
    }
    if let Some(column) = returning(cx, 3)? {
        json = json.returning(column);
    }
    json = match shaping(cx, 4)? {
        None => json,
        Some("withWrapper") => json.with_wrapper(),
        Some("withConditionalWrapper") => json.with_conditional_wrapper(),
        Some(_) => json.omit_quotes(),
    };
    if let Some(behavior) = query_behavior(cx, 5)? {
        json = json.on_empty(behavior);
    }
    if let Some(behavior) = query_behavior(cx, 6)? {
        json = json.on_error(behavior);
    }
    Ok(Node::Expr(json.into()))
}

/// `jsonObject(entries, absentOnNull, uniqueKeys, returning)`: each entry a
/// `[key, value]` pair, a key an operand — a string key is bound — and a value
/// read as JSON.
// [spec:pgorm:req:bindings.sql-json]
fn json_object(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut json = Func::json_object();
    let entries = arg(cx, 0);
    for entry in items(cx, entries)? {
        let pair = items(cx, entry)?;
        let (Some(key), Some(value)) = (pair.first().copied(), pair.get(1).copied()) else {
            return cx.throw_type_error("a JSON object's entry is a key and a value");
        };
        let key = operand(cx, key)?;
        json = json.entry(key, input(cx, value)?);
    }
    if flag(cx, 1)? {
        json = json.absent_on_null();
    }
    if flag(cx, 2)? {
        json = json.with_unique_keys();
    }
    if let Some(column) = returning(cx, 3)? {
        json = json.returning(column);
    }
    Ok(Node::Expr(json.into()))
}

/// `jsonArray(elements, nullOnNull, returning)`.
fn json_array(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut json = Func::json_array();
    let elements = arg(cx, 0);
    for element in items(cx, elements)? {
        json = json.element(input(cx, element)?);
    }
    if flag(cx, 1)? {
        json = json.null_on_null();
    }
    if let Some(column) = returning(cx, 2)? {
        json = json.returning(column);
    }
    Ok(Node::Expr(json.into()))
}

/// `jsonArrayQuery(select, returning)`: `JSON_ARRAY(SELECT ..)`.
fn json_array_query(cx: &mut FunctionContext) -> NeonResult<Node> {
    let select = select_at(cx, 0)?;
    let mut json = Func::json_array_query(select);
    if let Some(column) = returning(cx, 1)? {
        json = json.returning(column);
    }
    Ok(Node::Expr(json.into()))
}

/// The optional FILTER predicate at `index`.
fn filter(
    cx: &mut FunctionContext,
    index: usize,
) -> NeonResult<Option<pgorm::pgorm_query::Condition>> {
    let value = arg(cx, index);
    if absent(cx, value) {
        Ok(None)
    } else {
        predicate(cx, value).map(Some)
    }
}

/// `jsonObjectagg(key, value, absentOnNull, uniqueKeys, returning, filter)`.
// [spec:pgorm:req:bindings.sql-json]
fn json_object_agg(cx: &mut FunctionContext) -> NeonResult<Node> {
    let key = operand_at(cx, 0)?;
    let value = input_at(cx, 1)?;
    let mut json = Func::json_objectagg(key, value);
    if flag(cx, 2)? {
        json = json.absent_on_null();
    }
    if flag(cx, 3)? {
        json = json.with_unique_keys();
    }
    if let Some(column) = returning(cx, 4)? {
        json = json.returning(column);
    }
    if let Some(condition) = filter(cx, 5)? {
        json = json.filter(condition);
    }
    Ok(Node::Expr(json.into()))
}

/// `jsonArrayagg(value, orderBy, nullOnNull, returning, filter)`. The ORDER BY
/// takes no NULLS placement, so an ordering asking for one is refused rather
/// than dropped.
// [spec:pgorm:req:bindings.sql-json]
fn json_array_agg(cx: &mut FunctionContext) -> NeonResult<Node> {
    let value = input_at(cx, 0)?;
    let mut json = Func::json_arrayagg(value);
    for ordering in orderings_at(cx, 1)? {
        if ordering.nulls.is_some() {
            return refuse(
                cx,
                "JSON_ARRAYAGG's ORDER BY takes no NULLS FIRST or NULLS LAST",
            );
        }
        json = json.order_by(ordering.expr, ordering.order);
    }
    if flag(cx, 2)? {
        json = json.null_on_null();
    }
    if let Some(column) = returning(cx, 3)? {
        json = json.returning(column);
    }
    if let Some(condition) = filter(cx, 4)? {
        json = json.filter(condition);
    }
    Ok(Node::Expr(json.into()))
}

/// `jsonParse(input, uniqueKeys)`: `JSON(input)`.
fn json_parse(cx: &mut FunctionContext) -> NeonResult<Node> {
    let input = input_at(cx, 0)?;
    let json = Func::json(input);
    Ok(Node::Expr(if flag(cx, 1)? {
        json.with_unique_keys().into()
    } else {
        json.into()
    }))
}

/// `jsonScalar(operand)`: `JSON_SCALAR(operand)`, the operand's type carried
/// so a number stays a number.
fn json_scalar(cx: &mut FunctionContext) -> NeonResult<Node> {
    let operand = operand_at(cx, 0)?;
    Ok(Node::Expr(Func::json_scalar(operand)))
}

/// `jsonSerialize(input, returning)`.
fn json_serialize(cx: &mut FunctionContext) -> NeonResult<Node> {
    let input = input_at(cx, 0)?;
    let mut json = Func::json_serialize(input);
    if let Some(column) = returning(cx, 1)? {
        json = json.returning(column);
    }
    Ok(Node::Expr(json.into()))
}
