//! A registered graph's pages, keyset cursors, grouped read and stream,
//! through pgorm's own `PaginatorTrait`, `SelectGraph::cursor_by` and
//! `cursor_by_on`, `all_grouped` and `SelectGraph::stream`'s decode.

use std::sync::Arc;

use neon::prelude::*;

use super::{
    convert::{entity_failure, list, names, record},
    graph::Groups,
    info::EntityInfo,
    reads::{CursorHandle, DecoderHandle, PagesHandle, page_size},
    select_reads::columns_at,
};
use crate::{
    connect::job::{Job, JobHandle, Output, Settled},
    statements::Node,
    values::read,
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("graphQueryCursor", query_cursor)?;
    cx.export_function("graphQueryPages", query_pages)?;
    cx.export_function("graphQueryStream", query_stream)?;
    cx.export_function("graphQueryGroupedJob", query_grouped_job)?;
    Ok(())
}

fn query(cx: &mut FunctionContext) -> NeonResult<super::graph::Query> {
    Ok(cx
        .argument::<JsBox<super::graphs::GraphQueryHandle>>(0)?
        .0
        .clone())
}

/// `graphQueryCursor(query, source, columns)`: a keyset cursor ordered by
/// root columns, `cursor_by`, for source 0, and otherwise by columns of slot
/// `source`, `cursor_by_on`.
// [spec:pgorm:req:bindings.entity-graphs+4]
fn query_cursor(mut cx: FunctionContext) -> JsResult<JsValue> {
    let query = query(&mut cx)?;
    let source = super::reads::whole(&mut cx, 1, "a source")?;
    let columns = columns_at(&mut cx, 2)?;
    let Ok(source) = usize::try_from(source) else {
        return read::refuse(&mut cx, "the graph has no such source");
    };
    let Some(found) = query.info().bindings.sources.get(source) else {
        return read::refuse(&mut cx, format!("the graph has no source {source}"));
    };
    for column in &columns {
        if let Err(failure) = found.entity.column(column) {
            return crate::errors::throw(&mut cx, failure);
        }
    }
    match query.keyset(source, &columns) {
        Ok(keyset) => Ok(cx
            .boxed(CursorHandle(keyset, pgorm::CursorPlan::default()))
            .upcast()),
        Err(reason) => read::refuse(&mut cx, reason),
    }
}

/// `graphQueryPages(query, size)`: the graph's pages of `size` rows.
fn query_pages(mut cx: FunctionContext) -> JsResult<JsValue> {
    let query = query(&mut cx)?;
    let size = page_size(&mut cx, 1)?;
    Ok(cx.boxed(PagesHandle(query.pages(), size)).upcast())
}

/// `graphQueryStream(query)`: `[statement, decoder]`, the statement a
/// stream sends and the decode `SelectGraph::stream` runs on each row.
// [spec:pgorm:req:bindings.entity-graphs+4]
fn query_stream(mut cx: FunctionContext) -> JsResult<JsValue> {
    let query = query(&mut cx)?;
    let statement = cx.boxed(Node::Select(query.statement()));
    let decoder = cx.boxed(DecoderHandle {
        decode: query.decode(),
        sources: query.pages().sources.clone(),
    });
    let pair = JsArray::new(&mut cx, 2);
    pair.set(&mut cx, 0, statement)?;
    pair.set(&mut cx, 1, decoder)?;
    Ok(pair.upcast())
}

/// A grouped read's answer as JavaScript reads it: `[rootColumns,
/// slotColumns, [[root, [slot, ..]], ..]]`.
struct Grouped(Arc<EntityInfo>, Arc<EntityInfo>, Groups);

impl Settled for Grouped {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        let Self(root, slot, groups) = *self;
        let roots = names(cx, &root)?;
        let slots = names(cx, &slot)?;
        let groups = list(cx, groups, |cx, (parent, children)| {
            let pair = JsArray::new(cx, 2);
            let parent = record(cx, parent)?;
            let children = list(cx, children, record)?;
            pair.set(cx, 0, parent)?;
            pair.set(cx, 1, children)?;
            Ok(pair.upcast())
        })?;
        let triple = JsArray::new(cx, 3);
        triple.set(cx, 0, roots)?;
        triple.set(cx, 1, slots)?;
        triple.set(cx, 2, groups)?;
        Ok(triple.upcast())
    }
}

/// `graphQueryGroupedJob(query)`: `all_grouped`, which only a graph of one
/// `Opt` slot has.
// [spec:pgorm:req:bindings.entity-graphs+4]
fn query_grouped_job(mut cx: FunctionContext) -> JsResult<JsValue> {
    let query = query(&mut cx)?;
    let Some(grouped) = query.grouped() else {
        return read::refuse(
            &mut cx,
            "allGrouped reads a graph of one Opt slot, as Rust's does",
        );
    };
    let sources = query.pages().sources.clone();
    let (root, slot) = (sources[0].clone(), sources[1].clone());
    let job: Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            let groups = grouped(db)
                .await
                .map_err(|error| entity_failure(&error, secrets))?;
            Ok(Box::new(Grouped(root, slot, groups)) as Output)
        })
    });
    Ok(cx.boxed(JobHandle::new(job)).upcast())
}
