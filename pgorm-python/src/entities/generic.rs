//! Each adapter below is monomorphized at downstream module build time.

use crate::execution::Database;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use pgorm::pgorm_query::{Expr, IntoValueTuple, Name, SimpleExpr, Value, Values};
use pgorm::{
    ActiveModelBehavior, ActiveModelTrait, ActiveValue, Change as RustChange, ColumnTrait,
    EntityTrait, Error, FromQueryResult, Insert, IntoActiveModel, Iterable, ModelTrait,
    QueryFilter, QueryOrder, QuerySelect, QueryTrait, StaticName, Update, Upserted,
};

use super::{backend::*, metadata::EntityInfo};

fn column<E: EntityTrait>(name: &str) -> Result<E::Column, Error> {
    E::Column::iter()
        .find(|c| c.as_str() == name)
        .ok_or_else(|| Error::Type("unknown compiled entity column".to_owned()))
}

#[derive(Debug)]
pub(crate) struct EntityAdapter<E> {
    pub(crate) info: Arc<EntityInfo>,
    /// The registered entity value, whose table every statement names.
    pub(crate) entity: E,
}

// [spec:pgorm:req:bindings.entities+6]
impl<E> EntityBackend for EntityAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn info(&self) -> &Arc<EntityInfo> {
        &self.info
    }

    // [spec:pgorm:req:bindings.schema-from-entity]
    fn schema(&self) -> Result<crate::schema::PyEntitySchema, Error> {
        let schema = pgorm::Schema::new();
        Ok(crate::schema::PyEntitySchema {
            table: schema.create_table_from_entity(self.entity)?,
            enums: schema.create_enum_from_entity(self.entity),
            indexes: schema.create_index_from_entity(self.entity),
            comments: schema.create_comments_from_entity(self.entity),
        })
    }

    fn select(&self) -> Select {
        Arc::new(SelectAdapter::<E> {
            query: E::find_in(self.entity),
            info: self.info.clone(),
        })
    }

    fn active(&self) -> Active {
        Arc::new(ActiveAdapter::<E> {
            value: E::ActiveModel::new(),
            info: self.info.clone(),
        })
    }

    fn expression(&self, name: &str) -> Result<SimpleExpr, Error> {
        Ok(Expr::col(column::<E>(name)?.as_column_ref()).into())
    }

    // [spec:pgorm:req:bindings.pipeline-sources+3]
    fn pipeline_column(&self, name: &str) -> Result<Arc<dyn PipelineColumn>, Error> {
        Ok(Arc::new(RegisteredColumn::<E>(column::<E>(name)?)))
    }

    fn compare(
        &self,
        name: &str,
        comparison: Comparison,
        value: Value,
    ) -> Result<SimpleExpr, Error> {
        let column = column::<E>(name)?;
        Ok(match comparison {
            Comparison::Eq => column.eq(value),
            Comparison::Ne => column.ne(value),
            Comparison::Gt => column.gt(value),
            Comparison::Ge => column.gte(value),
            Comparison::Lt => column.lt(value),
            Comparison::Le => column.lte(value),
            Comparison::IsDistinctFrom => column.is_distinct_from(value),
            Comparison::IsNotDistinctFrom => column.is_not_distinct_from(value),
            Comparison::Contains => column.includes(value),
            Comparison::ContainedBy => column.contained_by(value),
            Comparison::Overlaps => column.overlaps(value),
            Comparison::StrictlyLeftOf => column.strictly_left_of(value),
            Comparison::StrictlyRightOf => column.strictly_right_of(value),
            Comparison::NotExtendRightOf => column.not_extend_right_of(value),
            Comparison::NotExtendLeftOf => column.not_extend_left_of(value),
            Comparison::AdjacentTo => column.adjacent_to(value),
        })
    }

    // [spec:pgorm:req:bindings.entity-reads+3]
    fn quantified(
        &self,
        name: &str,
        comparison: Comparison,
        all: bool,
        array: Value,
    ) -> Result<SimpleExpr, Error> {
        let column = column::<E>(name)?;
        Ok(match all {
            false => column.compare_any(comparison.operator(), array),
            true => column.compare_all(comparison.operator(), array),
        })
    }

    // [spec:pgorm:req:bindings.column-operators+3]
    fn between(
        &self,
        name: &str,
        between: Between,
        lower: Value,
        upper: Value,
    ) -> Result<SimpleExpr, Error> {
        let column = column::<E>(name)?;
        Ok(match (between.negated, between.symmetric) {
            (false, false) => column.between(lower, upper),
            (true, false) => column.not_between(lower, upper),
            (false, true) => column.between_symmetric(lower, upper),
            (true, true) => column.not_between_symmetric(lower, upper),
        })
    }

    // [spec:pgorm:req:bindings.entity-reads+3]
    fn membership(&self, name: &str, negated: bool, members: Members) -> Result<SimpleExpr, Error> {
        let column = column::<E>(name)?;
        Ok(match (members, negated) {
            (Members::Values(values), false) => column.is_in(values),
            (Members::Values(values), true) => column.is_not_in(values),
            (Members::Subquery(select), false) => column.in_subquery(*select),
            (Members::Subquery(select), true) => column.not_in_subquery(*select),
        })
    }

    fn versions<'a>(
        &'a self,
        db: Database<'a>,
        write: VersionWrite,
    ) -> BoxFuture<'a, Result<Vec<Versions>, Error>> {
        Box::pin(self.write_versions(db, write))
    }

    fn insert<'a>(
        &'a self,
        db: Database<'a>,
        insertion: Insertion,
    ) -> BoxFuture<'a, Result<Insertions, Error>> {
        Box::pin(self.write_insert(db, insertion))
    }

    // [spec:pgorm:req:bindings.disjoint+2]
    fn decode(&self, row: &pgorm::QueryResult, prefix: &str) -> Result<Model, Error> {
        E::Model::from_query_result(row, prefix).map(|value| self.model(value))
    }
}

