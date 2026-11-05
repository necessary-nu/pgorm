//! LIKE, ILIKE and SIMILAR TO patterns over pgorm-query's `LikeExpr`: the
//! pattern bound, and the `ESCAPE` the grammar admits only there written by
//! pgorm-query's literal renderer.

use neon::prelude::*;
use pgorm::pgorm_query::LikeExpr;

use super::{
    Node,
    args::{self, arg, refuse, this},
};
use crate::values::read;

pub(super) const EXPORTS: &[(&str, super::Build)] = &[
    ("likeExprNew", like_expr_new),
    ("likeExprEscape", like_expr_escape),
    ("likeExprNoEscape", like_expr_no_escape),
];

/// The pattern a match takes: a `LikeExpr`, or a string, which is the
/// pattern with the default escape, as pgorm-query's `IntoLikeExpr` reads
/// one.
// [spec:pgorm:req:bindings.expressions+3]
pub(super) fn pattern<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<LikeExpr> {
    match args::node(cx, value) {
        Some(Node::LikeExpr(pattern)) => Ok(pattern),
        Some(other) => {
            let what = other.describe();
            cx.throw_type_error(format!(
                "a LIKE or SIMILAR TO pattern is a string or a LikeExpr, not {what}"
            ))
        }
        None => Ok(LikeExpr::new(read::text(
            cx,
            value,
            "a LIKE or SIMILAR TO pattern",
        )?)),
    }
}

fn like_expr_at(cx: &mut FunctionContext) -> NeonResult<LikeExpr> {
    match this(cx, 0)? {
        Node::LikeExpr(pattern) => Ok(pattern),
        other => {
            let what = other.describe();
            cx.throw_type_error(format!("expected a LikeExpr, got {what}"))
        }
    }
}

/// `likeExprNew(pattern)`: `LikeExpr::new`, the default backslash escape.
// [spec:pgorm:req:bindings.expressions+3]
fn like_expr_new(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pattern = arg(cx, 0);
    let pattern = read::text(cx, pattern, "a LIKE or SIMILAR TO pattern")?;
    Ok(Node::LikeExpr(LikeExpr::new(pattern)))
}

/// `likeExprEscape(pattern, character)`: `LikeExpr::escape`, one character
/// and not NUL, which a literal cannot hold.
// [spec:pgorm:req:bindings.expressions+3]
fn like_expr_escape(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pattern = like_expr_at(cx)?;
    let escape = arg(cx, 1);
    let escape = read::text(cx, escape, "a pattern's escape")?;
    let mut chars = escape.chars();
    match (chars.next(), chars.next()) {
        (Some(character), None) if character != '\0' => {
            Ok(Node::LikeExpr(pattern.escape(character)))
        }
        _ => refuse(
            cx,
            "a pattern's escape is one character, not NUL; noEscape() turns escaping off",
        ),
    }
}

/// `likeExprNoEscape(pattern)`: `LikeExpr::no_escape`, `ESCAPE ''`.
// [spec:pgorm:req:bindings.expressions+3]
fn like_expr_no_escape(cx: &mut FunctionContext) -> NeonResult<Node> {
    let pattern = like_expr_at(cx)?;
    Ok(Node::LikeExpr(pattern.no_escape()))
}
