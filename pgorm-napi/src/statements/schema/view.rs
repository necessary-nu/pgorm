//! Views, recursive views and materialized views over a `Select`, the
//! refresh and drops a materialized view takes, and `CREATE TABLE AS`. Each
//! body is the `Select` builder's statement, written by pgorm-query with its
//! values inlined: a definition is stored as SQL text and binds nothing.
//! The body's tie limit and WINDOW clause are checked by pgorm-query's
//! constructor, which runs the two checks an embedded `Select` is read
//! through, in the same order and with the same errors, so the body is read
//! unchecked and the constructor's refusal reaches JavaScript as the error an
//! embedded read would raise.

use neon::prelude::*;
use pgorm::pgorm_query::{DropBehavior, MaterializedView, Table, View, ViewCheckOption};

use super::{
    super::{
        Node,
        args::{arg, definition_at, refuse},
    },
    Ddl, Part,
    options::{BEHAVIOR, choice, columns, flag, names, object, pick, relation_at, relations_at},
};

pub(super) const EXPORTS: &[(&str, super::super::Build)] = &[
    ("ddlCreateView", create_view),
    ("ddlCreateRecursiveView", create_recursive_view),
    ("ddlViewOrReplace", view_or_replace),
    ("ddlViewColumns", view_columns),
    ("ddlViewCheckOption", view_check_option),
    ("ddlViewSecurityBarrier", view_security_barrier),
    ("ddlViewSecurityInvoker", view_security_invoker),
    ("ddlDropView", drop_view),
    ("ddlCreateMaterializedView", create_materialized_view),
    ("ddlRelationIfNotExists", relation_if_not_exists),
    ("ddlRelationWithNoData", relation_with_no_data),
    ("ddlRefreshMaterializedView", refresh_materialized_view),
    ("ddlDropMaterializedView", drop_materialized_view),
    ("ddlCreateTableAs", create_table_as),
];

/// The statement at argument 0, which must be one of the relation-over-a-query
/// statements: a view of either kind, a materialized view or `CREATE TABLE AS`.
fn relation_statement(cx: &mut FunctionContext) -> NeonResult<Ddl> {
    super::receiver(
        cx,
        "a view, materialized view or CREATE TABLE AS",
        |part| match part {
            Part::Statement(
                ddl @ (Ddl::CreateView(_)
                | Ddl::CreateRecursiveView(_)
                | Ddl::CreateMaterializedView(_)
                | Ddl::CreateTableAs(_)),
            ) => Ok(ddl),
            other => Err(other.describe()),
        },
    )
}

/// Apply `$method`, which a plain and a recursive view share, to either; any
/// other statement is refused with `$why`.
macro_rules! on_view {
    ($cx:expr, $ddl:expr, $why:literal, $method:ident($($arg:expr),*)) => {
        match $ddl {
            Ddl::CreateView(mut view) => {
                view.$method($($arg),*);
                Ok(Ddl::CreateView(view).into())
            }
            Ddl::CreateRecursiveView(mut view) => {
                view.$method($($arg),*);
                Ok(Ddl::CreateRecursiveView(view).into())
            }
            _ => $cx.throw_type_error($why),
        }
    };
}

/// `ddlCreateView(name, select)`: a view is a relation, named as a table is.
// [spec:pgorm:req:bindings.schema-views+3]
fn create_view(cx: &mut FunctionContext) -> NeonResult<Node> {
    let name = relation_at(cx, 0)?;
    let body = definition_at(cx, 1)?;
    match View::create(name, body) {
        Ok(view) => Ok(Ddl::CreateView(Box::new(view)).into()),
        Err(error) => refuse(cx, error.to_string()),
    }
}

/// `ddlCreateRecursiveView(name, columns, select)`: the column list is what
/// the recursive query's rows are called, so it is never empty.
// [spec:pgorm:req:bindings.schema-views+3]
fn create_recursive_view(cx: &mut FunctionContext) -> NeonResult<Node> {
    let name = relation_at(cx, 0)?;
    let listed = arg(cx, 1);
    let (first, rest) = columns(cx, listed, "a recursive view's column list")?;
    let body = definition_at(cx, 2)?;
    let mut view = match View::create_recursive(name, first, body) {
        Ok(view) => view,
        Err(error) => return refuse(cx, error.to_string()),
    };
    view.columns(rest);
    Ok(Ddl::CreateRecursiveView(Box::new(view)).into())
}

