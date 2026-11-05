//! `SELECT`: projection, FROM items and joins, filters, grouping, ordering,
//! limits, set operations, locking and common table expressions, over
//! pgorm-query's `SelectStatement`.

use neon::prelude::*;
use pgorm::pgorm_query::{
    Asterisk, FromItem, JoinType, LockBehavior, LockType, OrderedStatement, Query, SelectStatement,
    UnionType,
};

use super::{
    Node,
    args::{arg, choice, count, list, node, orderings_at, predicate_at, refuse, select_at, this},
    grouping, table, with,
};

pub(super) const EXPORTS: &[(&str, super::Build)] = &[
    ("selectNew", select_new),
    ("selectProject", select_project),
    ("selectFrom", select_from),
    ("selectJoin", select_join),
    ("selectWhere", select_where),
    ("selectGroupBy", select_group_by),
    ("selectGroupByDistinct", select_group_by_distinct),
    ("selectHaving", select_having),
    ("selectOrderBy", select_order_by),
    ("selectLimit", select_limit),
    ("selectDistinct", select_distinct),
    ("selectSetOperation", select_set_operation),
    ("selectLock", select_lock),
    ("selectWith", select_with),
    ("selectWindow", select_window),
];

fn receiver<'cx>(cx: &mut FunctionContext<'cx>) -> NeonResult<SelectStatement> {
    match this(cx, 0)? {
        Node::Select(select) => Ok(select),
        other => cx.throw_type_error(format!("expected a Select, got {}", other.describe())),
    }
}

/// Add each item of the array at `index` to `select`'s projection: an
/// expression, or an aliased one.
fn project<'cx>(
    cx: &mut FunctionContext<'cx>,
    select: &mut SelectStatement,
    index: usize,
) -> NeonResult<()> {
    for item in list(cx, index)? {
        match node(cx, item) {
            Some(Node::Expr(expr)) => {
                select.expr(expr);
            }
            Some(Node::Aliased(aliased)) => {
                select.expr_as(aliased.expr, aliased.alias);
            }
            Some(Node::Windowed(windowed)) => windowed.project(select),
            Some(other) => {
                let what = other.describe();
                return cx.throw_type_error(format!("a projection is an expression, not {what}"));
            }
            None => {
                return cx.throw_type_error(
                    "a projection is an expression: bind a value with bind(value)",
                );
            }
        }
    }
    Ok(())
}

/// `selectNew(items)`: `SELECT items`, or `SELECT *` when there are none.
// [spec:pgorm:req:bindings.select+3]
fn select_new(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = Query::select();
    if list(cx, 0)?.is_empty() {
        select.column(Asterisk);
    } else {
        project(cx, &mut select, 0)?;
    }
    Ok(Node::Select(select))
}

/// `selectProject(select, items)`: the projection replaced by `items`, of
/// which there is at least one.
fn select_project(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    if list(cx, 1)?.is_empty() {
        return refuse(cx, "select() takes at least one item");
    }
    select.clear_selects();
    project(cx, &mut select, 1)?;
    Ok(Node::Select(select))
}

/// `selectFrom(select, item)`: one more FROM item, comma-joined to the ones
/// before it.
fn select_from(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    let item = arg(cx, 1);
    let item = table::from_item(cx, item)?;
    select.from(item);
    Ok(Node::Select(select))
}

/// `selectJoin(select, kind, item, on, lateral)`: a join, `on` absent for a
/// cross join. `lateral` applies to a subquery, which may then read the
/// columns of the items before it.
// [spec:pgorm:req:bindings.select+3]
// [spec:pgorm:req:bindings.from-subqueries]
fn select_join(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    let kind = choice(
        cx,
        1,
        "a join's kind",
        &["inner", "left", "right", "full", "cross"],
    )?;
    let item = arg(cx, 2);
    let item = table::from_item(cx, item)?;
    let lateral = super::args::flag(cx, 4)?;
    if kind == "cross" {
        if lateral {
            return refuse(cx, "a cross join takes no lateral subquery here");
        }
        select.cross_join(item);
        return Ok(Node::Select(select));
    }
    let on = predicate_at(cx, 3)?;
    let kind = match kind {
        "inner" => JoinType::InnerJoin,
        "left" => JoinType::LeftJoin,
        "right" => JoinType::RightJoin,
        _ => JoinType::FullOuterJoin,
    };
    match (lateral, item) {
        (false, item) => select.join(kind, item, on),
        (true, FromItem::SubQuery(query, alias)) => select.join_lateral(kind, query, alias, on),
        (true, _) => return refuse(cx, "lateral applies to a subquery"),
    };
    Ok(Node::Select(select))
}

/// `selectWhere(select, predicate)`: the predicate added to the WHERE
/// clause, joined to any already there with AND.
fn select_where(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    let predicate = predicate_at(cx, 1)?;
    select.cond_where(predicate);
    Ok(Node::Select(select))
}

/// `selectGroupBy(select, items)`: each item added to the GROUP BY list in
/// order, an expression or a grouping element, after any already there.
// [spec:pgorm:req:bindings.select+3]
fn select_group_by(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    for item in list(cx, 1)? {
        let element = grouping::element(cx, item)?;
        select.group_by_element(element);
    }
    Ok(Node::Select(select))
}