/// One of `E`'s columns, resolved when the pipeline expression naming it was
/// built, so lowering it cannot miss.
struct RegisteredColumn<E: EntityTrait>(E::Column);

impl<E: EntityTrait> std::fmt::Debug for RegisteredColumn<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("RegisteredColumn").field(&self.0).finish()
    }
}

// [spec:pgorm:req:bindings.pipeline-sources+3]
impl<E: EntityTrait + Send + Sync> PipelineColumn for RegisteredColumn<E> {
    fn lower<'brand>(&self, qualifier: &str) -> pgorm::pipeline::Expr<'brand> {
        pgorm::pipeline::named_runtime(E::default(), Name::runtime(qualifier)).col(self.0)
    }
}

/// The compiled ActiveModel a type-erased handle holds, refused if it was
/// minted by another registration.
fn concrete<E>(active: &Active) -> Result<E::ActiveModel, Error>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    active
        .as_any()
        .downcast_ref::<ActiveAdapter<E>>()
        .map(|adapter| adapter.value.clone())
        .ok_or_else(|| Error::Type("ActiveModel belongs to another entity registration".to_owned()))
}

impl<E> EntityAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn model(&self, value: E::Model) -> Model {
        Arc::new(ModelAdapter::<E> {
            value,
            info: self.info.clone(),
        })
    }

    fn change(&self, change: RustChange<E::Model>) -> Versions {
        Versions {
            old: Some(self.model(change.old)),
            new: self.model(change.new),
        }
    }

    fn upserted(&self, upserted: Upserted<E::Model>) -> Versions {
        match upserted {
            Upserted::Inserted(model) => Versions {
                old: None,
                new: self.model(model),
            },
            Upserted::Updated(change) => self.change(change),
        }
    }

    async fn write_versions(
        &self,
        db: Database<'_>,
        write: VersionWrite,
    ) -> Result<Vec<Versions>, Error> {
        match write {
            VersionWrite::Change(active) => {
                let change = Update::one_in(self.entity, concrete::<E>(&active)?)?
                    .exec_returning_change(&db)
                    .await?;
                Ok(vec![self.change(change)])
            }
            VersionWrite::Changes(assignments, filter) => {
                let mut update = Update::many(self.entity);
                for (name, assignment) in assignments {
                    let column = column::<E>(&name)?;
                    let expression = match assignment {
                        Assignment::Value(value) => column.save_as(Expr::val(value)),
                        Assignment::Expr(expression) => expression,
                    };
                    update = update.col_expr(column, expression);
                }
                let changes = update.filter(filter).exec_returning_changes(&db).await?;
                Ok(changes
                    .into_iter()
                    .map(|change| self.change(change))
                    .collect())
            }
        }
    }

    // [spec:pgorm:req:bindings.entity-inserts]
    async fn write_insert(
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
        if one {
            let model = models.into_iter().next().ok_or_else(|| {
                Error::Type("an insert of one row needs one ActiveModel".to_owned())
            })?;
            let mut insert = Insert::one_in(self.entity, model);
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
                    Insertions::Models(vec![self.model(insert.exec_returning_model(&db).await?)])
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
                    .map(|model| self.model(model))
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

#[derive(Debug)]
struct SelectAdapter<E: EntityTrait> {
    query: pgorm::Select<E>,
    info: Arc<EntityInfo>,
}

impl<E> SelectBackend for SelectAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn info(&self) -> &Arc<EntityInfo> {
        &self.info
    }

    fn change(&self, change: Change) -> Select {
        let query = self.query.clone();
        let query = match change {
            Change::Filter(condition) => query.filter(condition),
            Change::Order(expr, order, Some(nulls)) => {
                query.order_by_with_nulls(expr, order, nulls)
            }
            Change::Order(expr, order, None) => query.order_by(expr, order),
            Change::Limit(limit) => query.limit(limit),
            Change::Offset(offset) => query.offset(offset),
        };
        Arc::new(Self {
            query,
            info: self.info.clone(),
        })
    }

    fn compile(&self, terminal: Terminal) -> (String, Values) {
        let mut query = self.query.clone();
        if !matches!(terminal, Terminal::All) {
            QueryTrait::query(&mut query).narrow_limit_to_one();
        }
        query.build()
    }

    fn run<'a>(
        &'a self,
        db: Database<'a>,
        terminal: Terminal,
    ) -> BoxFuture<'a, Result<Vec<Model>, Error>> {
        Box::pin(self.read(db, terminal))
    }

    fn run_raw<'a>(
        &'a self,
        db: Database<'a>,
        sql: String,
        values: Values,
        terminal: Terminal,
    ) -> BoxFuture<'a, Result<Vec<Model>, Error>> {
        Box::pin(self.read_raw(db, sql, values, terminal))
    }
}

