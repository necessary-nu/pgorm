//! Registered source tuples as JavaScript reaches them: found by name,
//! selected as a pipeline's last stage under qualifiers JavaScript chooses,
//! and run through `SelectedSources`'s own terminals.

use std::sync::Arc;

use neon::{prelude::*, types::Finalize};
use pgorm::pipeline as pl;

use super::{
    adapter::Read,
    convert::GraphRows,
    inspected,
    sources::{Selected, SourcesFactory},
    strings, throw,
};
use crate::{
    connect::job::{Job, JobHandle, Output},
    errors::Failure,
    statements::{self, Node, pipeline::Part},
    values::read,
};

#[derive(Debug)]
struct SourcesHandle(Arc<dyn SourcesFactory>);

impl Finalize for SourcesHandle {}

#[derive(Debug)]
struct SelectedHandle(Selected);

impl Finalize for SelectedHandle {}

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("sourcesNames", sources_names)?;
    cx.export_function("sourcesGet", sources_get)?;
    cx.export_function("sourcesDescribe", sources_describe)?;
    cx.export_function("sourcesSelect", sources_select)?;
    cx.export_function("selectedInspect", selected_inspect)?;
    cx.export_function("selectedJob", selected_job)?;
    Ok(())
}

fn sources_names(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = held!(cx, super::RegistryHandle, 0);
    strings(
        &mut cx,
        registry
            .sources
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter(),
    )
}

/// `sourcesGet(registry, name)`: the source tuple registered as `name`.
// [spec:pgorm:req:bindings.pipeline-sources+3]
fn sources_get(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = held!(cx, super::RegistryHandle, 0);
    let name = text!(cx, 1);
    match registry.sources.get(&name) {
        Some(factory) => Ok(cx.boxed(SourcesHandle(factory.clone())).upcast()),
        None => throw(
            &mut cx,
            Failure::Construction(format!(
                "no source tuple is registered as {name:?} in this module"
            )),
        ),
    }
}

fn sources_describe(mut cx: FunctionContext) -> JsResult<JsValue> {
    let factory = held!(cx, SourcesHandle, 0);
    Ok(cx.string(factory.info().describe().to_string()).upcast())
}

/// The qualifiers at argument 2: one identifier per source, the sources'
/// tables by default.
fn qualifiers(cx: &mut FunctionContext, factory: &dyn SourcesFactory) -> NeonResult<Vec<String>> {
    let entities = &factory.info().bindings.entities;
    let given = cx.argument::<JsValue>(2)?;
    if given.is_a::<JsNull, _>(cx) || given.is_a::<JsUndefined, _>(cx) {
        return Ok(entities.iter().map(|entity| entity.table.clone()).collect());
    }
    let list = given.downcast_or_throw::<JsArray, _>(cx)?.to_vec(cx)?;
    if list.len() != entities.len() {
        return read::refuse(
            cx,
            format!(
                "the tuple has {} sources, and {} qualifiers were given",
                entities.len(),
                list.len()
            ),
        );
    }
    let mut names = Vec::with_capacity(list.len());
    for item in list {
        let name = read::text(cx, item, "a qualifier")?;
        if name.is_empty() || name.len() > 63 || name.contains('\0') {
            return read::refuse(cx, "a qualifier is 1–63 UTF-8 bytes without NUL");
        }
        names.push(name);
    }
    Ok(names)
}

/// `sourcesSelect(sources, pipeline, qualifiers)`: the pipeline with the
/// tuple's projection as its last stage, each source read under its
/// qualifier.
// [spec:pgorm:req:bindings.pipeline-sources+3]
fn sources_select(mut cx: FunctionContext) -> JsResult<JsValue> {
    let factory = held!(cx, SourcesHandle, 0);
    let value = cx.argument::<JsValue>(1)?;
    let pipeline: pl::Pipeline = match statements::node(&mut cx, value) {
        Some(Node::Pipeline(part)) => match *part {
            Part::Pipeline(pipeline) => pipeline,
            other => {
                let what = other.describe();
                return cx
                    .throw_type_error(format!("sources are selected from a Pipeline, not {what}"));
            }
        },
        _ => return cx.throw_type_error("sources are selected from a Pipeline"),
    };
    let qualifiers = qualifiers(&mut cx, factory.as_ref())?;
    Ok(cx
        .boxed(SelectedHandle(factory.select(pipeline, qualifiers)))
        .upcast())
}

fn read_terminal(cx: &mut FunctionContext) -> NeonResult<Read> {
    match cx.argument::<JsString>(1)?.value(cx).as_str() {
        "all" => Ok(Read::All),
        "one" => Ok(Read::One),
        "optional" => Ok(Read::OneOpt),
        _ => cx.throw_type_error("a terminal is all, one or optional"),
    }
}

/// `selectedInspect(selected, terminal)`: the SQL and values the terminal
/// sends, `one` and `optional` taking one row; a pipeline reshaped before the
/// selection is a `ConstructionError`, as pgorm refuses it.
// [spec:pgorm:req:bindings.pipeline-sources+3]
fn selected_inspect(mut cx: FunctionContext) -> JsResult<JsValue> {
    let selected = held!(cx, SelectedHandle, 0);
    let read = read_terminal(&mut cx)?;
    match selected.compile(read != Read::All) {
        Ok(compiled) => inspected(&mut cx, compiled),
        Err(error) => read::refuse(&mut cx, error.to_string()),
    }
}

/// `selectedJob(selected, terminal)`: `SelectedSources::all`, `one` or
/// `one_opt`, each row a tuple of the sources' optional models.
fn selected_job(mut cx: FunctionContext) -> JsResult<JsValue> {
    let selected = held!(cx, SelectedHandle, 0);
    let read = read_terminal(&mut cx)?;
    if let Err(error) = selected.compile(false) {
        return read::refuse(&mut cx, error.to_string());
    }
    let job: Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            let rows = selected
                .run(db, read)
                .await
                .map_err(|error| crate::errors::failure(&error, secrets))?;
            let rows = rows
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|model| model.map(|model| model.0))
                        .collect()
                })
                .collect();
            let entities = selected.info().bindings.entities.clone();
            Ok(Box::new(GraphRows(entities, rows)) as Output)
        })
    });
    Ok(cx.boxed(JobHandle::new(job)).upcast())
}
