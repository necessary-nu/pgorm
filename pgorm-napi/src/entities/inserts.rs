//! A registered entity's insert, typed by its row count as Rust's is:
//! `Insert::one` of one ActiveModel or `Insert::many` of a batch, its
//! conflict clause, and that count's own terminals — `exec`,
//! `exec_returning_pk(s)`, `exec_returning_model(s)` and
//! `exec_returning_upsert(s)`. They are statement terminals, as in Rust, so no
//! `ActiveModelBehavior` hook runs around them.

use neon::prelude::*;
use pgorm::pgorm_query::{IntoValueTuple, OnConflict};
use pgorm::{EntityTrait, Error, Insert, IntoActiveModel};

use super::{
    adapter::{EntityAdapter, InsertAnswer, Insertion, Insertions, concrete, model_of},
    convert::{Records, VersionRows, count, tagged},
    info::EntityInfo,
};
use crate::{
    codec::Codec,
    connect::job::{Database, Job, JobHandle, Output, Settled},
    errors::throw,
    rows,
    statements::{self, Node},
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("entityInsertJob", insert_job)?;
    Ok(())
}

impl<E> EntityAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    // [spec:pgorm:req:bindings.entity-inserts]
    pub(super) async fn write_insert(
        &self,
        db: Database<'_>,
        insertion: Insertion,
    ) -> Result<Insertions, Error> {
        let Insertion {
            actives,
            conflict,
            one,
            answer,
        } = insertion;
        let models = actives
            .iter()
            .map(concrete::<E>)
            .collect::<Result<Vec<_>, _>>()?;
        let model = |model| model_of::<E>(model, &self.info);
        if one {
            let active = models.into_iter().next().ok_or_else(|| {
                Error::Custom("an insert of one row needs one ActiveModel".to_owned())
            })?;
            let mut insert = Insert::one_in(self.entity, active);
            if let Some(conflict) = conflict {
                insert = insert.on_conflict(*conflict);
            }
            return Ok(match answer {
                InsertAnswer::Count => Insertions::Count(insert.exec(&db).await?),
                InsertAnswer::Keys => Insertions::Keys(vec![
                    insert
                        .exec_returning_pk(&db)
                        .await?
                        .into_value_tuple()
                        .into_iter()
                        .collect(),
                ]),
                InsertAnswer::Models => {
                    Insertions::Models(vec![model(insert.exec_returning_model(&db).await?)])
                }
                InsertAnswer::Upserts => Insertions::Upserts(
                    insert
                        .exec_returning_upsert(&db)
                        .await?
                        .into_iter()
                        .map(|row| self.upserted(row))
                        .collect(),
                ),
            });
        }
        let mut insert = Insert::many_in(self.entity, models);
        if let Some(conflict) = conflict {
            insert = insert.on_conflict(*conflict);
        }
        Ok(match answer {
            InsertAnswer::Count => Insertions::Count(insert.exec(&db).await?),
            InsertAnswer::Keys => Insertions::Keys(
                insert
                    .exec_returning_pks(&db)
                    .await?
                    .into_iter()
                    .map(|key| key.into_value_tuple().into_iter().collect())
                    .collect(),
            ),
            InsertAnswer::Models => Insertions::Models(
                insert
                    .exec_returning_models(&db)
                    .await?
                    .into_iter()
                    .map(model)
                    .collect(),
            ),
            InsertAnswer::Upserts => Insertions::Upserts(
                insert
                    .exec_returning_upserts(&db)
                    .await?
                    .into_iter()
                    .map(|row| self.upserted(row))
                    .collect(),
            ),
        })
    }
}

/// What an insert's terminal gave, as `lib/entity-versions.js` reads it: a
/// count, each key's values in key-column order, `[columns, [values,
/// model][]]` for models, or `[columns, [old | null, new][]]` for upserts.
struct Inserted(std::sync::Arc<EntityInfo>, Insertions);

impl Settled for Inserted {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        let Self(info, inserted) = *self;
        match inserted {
            Insertions::Count(written) => count(cx, written),
            Insertions::Models(models) => Box::new(Records(info, models)).into_js(cx),
            Insertions::Upserts(versions) => Box::new(VersionRows(info, versions)).into_js(cx),
            Insertions::Keys(keys) => {
                let codec = Codec::get(cx)?;
                let array = JsArray::new(cx, keys.len());
                for (index, values) in keys.into_iter().enumerate() {
                    let parts = JsArray::new(cx, values.len());
                    for (position, (name, value)) in info.primary_key.iter().zip(values).enumerate()
                    {
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
}

/// `entityInsertJob(entity, actives, conflict, one, answer)`: `Insert::one`
/// of the one ActiveModel or `Insert::many` of them all, with the conflict
/// clause if given, then the terminal `answer` names — `"count"`, `"keys"`,
/// `"models"` or `"upserts"` — of that row count.
// [spec:pgorm:req:bindings.entity-versions+3]
// [spec:pgorm:req:bindings.entity-inserts]
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
    let conflict: Option<Box<OnConflict>> = if conflict.is_a::<JsNull, _>(&mut cx) {
        None
    } else {
        Some(Box::new(match statements::node(&mut cx, conflict) {
            Some(Node::Conflict(conflict)) => conflict,
            Some(Node::ConflictUpdate(update)) => update.into(),
            _ => return cx.throw_type_error("onConflict takes a Conflict action"),
        }))
    };
    let one = cx.argument::<JsBoolean>(3)?.value(&mut cx);
    let answer = match cx.argument::<JsString>(4)?.value(&mut cx).as_str() {
        "count" => InsertAnswer::Count,
        "keys" => InsertAnswer::Keys,
        "models" => InsertAnswer::Models,
        "upserts" => InsertAnswer::Upserts,
        _ => {
            return cx
                .throw_type_error("an insert answers with its count, keys, models or upserts");
        }
    };
    let insertion = Insertion {
        actives,
        conflict,
        one,
        answer,
    };
    let job: Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            let inserted = entity
                .insert(db, insertion)
                .await
                .map_err(|error| crate::errors::failure(&error, secrets))?;
            Ok(Box::new(Inserted(entity.info().clone(), inserted)) as Output)
        })
    });
    Ok(cx.boxed(JobHandle::new(job)))
}
