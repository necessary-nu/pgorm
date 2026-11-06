//! What SQL's set-operation precedence costs a flat rendering.
//!
//! A pipeline writes set operations left to right, so
//! `append(b).intersect(c)` means `(a ∪ b) ∩ c`. Rendered flat — one
//! `SELECT` per operand with the operators between them, which is what prqlc
//! emits when nothing forces a subquery — that reaches PostgreSQL as
//! `a UNION ALL b INTERSECT ALL c`, and PostgreSQL binds `INTERSECT` tighter
//! than `UNION` and `EXCEPT`. The server evaluates `a ∪ (b ∩ c)`: a different
//! relation, and for a self-combining source every row twice.
//!
//! Parentheses are not available — prqlc owns the rendering — but a binding
//! is: a `let`-bound subrelation is a CTE the next operation reads as one
//! whole relation, which is the same bracket by another spelling. This module
//! is the knowledge of *when* that bracket is owed, and nothing else.

use super::adapter::{self, PlExpr};

/// How tightly PostgreSQL binds a pipeline set operation's SQL operator.
///
/// `UNION` and `EXCEPT` share one level and associate left, so a chain of
/// those two alone already evaluates the way it was written; `INTERSECT` sits
/// above both. Anything that is not a set operation has no binding power and
/// cannot reassociate a chain.
// [spec:pgorm:req:pipeline.compose+1]
fn binding_power(verb: &str) -> Option<u8> {
    match verb {
        "append" | "remove" => Some(1),
        "intersect" => Some(2),
        _ => None,
    }
}

/// Whether appending `op` to these stages would render a chain PostgreSQL
/// reassociates — the question [`Pipeline::set_op`](super::Pipeline) settles
/// a binding to answer `false` to.
///
/// The whole pending run is read rather than the stage that happens to be
/// last, because a stage between two set operations does not reliably break
/// the chain: prqlc folds a projection the relation already carries straight
/// back into the set operation's own arms, leaving the operators as adjacent
/// as they were. Reading the run costs a settle in the cases where prqlc
/// would have wrapped anyway — one binding, the same rows.
// [spec:pgorm:req:pipeline.compose+1]
pub(super) fn reassociates(stages: &[PlExpr], op: &str) -> bool {
    let Some(power) = binding_power(op) else {
        return false;
    };
    stages
        .iter()
        .filter_map(adapter::stage_verb)
        .filter_map(binding_power)
        .any(|pending| pending < power)
}
