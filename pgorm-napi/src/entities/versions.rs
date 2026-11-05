//! The registered entity's updates that read each written row's two
//! versions, reached as the statement terminals they are in Rust — no
//! `ActiveModelBehavior` hook runs around them: `UpdateOne`'s
//! `exec_returning_change` and `UpdateMany`'s `exec_returning_changes`. An
//! insert's, its upserts' among them, are in `inserts`.

use neon::prelude::*;

use super::{
    adapter::{Active, Assignment, VersionWrite},
    condition,
    convert::{VersionRows, column_value, entity_failure},
};
use crate::{
    connect::job::{Job, JobHandle, Output},
    statements::{self, Node},
    values::read,
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("entityChangeJob", change_job)?;
    cx.export_function("entityChangesJob", changes_job)?;
    Ok(())
}

/// The job running `write` on the entity, its rows' versions its outcome.
fn versions_job(
    entity: std::sync::Arc<dyn super::EntityBackend>,
    write: VersionWrite,
) -> JobHandle {
    let job: Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            let versions = entity
                .versions(db, write)
                .await
                .map_err(|error| entity_failure(&error, secrets))?;
            Ok(Box::new(VersionRows(entity.info().clone(), versions)) as Output)
        })
    });
    JobHandle::new(job)
}

/// An ActiveModel the entity at index 0 made, refused if another
/// registration made it.
fn own_active(cx: &mut FunctionContext, index: usize) -> NeonResult<Active> {
    let entity = held!(cx, super::EntityHandle, 0);
    let active = held!(cx, super::ActiveHandle, index);
    if active.info().name != entity.info().name {
        return read::refuse(
            cx,
            format!(
                "the ActiveModel is {}'s, not {}'s",
                active.info().name,
                entity.info().name
            ),
        );
    }
    Ok(active)
}

/// `entityChangeJob(entity, active)`: `Update::one(active)
/// .exec_returning_change`.
// [spec:pgorm:req:bindings.entity-versions+3]
fn change_job(mut cx: FunctionContext) -> JsResult<JsBox<JobHandle>> {
    let entity = held!(cx, super::EntityHandle, 0);
    let active = own_active(&mut cx, 1)?;
    Ok(cx.boxed(versions_job(entity, VersionWrite::Change(active))))
}

/// `entityChangesJob(entity, assignments, predicate)`: `Update::many`, each
/// assignment `[column, value]` written through the column's `save_as` or
/// an expression as written, filtered by the predicate, then
/// `exec_returning_changes`.
// [spec:pgorm:req:bindings.entity-versions+3]
fn changes_job(mut cx: FunctionContext) -> JsResult<JsBox<JobHandle>> {
    let entity = held!(cx, super::EntityHandle, 0);
    let pairs = cx.argument::<JsArray>(1)?.to_vec(&mut cx)?;
    let mut assignments = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let pair = pair.downcast_or_throw::<JsArray, _>(&mut cx)?;
        let column = pair.get::<JsString, _, _>(&mut cx, 0)?.value(&mut cx);
        let data: Handle<JsValue> = pair.get(&mut cx, 1)?;
        let info = match entity.info().column(&column) {
            Ok(info) => info.clone(),
            Err(failure) => {
                let error = failure.into_js(&mut cx)?;
                return cx.throw(error);
            }
        };
        let assignment = match statements::node(&mut cx, data) {
            Some(Node::Expr(expr)) => Assignment::Expr(expr),
            _ => Assignment::Value(column_value(&mut cx, &info, data)?),
        };
        assignments.push((column, assignment));
    }
    let predicate = cx.argument::<JsValue>(2)?;
    let filter = condition(&mut cx, predicate)?;
    Ok(cx.boxed(versions_job(
        entity,
        VersionWrite::Changes(assignments, filter),
    )))
}
