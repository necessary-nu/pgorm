//! `ON CONFLICT`, over pgorm-query's typestate: an arbiter — an inference
//! target of columns and index expressions, or a named constraint — then an
//! action. The target is never empty, the update never assigns nothing, and
//! only a completed action reaches an INSERT.

use neon::prelude::*;
use pgorm::pgorm_query::{ConflictConstraint, ConflictTarget, ConflictUpdate, OnConflict};

use super::{
    Node,
    args::{list, name, name_at, node, operand_at, predicate_at, refuse, this},
};

pub(super) const EXPORTS: &[(&str, super::Build)] = &[
    ("conflictDoNothing", conflict_do_nothing),
    ("conflictOn", conflict_on),
    ("conflictOnConstraint", conflict_on_constraint),
    ("conflictWhere", conflict_where),
    ("conflictAction", conflict_action),
    ("conflictSet", conflict_set),
    ("conflictUpdate", conflict_update),
];

/// An arbiter: an inference target or a named constraint, awaiting its
/// action.
#[derive(Debug, Clone)]
pub(crate) enum Arbiter {
    Target(ConflictTarget),
    Constraint(ConflictConstraint),
}

/// `conflictDoNothing()`: `ON CONFLICT DO NOTHING`, answering any conflict.
// [spec:pgorm:req:bindings.writes+2]
fn conflict_do_nothing(_: &mut FunctionContext) -> NeonResult<Node> {
    Ok(Node::Conflict(OnConflict::do_nothing()))
}

/// `conflictOn(items)`: an inference target of columns, named by string, and
/// index expressions; at least one.
// [spec:pgorm:req:bindings.writes+2]
// [spec:pgorm:req:bindings.conflict-arbiters]
fn conflict_on(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut target: Option<ConflictTarget> = None;
    for item in list(cx, 0)? {
        target = Some(if item.is_a::<JsString, _>(cx) {
            let column = name(cx, item)?;
            match target {
                Some(target) => target.and_column(column),
                None => OnConflict::column(column),
            }
        } else {
            let expr = match node(cx, item) {
                Some(Node::Expr(expr)) => expr,
                _ => {
                    return cx.throw_type_error(
                        "a conflict target is a column name or an index expression",
                    );
                }
            };
            match target {
                Some(target) => target.and_expr(expr),
                None => OnConflict::expr(expr),
            }
        });
    }
    match target {
        Some(target) => Ok(Node::Arbiter(Arbiter::Target(target))),
        None => refuse(
            cx,
            "a conflict target names at least one column or expression",
        ),
    }
}

/// `conflictOnConstraint(name)`: `ON CONSTRAINT "name"`, which takes no
/// predicate.
// [spec:pgorm:req:bindings.conflict-arbiters]
fn conflict_on_constraint(cx: &mut FunctionContext) -> NeonResult<Node> {
    let constraint = name_at(cx, 0)?;
    Ok(Node::Arbiter(Arbiter::Constraint(OnConflict::constraint(
        constraint,
    ))))
}

/// `conflictWhere(node, predicate)`: an inference target's partial-index
/// predicate, or an update's condition; a constraint arbiter takes neither.
// [spec:pgorm:req:bindings.conflict-arbiters]
fn conflict_where(cx: &mut FunctionContext) -> NeonResult<Node> {
    let node = this(cx, 0)?;
    let predicate = predicate_at(cx, 1)?;
    let node = match node {
        Node::Arbiter(Arbiter::Target(target)) => {
            Node::Arbiter(Arbiter::Target(target.cond_where(predicate)))
        }
        Node::ConflictUpdate(update) => Node::ConflictUpdate(update.cond_where(predicate)),
        Node::Arbiter(Arbiter::Constraint(_)) => {
            return refuse(
                cx,
                "ON CONSTRAINT takes no predicate; an index's predicate goes with Conflict.on(..)",
            );
        }
        other => {
            return cx.throw_type_error(format!(
                "expected a conflict target or update, got {}",
                other.describe()
            ));
        }
    };
    Ok(node)
}

fn arbiter<'cx>(cx: &mut FunctionContext<'cx>) -> NeonResult<Arbiter> {
    match this(cx, 0)? {
        Node::Arbiter(arbiter) => Ok(arbiter),
        other => cx.throw_type_error(format!(
            "expected a conflict arbiter, got {}",
            other.describe()
        )),
    }
}

/// `conflictAction(arbiter)`: `DO NOTHING` for the conflicts the arbiter
/// matches.
fn conflict_action(cx: &mut FunctionContext) -> NeonResult<Node> {
    let conflict = match arbiter(cx)? {
        Arbiter::Target(target) => target.do_nothing(),
        Arbiter::Constraint(constraint) => constraint.do_nothing(),
    };
    Ok(Node::Conflict(conflict))
}

/// `conflictSet(node, column, value)`: an assignment of an expression, the
/// first making an arbiter an update.
fn conflict_set(cx: &mut FunctionContext) -> NeonResult<Node> {
    let node = this(cx, 0)?;
    let column = name_at(cx, 1)?;
    let value = operand_at(cx, 2)?;
    let update = match node {
        Node::Arbiter(Arbiter::Target(target)) => target.value(column, value),
        Node::Arbiter(Arbiter::Constraint(constraint)) => constraint.value(column, value),
        Node::ConflictUpdate(update) => update.value(column, value),
        other => {
            return cx.throw_type_error(format!(
                "expected a conflict arbiter or update, got {}",
                other.describe()
            ));
        }
    };
    Ok(Node::ConflictUpdate(update))
}

/// `conflictUpdate(node, columns)`: columns set from the row that failed to
/// insert, `EXCLUDED`'s; at least one.
fn conflict_update(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut node = this(cx, 0)?;
    let columns = list(cx, 1)?;
    if columns.is_empty() {
        return refuse(cx, "an update names at least one column");
    }
    for column in columns {
        let column = name(cx, column)?;
        let update: ConflictUpdate = match node {
            Node::Arbiter(Arbiter::Target(target)) => target.update_column(column),
            Node::Arbiter(Arbiter::Constraint(constraint)) => constraint.update_column(column),
            Node::ConflictUpdate(update) => update.update_column(column),
            other => {
                return cx.throw_type_error(format!(
                    "expected a conflict arbiter or update, got {}",
                    other.describe()
                ));
            }
        };
        node = Node::ConflictUpdate(update);
    }
    Ok(node)
}

/// The completed action `value` holds; an arbiter without its action is
/// refused.
pub(crate) fn clause<'cx>(
    cx: &mut FunctionContext<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<OnConflict> {
    match node(cx, value) {
        Some(Node::Conflict(conflict)) => Ok(conflict),
        Some(Node::ConflictUpdate(update)) => Ok(update.into()),
        Some(Node::Arbiter(_)) => cx.throw_type_error(
            "a conflict target needs its action: doNothing(), update(..) or set(..)",
        ),
        _ => cx.throw_type_error("onConflict takes a Conflict action"),
    }
}
