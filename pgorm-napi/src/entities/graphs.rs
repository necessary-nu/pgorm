//! Registered graphs as JavaScript reaches them: a shape found by name, a
//! query over it under the aliases JavaScript chooses, and its rows decoded
//! by Rust's `GraphRow`.

use std::sync::Arc;

use neon::{prelude::*, types::Finalize};
use pgorm_bindings::entity::{GraphBackend, GraphChange as Change, GraphQuery as Query};

use super::{convert::GraphRows, inspected, strings, throw};
use crate::{
    connect::job::{Job, JobHandle, Output},
    errors::Failure,
    statements::Node,
    values::read,
};

#[derive(Debug)]
struct GraphHandle(Arc<dyn GraphBackend>);

impl Finalize for GraphHandle {}

#[derive(Debug)]
pub(super) struct GraphQueryHandle(pub(super) Query);

impl Finalize for GraphQueryHandle {}

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("graphNames", graph_names)?;
    cx.export_function("graphGet", graph_get)?;
    cx.export_function("graphDescribe", graph_describe)?;
    cx.export_function("graphFind", graph_find)?;
    cx.export_function("graphQueryAliases", query_aliases)?;
    cx.export_function("graphQueryChange", query_change)?;
    cx.export_function("graphQueryCol", query_col)?;
    cx.export_function("graphQueryInspect", query_inspect)?;
    cx.export_function("graphQueryJob", query_job)?;
    Ok(())
}

fn graph_names(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = held!(cx, super::RegistryHandle, 0);
    strings(
        &mut cx,
        registry
            .0
            .graphs()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter(),
    )
}

/// `graphGet(registry, name)`: the graph shape registered as `name`.
// [spec:pgorm:req:bindings.entity-graphs+4]
fn graph_get(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = held!(cx, super::RegistryHandle, 0);
    let name = text!(cx, 1);
    match registry.0.graphs().get(&name) {
        Some(factory) => Ok(cx.boxed(GraphHandle(factory.clone())).upcast()),
        None => throw(
            &mut cx,
            Failure::Construction(format!("no graph is registered as {name:?} in this module")),
        ),
    }
}

fn graph_describe(mut cx: FunctionContext) -> JsResult<JsValue> {
    let graph = held!(cx, GraphHandle, 0);
    Ok(cx.string(graph.info().describe().to_string()).upcast())
}

/// The aliases at argument 1, one per slot, or `null` for the defaults,
/// as `GraphInfo::aliases` takes them.
fn aliases(cx: &mut FunctionContext, factory: &dyn GraphBackend) -> NeonResult<Vec<String>> {
    let given = cx.argument::<JsValue>(1)?;
    let given = if given.is_a::<JsNull, _>(cx) || given.is_a::<JsUndefined, _>(cx) {
        None
    } else {
        let list = given.downcast_or_throw::<JsArray, _>(cx)?.to_vec(cx)?;
        let mut names = Vec::with_capacity(list.len());
        for item in list {
            names.push(read::text(cx, item, "an alias")?);
        }
        Some(names)
    };
    factory
        .info()
        .aliases(given)
        .or_else(|failure| throw(cx, failure))
}

/// `graphFind(graph, aliases)`: the graph built by its factory under the
/// aliases.
// [spec:pgorm:req:bindings.entity-graphs+4]
fn graph_find(mut cx: FunctionContext) -> JsResult<JsValue> {
    let factory = held!(cx, GraphHandle, 0);
    let aliases = aliases(&mut cx, factory.as_ref())?;
    Ok(cx.boxed(GraphQueryHandle(factory.find(aliases))).upcast())
}

fn query_aliases(mut cx: FunctionContext) -> JsResult<JsValue> {
    let query = held!(cx, GraphQueryHandle, 0);
    strings(&mut cx, query.aliases().to_vec().into_iter())
}

fn query_change(mut cx: FunctionContext) -> JsResult<JsValue> {
    let query = held!(cx, GraphQueryHandle, 0);
    let change = match super::step(&mut cx)? {
        super::Change::Filter(condition) => Change::Filter(condition),
        super::Change::Order(expr, order, nulls) => Change::Order(expr, order, nulls),
        super::Change::Limit(_) | super::Change::Offset(_) | super::Change::Join(..) => {
            return cx.throw_type_error("a registered graph takes where and orderBy steps");
        }
    };
    Ok(cx.boxed(GraphQueryHandle(query.change(change))).upcast())
}

/// `graphQueryCol(query, source, column)`: a column of a decoded source,
/// qualified as the query names it — the root by its table, a slot by its
/// alias.
// [spec:pgorm:req:bindings.entity-graphs+4]
fn query_col(mut cx: FunctionContext) -> JsResult<JsValue> {
    let query = held!(cx, GraphQueryHandle, 0);
    let source = cx.argument::<JsNumber>(1)?.value(&mut cx);
    let column = text!(cx, 2);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let index = (source.fract() == 0.0 && source >= 0.0).then_some(source as usize);
    let Some(index) = index else {
        return read::refuse(&mut cx, format!("the graph has no source {source}"));
    };
    match query.column(index, &column) {
        Ok(expr) => Ok(cx.boxed(Node::Expr(expr)).upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

fn query_inspect(mut cx: FunctionContext) -> JsResult<JsValue> {
    let query = held!(cx, GraphQueryHandle, 0);
    let optional = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    let compiled = query.compile(optional);
    inspected(&mut cx, compiled)
}

/// `graphQueryJob(query, optional)`: `all`, or `one_opt` with its `LIMIT 1`.
fn query_job(mut cx: FunctionContext) -> JsResult<JsValue> {
    let query = held!(cx, GraphQueryHandle, 0);
    let optional = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    let job: Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            let rows = query
                .run(db, optional)
                .await
                .map_err(|error| crate::errors::failure(&error, secrets))?;
            Ok(Box::new(GraphRows(query.info().bindings.entities(), rows)) as Output)
        })
    });
    Ok(cx.boxed(JobHandle::new(job)).upcast())
}