/// `selectGroupByDistinct(select)`: `GROUP BY DISTINCT`, each grouping set
/// the list reaches more than once grouped once. It quantifies the whole
/// list, whichever call adds to it, and renders nothing without one.
// [spec:pgorm:req:bindings.select+3]
fn select_group_by_distinct(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    select.group_by_distinct();
    Ok(Node::Select(select))
}

fn select_having(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    let predicate = predicate_at(cx, 1)?;
    select.cond_having(predicate);
    Ok(Node::Select(select))
}

/// Append orderings to a statement's or a window's ORDER BY.
pub(super) fn order<S: OrderedStatement>(statement: &mut S, orderings: Vec<super::Ordering>) {
    for ordering in orderings {
        match ordering.nulls {
            Some(nulls) => statement.order_by_expr_with_nulls(ordering.expr, ordering.order, nulls),
            None => statement.order_by_expr(ordering.expr, ordering.order),
        };
    }
}

fn select_order_by(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    let orderings = orderings_at(cx, 1)?;
    order(&mut select, orderings);
    Ok(Node::Select(select))
}

/// `selectLimit(select, "limit" | "withTies" | "offset", count)`: the clause
/// set to a count, or removed by `null`. A tie limit takes LIMIT's slot,
/// replacing a plain limit as one replaces it, and needs its count; whether
/// the statement can take one is checked as it is built, since its ORDER BY
/// may come later.
// [spec:pgorm:req:bindings.select+3]
fn select_limit(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    let clause = choice(cx, 1, "the clause", &["limit", "withTies", "offset"])?;
    let value = arg(cx, 2);
    let value = if clause != "withTies" && super::args::absent(cx, value) {
        None
    } else {
        Some(count(cx, 2)?)
    };
    match (clause, value) {
        ("limit", Some(value)) => select.limit(value),
        ("limit", None) => select.reset_limit(),
        ("withTies", Some(value)) => select.limit_with_ties(value),
        (_, Some(value)) => select.offset(value),
        (_, None) => select.reset_offset(),
    };
    Ok(Node::Select(select))
}

fn select_distinct(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    select.distinct();
    Ok(Node::Select(select))
}

/// `selectSetOperation(select, operation, other)`: `UNION`, `INTERSECT` or
/// `EXCEPT`, each with its `ALL` form, of this query's rows and `other`'s.
// [spec:pgorm:req:bindings.select+3]
fn select_set_operation(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    let operation = choice(
        cx,
        1,
        "a set operation",
        &[
            "union",
            "unionAll",
            "intersect",
            "intersectAll",
            "except",
            "exceptAll",
        ],
    )?;
    let other = select_at(cx, 2)?;
    let operation = match operation {
        "union" => UnionType::Distinct,
        "unionAll" => UnionType::All,
        "intersect" => UnionType::Intersect,
        "intersectAll" => UnionType::IntersectAll,
        "except" => UnionType::Except,
        _ => UnionType::ExceptAll,
    };
    select.union(operation, other);
    Ok(Node::Select(select))
}

/// `selectLock(select, strength, of, behavior)`: `FOR UPDATE` and the other
/// row-locking strengths, optionally `OF` some of the statement's items and
/// with `NOWAIT` or `SKIP LOCKED`.
// [spec:pgorm:req:bindings.row-locks]
fn select_lock(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    let strength = match choice(
        cx,
        1,
        "a lock strength",
        &["update", "noKeyUpdate", "share", "keyShare"],
    )? {
        "update" => LockType::Update,
        "noKeyUpdate" => LockType::NoKeyUpdate,
        "share" => LockType::Share,
        _ => LockType::KeyShare,
    };
    // `OF` names each item by its qualifier, the name it answers to in the
    // statement, which PostgreSQL requires unqualified.
    let mut tables = Vec::new();
    for item in list(cx, 2)? {
        tables.push(table::from_item(cx, item)?.qualifier().clone());
    }
    let behavior = arg(cx, 3);
    let behavior = if super::args::absent(cx, behavior) {
        None
    } else {
        Some(
            match choice(cx, 3, "a lock's wait", &["nowait", "skipLocked"])? {
                "nowait" => LockBehavior::Nowait,
                _ => LockBehavior::SkipLocked,
            },
        )
    };
    match behavior {
        Some(behavior) => select.lock_with_tables_behavior(strength, tables, behavior),
        None => select.lock_with_tables(strength, tables),
    };
    Ok(Node::Select(select))
}

/// `selectWith(select, clause)`: the statement's WITH clause, replacing any
/// it had.
fn select_with(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    let clause = with::clause_at(cx, 1)?;
    select.with(clause);
    Ok(Node::Select(select))
}

/// `selectWindow(select, name, window)`: an entry appended to the WINDOW
/// clause, `"name" AS (..)`, which `over(name)` reads. A clause PostgreSQL
/// refuses is refused where the tie limit is: when the statement is compiled
/// or read into another.
// [spec:pgorm:req:bindings.windows]
fn select_window(cx: &mut FunctionContext) -> NeonResult<Node> {
    let mut select = receiver(cx)?;
    let name = super::args::name_at(cx, 1)?;
    let window = super::window::definition_at(cx, 2)?;
    select.window(name, window);
    Ok(Node::Select(select))
}
