//! A registered entity's insert, typed by its row count as Rust's is:
//! `Insert::one` of one ActiveModel or `Insert::many` of a batch, its
//! conflict clause, and that count's own terminals — `exec`,
//! `exec_returning_pk(s)`, `exec_returning_model(s)` and
//! `exec_returning_upsert(s)` — or the `TryInsert` its
//! `on_conflict_do_nothing` or `on_empty_do_nothing` makes, answering
//! `TryInsertResult`. pgorm-bindings builds and runs each statement, so
//! Python's inserts are the same Rust. They are statement terminals, as in
//! Rust, so no `ActiveModelBehavior` hook runs around them.

use neon::prelude::*;
use pgorm::TryInsertResult;
use pgorm::pgorm_query::OnConflict;
use pgorm_bindings::entity::{EntityInfo, Insertion, Insertions, Model, Versions};
use pgorm_bindings::{
    InsertConflict, InsertTerminal, InsertWrite, Inserted as Written, TryInsertTerminal,
};

use super::convert::{Records, VersionRows, count, tagged};
use crate::{
    codec::Codec,
    connect::job::{Job, JobHandle, Output, Settled},
    errors::throw,
    rows,
    statements::{self, Node},
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("entityInsertJob", insert_job)?;
    Ok(())
}

/// What an insert's terminal gave, as `lib/entity-inserts.js` reads it: a
/// count, each key's values in key-column order, `[columns, [values,
/// model][]]` for models, or `[columns, [old | null, new][]]` for upserts;
/// for a try-insert, `tried`, `["inserted", that]`, `["conflicted"]` or
/// `["empty"]`.
struct Inserted(std::sync::Arc<EntityInfo>, Insertions, bool);

impl Settled for Inserted {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        let Self(info, result, tried) = *self;
        let inserted = match (result, tried) {
            (TryInsertResult::Inserted(inserted), false) => return written(cx, &info, inserted),
            (TryInsertResult::Inserted(inserted), true) => inserted,
            (TryInsertResult::Conflicted, true) => return kind(cx, "conflicted"),
            (TryInsertResult::Empty, true) => return kind(cx, "empty"),
            (_, false) => return cx.throw_error("an insert answered as a try-insert"),
        };
        let answer = JsArray::new(cx, 2);
        let name = cx.string("inserted");
        answer.set(cx, 0, name)?;
        let value = written(cx, &info, inserted)?;
        answer.set(cx, 1, value)?;
        Ok(answer.upcast())
    }
}

/// `[kind]`: a try-insert's answer that holds nothing it wrote.
fn kind<'cx>(cx: &mut Cx<'cx>, kind: &str) -> JsResult<'cx, JsValue> {
    let answer = JsArray::new(cx, 1);
    let name = cx.string(kind);
    answer.set(cx, 0, name)?;
    Ok(answer.upcast())
}

/// What the terminal wrote, as the addon gives it to JavaScript.
fn written<'cx>(
    cx: &mut Cx<'cx>,
    info: &std::sync::Arc<EntityInfo>,
    inserted: Written<Model, Versions>,
) -> JsResult<'cx, JsValue> {
    match inserted {
        Written::Count(written) => count(cx, written),
        Written::Models(models) => Box::new(Records(info.clone(), models)).into_js(cx),
        Written::Upserts(versions) => Box::new(VersionRows(info.clone(), versions)).into_js(cx),
        Written::Keys(keys) => {
            let codec = Codec::get(cx)?;
            let array = JsArray::new(cx, keys.len());
            for (index, values) in keys.into_iter().enumerate() {
                let parts = JsArray::new(cx, values.arity());
                for (position, (name, value)) in info.primary_key.iter().zip(values).enumerate() {
                    let column = info.column(name).or_else(|failure| throw(cx, failure))?;
                    let value = tagged(column, value).or_else(|failure| throw(cx, failure))?;
                    let value = rows::value(cx, codec, value, false)?;
                    parts.set(cx, u32::try_from(position).unwrap_or(u32::MAX), value)?;
                }
                array.set(cx, u32::try_from(index).unwrap_or(u32::MAX), parts)?;
            }
            Ok(array.upcast())
        }
    }
}

