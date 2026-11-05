//! A write's RETURNING list, and the two versions of a written row it reads:
//! as it was before the write and as the statement left it.

use neon::prelude::*;
use pgorm::pgorm_query::{
    Asterisk, BinOper, Expr, ExprTrait, Query, ReturningClause, ReturningRow,
};

use super::{
    Node,
    args::{arg, choice, list, name_at, node, optional_name},
};

pub(super) const EXPORTS: &[(&str, super::Build)] = &[
    ("returningCol", returning_col),
    ("returningStar", returning_star),
];

fn version(cx: &mut FunctionContext) -> NeonResult<ReturningRow> {
    Ok(match choice(cx, 0, "a row version", &["old", "new"])? {
        "old" => ReturningRow::Old,
        _ => ReturningRow::New,
    })
}

/// `returningCol(version, name)`: one column of the row's old or new version.
// [spec:pgorm:req:bindings.writes+2]
fn returning_col(cx: &mut FunctionContext) -> NeonResult<Node> {
    let version = version(cx)?;
    let column = name_at(cx, 1)?;
    Ok(Node::Expr(Expr::col((version, column)).into()))
}

/// `returningStar(version)`: every column of the row's old or new version.
fn returning_star(cx: &mut FunctionContext) -> NeonResult<Node> {
    let version = version(cx)?;
    Ok(Node::Expr(Expr::col((version, Asterisk)).into()))
}

/// The RETURNING list of the items at `index` — every column when there are
/// none — with the row's versions renamed by the names at `index + 1` and
/// `index + 2`. An aliased item is written `expr AS "name"`, as pgorm-python
/// writes one.
// [spec:pgorm:req:bindings.writes+2]
pub(super) fn clause(cx: &mut FunctionContext, index: usize) -> NeonResult<ReturningClause> {
    let items = list(cx, index)?;
    let mut clause = if items.is_empty() {
        Query::returning().all()
    } else {
        let mut exprs = Vec::with_capacity(items.len());
        for item in items {
            exprs.push(match node(cx, item) {
                Some(Node::Expr(expr)) => expr,
                Some(Node::Aliased(aliased)) => {
                    aliased.expr.binary(BinOper::As, Expr::col(aliased.alias))
                }
                _ => {
                    return cx
                        .throw_type_error("a RETURNING item is an expression or an aliased one");
                }
            });
        }
        Query::returning().exprs(exprs)
    };
    let old = arg(cx, index + 1);
    if let Some(old) = optional_name(cx, old)? {
        clause = clause.old_as(old);
    }
    let new = arg(cx, index + 2);
    if let Some(new) = optional_name(cx, new)? {
        clause = clause.new_as(new);
    }
    Ok(clause)
}
