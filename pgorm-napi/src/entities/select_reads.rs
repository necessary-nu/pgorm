//! A registered `Select<E>`'s pages, keyset cursor, join, projection and
//! stream, through pgorm's own `PaginatorTrait`, `Select::cursor_by`,
//! `QuerySelect::join`, `select_only` and `Select::stream`'s decode.

use neon::prelude::*;
use pgorm_bindings::entity::Change;

use super::{
    EntityHandle, SelectHandle, read_terminal,
    reads::{CursorHandle, DecoderHandle, PagesHandle, page_size},
};
use crate::{errors::throw, statements::Node, values::read};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("entityRelation", entity_relation)?;
    cx.export_function("entitySelectJoin", select_join)?;
    cx.export_function("entitySelectProjected", select_projected)?;
    cx.export_function("entitySelectPages", select_pages)?;
    cx.export_function("entitySelectCursor", select_cursor)?;
    cx.export_function("entitySelectStream", select_stream)?;
    Ok(())
}

/// The column names in the array at argument `index`.
pub(super) fn columns_at(cx: &mut FunctionContext, index: usize) -> NeonResult<Vec<String>> {
    let list = cx.argument::<JsArray>(index)?.to_vec(cx)?;
    let mut names = Vec::with_capacity(list.len());
    for item in list {
        names.push(item.downcast_or_throw::<JsString, _>(cx)?.value(cx));
    }
    Ok(names)
}

/// `entityRelation(entity, name)`: refuses a name the entity's `Relation`
/// enum does not declare.
fn entity_relation(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = held!(cx, EntityHandle, 0);
    let name = text!(cx, 1);
    if let Err(failure) = entity.relation(&name) {
        return throw(&mut cx, failure);
    }
    Ok(cx.undefined().upcast())
}

/// `entitySelectJoin(select, entity, relation, kind)`: `QuerySelect::join`
/// of the relation `entity` declares as `relation`.
// [spec:pgorm:req:bindings.entity-reads+4]
fn select_join(mut cx: FunctionContext) -> JsResult<JsValue> {
    let select = held!(cx, SelectHandle, 0);
    let entity = held!(cx, EntityHandle, 1);
    let relation = text!(cx, 2);
    let kind = match text!(cx, 3).as_str() {
        "inner" => pgorm::pgorm_query::JoinType::InnerJoin,
        "left" => pgorm::pgorm_query::JoinType::LeftJoin,
        "right" => pgorm::pgorm_query::JoinType::RightJoin,
        "full" => pgorm::pgorm_query::JoinType::FullOuterJoin,
        _ => return cx.throw_type_error("a join's kind is inner, left, right or full"),
    };
    if let Err(failure) = entity.relation(&relation) {
        return throw(&mut cx, failure);
    }
    let joined = select.change(Change::Join(kind, entity, relation));
    Ok(cx.boxed(SelectHandle(joined)).upcast())
}

/// `entitySelectProjected(select, columns, terminal)`: the query's
/// `select_only` and `columns`, a `Select` statement whose rows are records
/// of those columns, `one` and `optional` narrowed to one row.
// [spec:pgorm:req:bindings.entity-reads+4]
fn select_projected(mut cx: FunctionContext) -> JsResult<JsValue> {
    let select = held!(cx, SelectHandle, 0);
    let columns = columns_at(&mut cx, 1)?;
    if columns.is_empty() {
        return read::refuse(&mut cx, "a selection names at least one column");
    }
    let terminal = read_terminal(&mut cx, 2)?;
    match select.projected(&columns, terminal) {
        Ok(statement) => Ok(cx.boxed(Node::Select(statement)).upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// `entitySelectPages(select, size)`: the query's pages of `size` rows.
fn select_pages(mut cx: FunctionContext) -> JsResult<JsValue> {
    let select = held!(cx, SelectHandle, 0);
    let size = page_size(&mut cx, 1)?;
    Ok(cx.boxed(PagesHandle(select.pages(), size)).upcast())
}

fn select_cursor(mut cx: FunctionContext) -> JsResult<JsValue> {
    let select = held!(cx, SelectHandle, 0);
    let columns = columns_at(&mut cx, 1)?;
    match select.cursor(&columns) {
        Ok(keyset) => Ok(cx
            .boxed(CursorHandle(keyset, pgorm::CursorPlan::default()))
            .upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// `entitySelectStream(select)`: `[statement, decoder]`, the statement a
/// stream sends and the decode `Select::stream` runs on each row.
// [spec:pgorm:req:bindings.entity-reads+4]
fn select_stream(mut cx: FunctionContext) -> JsResult<JsValue> {
    let select = held!(cx, SelectHandle, 0);
    let statement = cx.boxed(Node::Select(select.statement()));
    let decoder = cx.boxed(DecoderHandle {
        decode: select.decode(),
        sources: vec![select.info().clone()],
    });
    let pair = JsArray::new(&mut cx, 2);
    pair.set(&mut cx, 0, statement)?;
    pair.set(&mut cx, 1, decoder)?;
    Ok(pair.upcast())
}
