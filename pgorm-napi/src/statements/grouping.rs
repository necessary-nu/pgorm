//! Grouping elements: the items of a `GROUP BY` list beyond a bare
//! expression — a parenthesised set, `()`, `ROLLUP`, `CUBE` and
//! `GROUPING SETS` — over pgorm-query's `GroupingElement`.

use neon::prelude::*;
use pgorm::pgorm_query::GroupingElement;

use super::{
    Node,
    args::{arg, choice, expressions_at, list, node},
};

pub(super) const EXPORTS: &[(&str, super::Build)] = &[
    ("groupingNew", grouping_new),
    ("groupingSets", grouping_sets),
];

/// A `GROUP BY` item: a grouping element, or an expression, which is the set
/// of that one expression and renders bare, as `add_group_by` writes it.
// [spec:pgorm:req:bindings.select+3]
pub(super) fn element<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<GroupingElement> {
    match node(cx, value) {
        Some(Node::GroupingElement(element)) => Ok(element),
        Some(Node::Expr(expr)) => Ok(GroupingElement::set([expr])),
        Some(other) => {
            let what = other.describe();
            cx.throw_type_error(format!(
                "a grouping item is an expression or a grouping element, not {what}"
            ))
        }
        None => cx.throw_type_error(
            "a grouping item is an expression or a grouping element, never a bound value",
        ),
    }
}

/// `groupingNew("set" | "empty" | "rollup" | "cube", expressions)`: one set,
/// `("a", "b")`, the empty set `()`, or `ROLLUP` or `CUBE` over the
/// expressions, each of which may be a `tuple` standing as one unit — each
/// through pgorm-query's `GroupingElement` constructor of the same name.
// [spec:pgorm:req:bindings.select+3]
fn grouping_new(cx: &mut FunctionContext) -> NeonResult<Node> {
    let kind = choice(
        cx,
        0,
        "a grouping element",
        &["set", "empty", "rollup", "cube"],
    )?;
    let expressions = expressions_at(cx, 1)?;
    let element = match kind {
        "set" => GroupingElement::set(expressions),
        "empty" => GroupingElement::empty(),
        "rollup" => GroupingElement::rollup(expressions),
        _ => GroupingElement::cube(expressions),
    };
    Ok(Node::GroupingElement(element))
}

/// `groupingSets(first, rest)`: `GROUPING SETS (..)` over its first item and
/// the array of the others, each a grouping element or an expression, as
/// pgorm-query's builder takes its first element before any other.
// [spec:pgorm:req:bindings.select+3]
fn grouping_sets(cx: &mut FunctionContext) -> NeonResult<Node> {
    let first = arg(cx, 0);
    let first = element(cx, first)?;
    let mut sets = GroupingElement::sets(first);
    for item in list(cx, 1)? {
        sets = sets.add(element(cx, item)?);
    }
    Ok(Node::GroupingElement(sets.into()))
}