/// `entityInsertJob(entity, actives, conflict, one, answer, attempt)`:
/// `Insert::one` of the one ActiveModel or `Insert::many` of them all, as
/// the `TryInsert` `on_conflict_do_nothing` or `on_empty_do_nothing` makes
/// of it with `attempt`, with the conflict clause if given — `"primaryKey"`
/// for `on_conflict_do_nothing`'s — then the terminal `answer` names, of
/// that row count: `"count"`, `"keys"`, `"models"`, or for an `Insert`
/// `"upserts"`.
// [spec:pgorm:req:bindings.entity-versions+3]
// [spec:pgorm:req:bindings.entity-inserts]
// [spec:pgorm:req:bindings.entity-try-inserts]
fn insert_job(mut cx: FunctionContext) -> JsResult<JsBox<JobHandle>> {
    let entity = held!(cx, super::EntityHandle, 0);
    let list = cx.argument::<JsArray>(1)?.to_vec(&mut cx)?;
    let mut actives = Vec::with_capacity(list.len());
    for item in list {
        let active = item
            .downcast_or_throw::<JsBox<super::ActiveHandle>, _>(&mut cx)?
            .0
            .clone();
        if active.info().name != entity.info().name {
            return cx.throw_type_error(format!(
                "the ActiveModel is {}'s, not {}'s",
                active.info().name,
                entity.info().name
            ));
        }
        actives.push(active);
    }
    let conflict = cx.argument::<JsValue>(2)?;
    let key = conflict
        .downcast::<JsString, _>(&mut cx)
        .is_ok_and(|name| name.value(&mut cx) == "primaryKey");
    let clause: Option<Box<OnConflict>> = if key || conflict.is_a::<JsNull, _>(&mut cx) {
        None
    } else {
        Some(Box::new(match statements::node(&mut cx, conflict) {
            Some(Node::Conflict(conflict)) => conflict,
            Some(Node::ConflictUpdate(update)) => update.into(),
            _ => return cx.throw_type_error("onConflict takes a Conflict action"),
        }))
    };
    let one = cx.argument::<JsBoolean>(3)?.value(&mut cx);
    let answer = cx.argument::<JsString>(4)?.value(&mut cx);
    let attempt = cx.argument::<JsBoolean>(5)?.value(&mut cx);
    let write = if attempt {
        let conflict = match key {
            true => Some(InsertConflict::PrimaryKey),
            false => clause.map(InsertConflict::Clause),
        };
        InsertWrite::Try(
            conflict,
            match answer.as_str() {
                "count" => TryInsertTerminal::Count,
                "keys" => TryInsertTerminal::Keys,
                "models" => TryInsertTerminal::Models,
                _ => {
                    return cx
                        .throw_type_error("a try-insert answers with its count, keys or models");
                }
            },
        )
    } else {
        if key {
            return cx.throw_type_error("only a try-insert does nothing on its primary key");
        }
        InsertWrite::Insert(
            clause,
            match answer.as_str() {
                "count" => InsertTerminal::Count,
                "keys" => InsertTerminal::Keys,
                "models" => InsertTerminal::Models,
                "upserts" => InsertTerminal::Upserts,
                _ => {
                    return cx.throw_type_error(
                        "an insert answers with its count, keys, models or upserts",
                    );
                }
            },
        )
    };
    let insertion = Insertion {
        models: actives,
        one,
        write,
    };
    let job: Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            let inserted = entity
                .insert(db, insertion)
                .await
                .map_err(|error| crate::errors::failure(&error, secrets))?;
            Ok(Box::new(Inserted(entity.info().clone(), inserted, attempt)) as Output)
        })
    });
    Ok(cx.boxed(JobHandle::new(job)))
}