fn view_or_replace(cx: &mut FunctionContext) -> NeonResult<Node> {
    let ddl = relation_statement(cx)?;
    on_view!(
        cx,
        ddl,
        "OR REPLACE belongs to a view: PostgreSQL 19 has none for a materialized view or a table",
        or_replace()
    )
}

/// `ddlViewColumns(statement, names)`: the names appended to those the
/// statement has, renaming the query's leading columns.
// [spec:pgorm:req:bindings.schema-views+3]
fn view_columns(cx: &mut FunctionContext) -> NeonResult<Node> {
    let ddl = relation_statement(cx)?;
    let listed = arg(cx, 1);
    let added = names(cx, listed, "columns")?;
    Ok(match ddl {
        Ddl::CreateView(mut view) => {
            view.columns(added);
            Ddl::CreateView(view)
        }
        Ddl::CreateRecursiveView(mut view) => {
            view.columns(added);
            Ddl::CreateRecursiveView(view)
        }
        Ddl::CreateMaterializedView(mut create) => {
            create.columns(added);
            Ddl::CreateMaterializedView(create)
        }
        Ddl::CreateTableAs(mut create) => {
            create.columns(added);
            Ddl::CreateTableAs(create)
        }
        other => other,
    }
    .into())
}

/// `ddlViewCheckOption(view, "local" | "cascaded")`. A recursive view has
/// none to set: the grammar refuses one there.
// [spec:pgorm:req:bindings.schema-views+3]
fn view_check_option(cx: &mut FunctionContext) -> NeonResult<Node> {
    let ddl = relation_statement(cx)?;
    let option = arg(cx, 1);
    let option = pick(
        cx,
        option,
        "a check option",
        &[
            ("local", ViewCheckOption::Local),
            ("cascaded", ViewCheckOption::Cascaded),
        ],
    )?;
    match ddl {
        Ddl::CreateView(mut view) => {
            view.check_option(option);
            Ok(Ddl::CreateView(view).into())
        }
        Ddl::CreateRecursiveView(_) => cx.throw_type_error(
            "a recursive view takes no check option: PostgreSQL refuses WITH CHECK OPTION there",
        ),
        _ => cx.throw_type_error("a check option belongs to a view"),
    }
}

fn view_security_barrier(cx: &mut FunctionContext) -> NeonResult<Node> {
    let ddl = relation_statement(cx)?;
    on_view!(
        cx,
        ddl,
        "security_barrier belongs to a view",
        security_barrier()
    )
}

fn view_security_invoker(cx: &mut FunctionContext) -> NeonResult<Node> {
    let ddl = relation_statement(cx)?;
    on_view!(
        cx,
        ddl,
        "security_invoker belongs to a view",
        security_invoker()
    )
}

/// A drop's `{ ifExists, behavior }`, the object at argument 1.
fn drop_options(cx: &mut FunctionContext) -> NeonResult<(bool, Option<DropBehavior>)> {
    let options = object(cx, 1, &["ifExists", "behavior"])?;
    Ok((
        flag(cx, options, "ifExists")?,
        choice(cx, options, "behavior", BEHAVIOR)?,
    ))
}

/// `ddlDropView(names, { ifExists, behavior })`.
fn drop_view(cx: &mut FunctionContext) -> NeonResult<Node> {
    let (first, rest) = relations_at(cx, 0)?;
    let (if_exists, behavior) = drop_options(cx)?;
    let mut drop = View::drop(first);
    for name in rest {
        drop.name(name);
    }
    if if_exists {
        drop.if_exists();
    }
    match behavior {
        Some(DropBehavior::Cascade) => drop.cascade(),
        Some(DropBehavior::Restrict) => drop.restrict(),
        None => &mut drop,
    };
    Ok(Ddl::DropView(drop).into())
}