#[derive(Debug)]
pub(crate) struct ModelAdapter<E: EntityTrait> {
    pub(crate) value: E::Model,
    pub(crate) info: Arc<EntityInfo>,
}

impl<E> ModelBackend for ModelAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn info(&self) -> &Arc<EntityInfo> {
        &self.info
    }

    fn get(&self, name: &str) -> Result<Value, Error> {
        Ok(self.value.get(column::<E>(name)?))
    }

    fn set(&self, name: &str, value: Value) -> Result<Model, Error> {
        let mut model = self.value.clone();
        model.set(column::<E>(name)?, value)?;
        Ok(Arc::new(Self {
            value: model,
            info: self.info.clone(),
        }))
    }

    fn active(&self) -> Active {
        Arc::new(ActiveAdapter::<E> {
            value: self.value.clone().into_active_model(),
            info: self.info.clone(),
        })
    }
}

#[derive(Debug)]
struct ActiveAdapter<E: EntityTrait> {
    value: E::ActiveModel,
    info: Arc<EntityInfo>,
}

impl<E> ActiveBackend for ActiveAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn info(&self) -> &Arc<EntityInfo> {
        &self.info
    }

    fn get(&self, name: &str) -> Result<ActiveValue<Value>, Error> {
        self.value.get(column::<E>(name)?)
    }

    fn set(&self, name: &str, value: Value) -> Result<Active, Error> {
        let mut active = self.value.clone();
        active.set(column::<E>(name)?, value)?;
        Ok(Arc::new(Self {
            value: active,
            info: self.info.clone(),
        }))
    }

    fn not_set(&self, name: &str) -> Result<Active, Error> {
        let mut active = self.value.clone();
        active.not_set(column::<E>(name)?);
        Ok(Arc::new(Self {
            value: active,
            info: self.info.clone(),
        }))
    }

    fn reset(&self, name: &str) -> Result<Active, Error> {
        let mut active = self.value.clone();
        active.reset(column::<E>(name)?);
        Ok(Arc::new(Self {
            value: active,
            info: self.info.clone(),
        }))
    }

    fn run<'a>(&'a self, db: Database<'a>, write: Write) -> BoxFuture<'a, Result<Written, Error>> {
        Box::pin(self.write(db, write))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl<E> SelectAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    async fn read(&self, db: Database<'_>, terminal: Terminal) -> Result<Vec<Model>, Error> {
        let query = self.query.clone();
        let models = match terminal {
            Terminal::All => query.all(&db).await?,
            Terminal::One => vec![query.one(&db).await?],
            Terminal::Optional => query.one_opt(&db).await?.into_iter().collect(),
        };
        Ok(self.models(models))
    }

    // [spec:pgorm:req:bindings.raw-models]
    async fn read_raw(
        &self,
        db: Database<'_>,
        sql: String,
        values: Values,
        terminal: Terminal,
    ) -> Result<Vec<Model>, Error> {
        let query = self.query.clone().from_raw_sql(&sql, values);
        let models = match terminal {
            Terminal::All => query.all(&db).await?,
            Terminal::One => vec![query.one(&db).await?],
            Terminal::Optional => query.one_opt(&db).await?.into_iter().collect(),
        };
        Ok(self.models(models))
    }

    fn models(&self, models: Vec<E::Model>) -> Vec<Model> {
        models
            .into_iter()
            .map(|value| {
                Arc::new(ModelAdapter::<E> {
                    value,
                    info: self.info.clone(),
                }) as Model
            })
            .collect()
    }
}

impl<E> ActiveAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    async fn write(&self, db: Database<'_>, write: Write) -> Result<Written, Error> {
        let (entity, active) = (self.info.entity::<E>()?, self.value.clone());
        let model = match write {
            Write::Insert => active.insert_in(entity, &db).await?,
            Write::Update => active.update_in(entity, &db).await?,
            Write::Delete => return active.delete_in(entity, &db).await.map(Written::Count),
        };
        Ok(Written::Model(Arc::new(ModelAdapter::<E> {
            value: model,
            info: self.info.clone(),
        })))
    }
}
