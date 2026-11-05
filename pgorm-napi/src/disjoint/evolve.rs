//! A disjoint model's or registration's kind changes, as pgorm's
//! [`DisjointShape`] writes them, each step's statements as JavaScript's DDL
//! classes take them.

use neon::prelude::*;
use pgorm::pgorm_query::TableName;
use pgorm::{DdlStatement, DisjointChange, DisjointShape};

use super::DisjointHandle;
use crate::{
    errors::{Failure, throw},
    statements::{Ddl, Node, relation},
};

/// `statements` as `[kind, statement]` pairs, the kind naming the class
/// `lib/disjoint.js` gives the statement: `createType`, `createTable` and
/// `alterTable` for the builders that take more, `ddl` for the rest.
// [spec:pgorm:req:bindings.disjoint+1]
// [spec:pgorm:req:bindings.disjoint-evolve]
pub(super) fn ddl_pairs<'cx>(
    cx: &mut FunctionContext<'cx>,
    statements: Vec<DdlStatement>,
) -> JsResult<'cx, JsArray> {
    let array = JsArray::new(cx, statements.len());
    for (index, statement) in statements.into_iter().enumerate() {
        let (kind, ddl) = match statement {
            DdlStatement::CreateType(statement) => ("createType", Ddl::CreateType(statement)),
            DdlStatement::CreateTable(statement) => ("createTable", Ddl::CreateTable(statement)),
            DdlStatement::AlterTable(statement) => (
                "alterTable",
                Ddl::AlterTable(statement.get_table_name().clone(), statement),
            ),
            DdlStatement::AlterType(statement) => ("ddl", Ddl::AlterType(statement)),
            DdlStatement::DropType(statement) => ("ddl", Ddl::DropType(statement)),
            DdlStatement::DropTable(statement) => ("ddl", Ddl::DropTable(statement)),
            _ => {
                return throw(
                    cx,
                    Failure::Internal(
                        "a schema statement of a kind this build does not know".to_owned(),
                    ),
                );
            }
        };
        let pair = JsArray::new(cx, 2);
        let kind = cx.string(kind);
        pair.set(cx, 0, kind)?;
        let ddl = cx.boxed(Node::from(ddl));
        pair.set(cx, 1, ddl)?;
        array.set(cx, u32::try_from(index).unwrap_or(u32::MAX), pair)?;
    }
    Ok(array)
}

/// The change `change` names, of the kind `value`, which `table` names the
/// table of where the kind loses it.
// [spec:pgorm:req:bindings.disjoint-evolve]
fn written(
    shape: &DisjointShape,
    change: &str,
    value: &str,
    table: Option<TableName>,
) -> Result<DisjointChange, Failure> {
    let changed = match (change, table) {
        ("addKind", None) => shape.add_kind(value),
        ("addKindRecreating", None) => shape.add_kind_recreating(value),
        ("removeKind", None) => shape.remove_kind(value),
        ("addKindTable", None) => shape.add_kind_table(value),
        ("removeKindTable", Some(table)) => shape.remove_kind_table(value, table),
        _ => return Err(Failure::Internal("unknown disjoint kind change".to_owned())),
    };
    changed.map_err(|error| Failure::Construction(error.to_string()))
}

/// `disjointChange(disjoint, change, value, table)`: the steps of the kind
/// change `change` names, each `[statements, gap]`, the gap null or `[kind,
/// schema, table]`, `table` given only where the kind loses its table.
// [spec:pgorm:req:bindings.disjoint-evolve]
pub(super) fn change(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let change = cx.argument::<JsString>(1)?.value(&mut cx);
    let value = cx.argument::<JsString>(2)?.value(&mut cx);
    let table = cx.argument::<JsValue>(3)?;
    let table = match table.is_a::<JsNull, _>(&mut cx) {
        true => None,
        false => Some(relation(&mut cx, table)?),
    };
    let steps = match written(&core.shape, &change, &value, table) {
        Ok(changed) => changed.into_steps(),
        Err(failure) => return throw(&mut cx, failure),
    };
    let array = JsArray::new(&mut cx, steps.len());
    for (index, step) in steps.into_iter().enumerate() {
        let statements = ddl_pairs(&mut cx, step.statements().to_vec())?;
        let gap: Handle<JsValue> = match step.gap() {
            Some(gap) => {
                let described = JsArray::new(&mut cx, 3);
                let kind = cx.string(gap.kind());
                described.set(&mut cx, 0, kind)?;
                let schema: Handle<JsValue> = match gap.table().schema() {
                    Some(schema) => cx.string(schema.to_string()).upcast(),
                    None => cx.null().upcast(),
                };
                described.set(&mut cx, 1, schema)?;
                let table = cx.string(gap.table().table().to_string());
                described.set(&mut cx, 2, table)?;
                described.upcast()
            }
            None => cx.null().upcast(),
        };
        let pair = JsArray::new(&mut cx, 2);
        pair.set(&mut cx, 0, statements)?;
        pair.set(&mut cx, 1, gap)?;
        array.set(&mut cx, u32::try_from(index).unwrap_or(u32::MAX), pair)?;
    }
    Ok(array.upcast())
}
