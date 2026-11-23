//! A table's column, pgorm-query's `ColumnDef`: its name, type and collation,
//! and the clauses written after them in the order they were added — `NULL`,
//! the one `NOT NULL` constraint, `DEFAULT`, `CHECK`, a generated expression
//! of a stated kind, and an identity with its sequence's options.

use neon::prelude::*;
use pgorm::pgorm_query::{Check, ColumnDef, GeneratedKind, IdentityGeneration};

use super::{
    super::{
        Node,
        args::{absent, arg, expression, name_at, operand_at},
        data_type::column_type,
    },
    Part,
    options::{self, ENFORCEMENT, collation, flag, name_option, object, pick},
};

pub(super) const EXPORTS: &[(&str, super::super::Build)] = &[
    ("ddlColumnNew", column_new),
    ("ddlColumnNotNull", column_not_null),
    ("ddlColumnNull", column_null),
    ("ddlColumnDefault", column_default),
    ("ddlColumnCheck", column_check),
    ("ddlColumnGenerated", column_generated),
    ("ddlColumnIdentity", column_identity),
    ("ddlColumnAutoIncrement", column_auto_increment),
    ("ddlColumnCollate", column_collate),
];

pub(super) fn column<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<ColumnDef> {
    match super::super::args::node(cx, value) {
        Some(Node::Schema(part)) => match *part {
            Part::Column(column) => Ok(column),
            other => cx.throw_type_error(format!("expected a ColumnDef, got {}", other.describe())),
        },
        Some(other) => {
            cx.throw_type_error(format!("expected a ColumnDef, got {}", other.describe()))
        }
        None => cx.throw_type_error("expected a ColumnDef"),
    }
}

/// The receiver, a `ColumnDef`, and a method's change to a copy of it.
fn with(
    cx: &mut FunctionContext,
    change: impl FnOnce(&mut FunctionContext, &mut ColumnDef) -> NeonResult<()>,
) -> NeonResult<Node> {
    let receiver = arg(cx, 0);
    let mut column = column(cx, receiver)?;
    change(cx, &mut column)?;
    Ok(Part::Column(column).into())
}

/// `ddlColumnNew(name, type)`: a column, its type left out for a
/// `modifyColumn` that does not retype it.
// [spec:pgorm:req:bindings.schema-tables]
fn column_new(cx: &mut FunctionContext) -> NeonResult<Node> {
    let name = name_at(cx, 0)?;
    let kind = arg(cx, 1);
    let column = if absent(cx, kind) {
        ColumnDef::new(name)
    } else {
        ColumnDef::new_with_type(name, column_type(cx, kind)?)
    };
    Ok(Part::Column(column).into())
}

/// `ddlColumnNotNull(column, { name, noInherit })`: the column's one `NOT
/// NULL` constraint, named and kept from inheriting tables as asked; a later
/// call changes it where it stands.
// [spec:pgorm:req:bindings.schema-tables]
fn column_not_null(cx: &mut FunctionContext) -> NeonResult<Node> {
    with(cx, |cx, column| {
        let options = object(cx, 1, &["name", "noInherit"])?;
        let name = name_option(cx, options, "name")?;
        let no_inherit = flag(cx, options, "noInherit")?;
        column.not_null();
        if let Some(name) = name {
            column.not_null_named(name);
        }
        if no_inherit {
            column.not_null_no_inherit();
        }
        Ok(())
    })
}

fn column_null(cx: &mut FunctionContext) -> NeonResult<Node> {
    with(cx, |_, column| {
        column.null();
        Ok(())
    })
}

/// `ddlColumnDefault(column, value)`: an expression, or a value written as
/// pgorm-query's escaped literal.
fn column_default(cx: &mut FunctionContext) -> NeonResult<Node> {
    with(cx, |cx, column| {
        let value = operand_at(cx, 1)?;
        column.default(value);
        Ok(())
    })
}

/// A `CHECK` over the expression at `index`, named, enforced or not and kept
/// from inheriting tables as the options object after it says: what a
/// column's, a table's and an added `CHECK` each are.
// [spec:pgorm:req:bindings.schema-tables]
pub(super) fn check<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
    extra: &[&str],
) -> NeonResult<(Check, Option<Handle<'cx, JsObject>>)> {
    let condition = arg(cx, index);
    let condition = expression(cx, condition)?;
    let mut known = vec!["name", "enforcement", "noInherit"];
    known.extend_from_slice(extra);
    let options = object(cx, index + 1, &known)?;
    let mut check = Check::new(condition);
    if let Some(name) = name_option(cx, options, "name")? {
        check = check.name(name);
    }
    if let Some(enforcement) = options::choice(cx, options, "enforcement", ENFORCEMENT)? {
        check = check.enforcement(enforcement);
    }
    if flag(cx, options, "noInherit")? {
        check = check.no_inherit();
    }
    Ok((check, options))
}

fn column_check(cx: &mut FunctionContext) -> NeonResult<Node> {
    let (check, _) = check(cx, 1, &[])?;
    with(cx, |_, column| {
        column.check(check);
        Ok(())
    })
}

/// `ddlColumnGenerated(column, expression, kind)`: the kind is always named,
/// PostgreSQL 19 reading a generated column without one as virtual, where 17
/// refused it.
// [spec:pgorm:req:bindings.schema-tables]
fn column_generated(cx: &mut FunctionContext) -> NeonResult<Node> {
    with(cx, |cx, column| {
        let expr = arg(cx, 1);
        let expr = expression(cx, expr)?;
        let kind = arg(cx, 2);
        let kind = pick(
            cx,
            kind,
            "a generated column's kind",
            &[
                ("stored", GeneratedKind::Stored),
                ("virtual", GeneratedKind::Virtual),
            ],
        )?;
        column.generated(expr, kind);
        Ok(())
    })
}

/// `ddlColumnIdentity(column, generation, options)`: `GENERATED { ALWAYS |
/// BY DEFAULT } AS IDENTITY`, with its sequence's options when there are any.
// [spec:pgorm:req:bindings.schema-tables]
fn column_identity(cx: &mut FunctionContext) -> NeonResult<Node> {
    with(cx, |cx, column| {
        let generation = arg(cx, 1);
        let generation = pick(
            cx,
            generation,
            "an identity's generation",
            &[
                ("always", IdentityGeneration::Always),
                ("byDefault", IdentityGeneration::ByDefault),
            ],
        )?;
        match options::sequence_options(cx, 2)? {
            Some(sequence) => column.identity_with(generation, sequence),
            None => match generation {
                IdentityGeneration::Always => column.identity(),
                IdentityGeneration::ByDefault => column.identity_by_default(),
            },
        };
        Ok(())
    })
}

fn column_auto_increment(cx: &mut FunctionContext) -> NeonResult<Node> {
    with(cx, |_, column| {
        column.auto_increment();
        Ok(())
    })
}

/// `ddlColumnCollate(column, collation)`: the column's one collation, a later
/// call replacing it.
fn column_collate(cx: &mut FunctionContext) -> NeonResult<Node> {
    with(cx, |cx, column| {
        let named = arg(cx, 1);
        let named = collation(cx, named)?;
        column.collate(named);
        Ok(())
    })
}