/// `ddlCreateMaterializedView(name, select)`.
// [spec:pgorm:req:bindings.schema-views+3]
fn create_materialized_view(cx: &mut FunctionContext) -> NeonResult<Node> {
    let name = relation_at(cx, 0)?;
    let body = definition_at(cx, 1)?;
    match MaterializedView::create(name, body) {
        Ok(create) => Ok(Ddl::CreateMaterializedView(Box::new(create)).into()),
        Err(error) => refuse(cx, error.to_string()),
    }
}

fn relation_if_not_exists(cx: &mut FunctionContext) -> NeonResult<Node> {
    match relation_statement(cx)? {
        Ddl::CreateMaterializedView(mut create) => {
            create.if_not_exists();
            Ok(Ddl::CreateMaterializedView(create).into())
        }
        Ddl::CreateTableAs(mut create) => {
            create.if_not_exists();
            Ok(Ddl::CreateTableAs(create).into())
        }
        _ => cx.throw_type_error("IF NOT EXISTS belongs to a materialized view or CREATE TABLE AS: PostgreSQL has none for a view",
        ),
    }
}

fn relation_with_no_data(cx: &mut FunctionContext) -> NeonResult<Node> {
    match relation_statement(cx)? {
        Ddl::CreateMaterializedView(mut create) => {
            create.with_no_data();
            Ok(Ddl::CreateMaterializedView(create).into())
        }
        Ddl::CreateTableAs(mut create) => {
            create.with_no_data();
            Ok(Ddl::CreateTableAs(create).into())
        }
        _ => cx.throw_type_error("WITH NO DATA belongs to a materialized view or CREATE TABLE AS"),
    }
}

/// `ddlRefreshMaterializedView(name, { concurrently, withNoData })`. The two
/// are one choice in pgorm-query, because PostgreSQL refuses them together,
/// so asking for both is refused here rather than one silently dropped.
// [spec:pgorm:req:bindings.schema-views+3]
fn refresh_materialized_view(cx: &mut FunctionContext) -> NeonResult<Node> {
    let name = relation_at(cx, 0)?;
    let options = object(cx, 1, &["concurrently", "withNoData"])?;
    let concurrently = flag(cx, options, "concurrently")?;
    let no_data = flag(cx, options, "withNoData")?;
    let mut refresh = MaterializedView::refresh(name);
    match (concurrently, no_data) {
        (true, true) => {
            return refuse(
                cx,
                "a refresh is concurrent or WITH NO DATA, never both: PostgreSQL refuses the pair",
            );
        }
        (true, false) => {
            refresh.concurrently();
        }
        (false, true) => {
            refresh.with_no_data();
        }
        (false, false) => {}
    }
    Ok(Ddl::RefreshMaterializedView(refresh).into())
}

/// `ddlDropMaterializedView(names, { ifExists, behavior })`.
fn drop_materialized_view(cx: &mut FunctionContext) -> NeonResult<Node> {
    let (first, rest) = relations_at(cx, 0)?;
    let (if_exists, behavior) = drop_options(cx)?;
    let mut drop = MaterializedView::drop(first);
    for name in rest {
        drop.name(name);
    }
    if if_exists {
        drop.if_exists();
    }
    match behavior {
        Some(DropBehavior::Cascade) => drop.cascade(),
        Some(DropBehavior::Restrict) => drop.restrict(),
        None => &mut drop,
    };
    Ok(Ddl::DropMaterializedView(drop).into())
}

/// `ddlCreateTableAs(name, select)`: a table of the query's columns and rows.
// [spec:pgorm:req:bindings.schema-views+3]
fn create_table_as(cx: &mut FunctionContext) -> NeonResult<Node> {
    let name = relation_at(cx, 0)?;
    let body = definition_at(cx, 1)?;
    match Table::create_as(name, body) {
        Ok(create) => Ok(Ddl::CreateTableAs(Box::new(create)).into()),
        Err(error) => refuse(cx, error.to_string()),
    }
}
