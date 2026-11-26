//! The adapters each registration monomorphizes when the application's
//! module is built.

use std::{fmt, sync::Arc};

use futures_util::future::BoxFuture;
use pgorm::pgorm_query::{Expr, Name, SelectStatement, SimpleExpr, Value, Values};
use pgorm::{
    ActiveModelBehavior, Change as RustChange, Column, DecodeRow, Entity, Error, FromQueryResult,
    IntoActiveModel, Iterable, QueryFilter, QueryOrder, QuerySelect, Relation, RelationDef,
    SelectModel, Statement, StaticName, Upserted,
};

use super::{
    Active, Assignment, Between, Change, Comparison, EntityBackend, EntityInfo, EntitySchema,
    Insertion, Insertions, Members, Model, PipelineColumn, Read, Select, SelectBackend,
    VersionWrite, Versions,
    active::{ActiveAdapter, concrete, model_of},
    reach::Reach,
    reads::{self, Decode, Keyset, Pages},
};
use crate::{Database, Failure, scope::Scoping};

/// The entity's column of SQL name `name`, refused as one the registration
/// lacks.
pub(crate) fn column<E: Entity>(info: &EntityInfo, name: &str) -> Result<E::Column, Failure> {
    <E::Column as Iterable>::iter()
        .find(|column| column.as_str() == name)
        .ok_or_else(|| Failure::Construction(format!("{} has no column {name:?}", info.name)))
}

/// A registered entity: what the binding knows of it, and how its
/// statements reach it.
#[derive(Debug)]
pub struct EntityAdapter<E: Entity> {
    /// What the binding knows of the entity: its table, columns and key.
    pub info: Arc<EntityInfo>,
    pub(crate) reach: Arc<dyn Reach<E>>,
}

/// One of `E`'s columns, resolved when the pipeline expression naming it was
/// built, so lowering it cannot miss.
struct RegisteredColumn<E: Entity>(E::Column);

impl<E: Entity> fmt::Debug for RegisteredColumn<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RegisteredColumn").field(&self.0).finish()
    }
}

// [spec:pgorm:req:bindings.pipeline-sources+3]
impl<E: Entity + Send + Sync> PipelineColumn for RegisteredColumn<E> {
    fn lower<'brand>(&self, qualifier: Name) -> pgorm::pipeline::Expr<'brand> {
        pgorm::pipeline::named_runtime(E::default(), qualifier).col(self.0)
    }
}

// [spec:pgorm:req:bindings.entities+6]
impl<E> EntityBackend for EntityAdapter<E>
where
    E: Entity + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn info(&self) -> &Arc<EntityInfo> {
        &self.info
    }

    // [spec:pgorm:req:bindings.schema-from-entity]
    fn schema(&self) -> Result<EntitySchema, Error> {
        let (schema, entity) = (pgorm::Schema::new(), self.reach.entity());
        Ok(EntitySchema {
            table: schema.create_table_from_entity(entity)?,
            enums: schema.create_enum_from_entity(entity),
            indexes: schema.create_index_from_entity(entity),
            comments: schema.create_comments_from_entity(entity),
        })
    }

    fn pipeline(&self, cx: &Scoping) -> pgorm::pipeline::Pipeline {
        self.reach.pipeline(cx)
    }

    fn source(&self, cx: &Scoping) -> pgorm::pipeline::Source {
        self.reach.source(cx)
    }

    fn select(&self, cx: &Scoping) -> Select {
        Arc::new(SelectAdapter::<E> {
            query: self.reach.find(cx),
            info: self.info.clone(),
        })
    }

    fn active(&self) -> Active {
        Arc::new(ActiveAdapter::<E> {
            value: E::ActiveModel::new(),
            info: self.info.clone(),
        })
    }

    fn relation(&self, name: &str) -> Result<RelationDef, Failure> {
        E::Relation::iter()
            .find(|relation| format!("{relation:?}") == name)
            .map(|relation| relation.def())
            .ok_or_else(|| {
                Failure::Construction(format!("{} declares no relation {name}", self.info.name))
            })
    }

    fn column(&self, name: &str) -> Result<SimpleExpr, Failure> {
        Ok(Expr::col(column::<E>(&self.info, name)?.as_column_ref()).into())
    }

    fn compare(
        &self,
        name: &str,
        comparison: Comparison,
        value: Value,
    ) -> Result<SimpleExpr, Failure> {
        let column = column::<E>(&self.info, name)?;
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

    // [spec:pgorm:req:bindings.entity-reads+5]
    fn quantified(
        &self,
        name: &str,
        comparison: Comparison,
        all: bool,
        array: Value,
    ) -> Result<SimpleExpr, Failure> {
        let column = column::<E>(&self.info, name)?;
        Ok(match all {
            false => column.compare_any(comparison.operator(), array),
            true => column.compare_all(comparison.operator(), array),
        })
    }

    // [spec:pgorm:req:bindings.column-operators+4]
    fn between(
        &self,
        name: &str,
        between: Between,
        lower: Value,
        upper: Value,
    ) -> Result<SimpleExpr, Failure> {
        let column = column::<E>(&self.info, name)?;
        Ok(match (between.negated, between.symmetric) {
            (false, false) => column.between(lower, upper),
            (true, false) => column.not_between(lower, upper),
            (false, true) => column.between_symmetric(lower, upper),
            (true, true) => column.not_between_symmetric(lower, upper),
        })
    }

    // [spec:pgorm:req:bindings.entity-reads+5]
    fn membership(
        &self,
        name: &str,
        negated: bool,
        members: Members,
    ) -> Result<SimpleExpr, Failure> {
        let column = column::<E>(&self.info, name)?;
        Ok(match (members, negated) {
            (Members::Values(values), false) => column.is_in(values),
            (Members::Values(values), true) => column.is_not_in(values),
            (Members::Subquery(select), false) => column.in_subquery(*select),
            (Members::Subquery(select), true) => column.not_in_subquery(*select),
        })
    }

    // [spec:pgorm:req:bindings.pipeline-sources+3]
    fn pipeline_column(&self, name: &str) -> Result<Arc<dyn PipelineColumn>, Failure> {
        Ok(Arc::new(RegisteredColumn::<E>(column::<E>(
            &self.info, name,
        )?)))
    }

    fn versions<'a>(
        &'a self,
        db: Database<'a>,
        write: VersionWrite,
        cx: &'a Scoping,
    ) -> BoxFuture<'a, Result<Vec<Versions>, Error>> {
        Box::pin(self.write_versions(db, write, cx))
    }

    fn insert<'a>(
        &'a self,
        db: Database<'a>,
        insertion: Insertion,
        cx: &'a Scoping,
    ) -> BoxFuture<'a, Result<Insertions, Error>> {
        Box::pin(self.write_insert(db, insertion, cx))
    }

    // [spec:pgorm:req:bindings.disjoint+2]
    fn decode(&self, row: &pgorm::QueryResult, prefix: &str) -> Result<Model, Error> {
        E::Model::from_query_result(row, prefix).map(|value| model_of::<E>(value, &self.info))
    }
}

impl<E> EntityAdapter<E>
where
    E: Entity + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn model(&self, value: E::Model) -> Model {
        model_of::<E>(value, &self.info)
    }

    fn change(&self, change: RustChange<E::Model>) -> Versions {
        Versions {
            old: Some(self.model(change.old)),
            new: self.model(change.new),
        }
    }

    /// A row an upsert wrote, as its versions.
    pub fn upserted(&self, upserted: Upserted<E::Model>) -> Versions {
        match upserted {
            Upserted::Inserted(model) => Versions {
                old: None,
                new: self.model(model),
            },
            Upserted::Updated(change) => self.change(change),
        }
    }

    // [spec:pgorm:req:bindings.entity-versions+3]
    async fn write_versions(
        &self,
        db: Database<'_>,
        write: VersionWrite,
        cx: &Scoping,
    ) -> Result<Vec<Versions>, Error> {
        match write {
            VersionWrite::Change(active) => {
                let change = self
                    .reach
                    .update_one(cx, concrete::<E>(&active)?)?
                    .exec_returning_change(&db)
                    .await?;
                Ok(vec![self.change(change)])
            }
            VersionWrite::Changes(assignments, filter) => {
                let mut update = self.reach.update_many(cx);
                for (name, assignment) in assignments {
                    let column = column::<E>(&self.info, &name)
                        .map_err(|failure| Error::Custom(failure.message().to_owned()))?;
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
    // [spec:pgorm:req:bindings.entity-try-inserts]
    async fn write_insert(
        &self,
        db: Database<'_>,
        insertion: Insertion,
        cx: &Scoping,
    ) -> Result<Insertions, Error> {
        crate::insert::<E, _, _, _>(
            |model| self.reach.insert_one(cx, model),
            |models| self.reach.insert_many(cx, models),
            &db,
            insertion.try_map(concrete::<E>)?,
            |model| self.model(model),
            |row| self.upserted(row),
        )
        .await
    }
}

/// A registered entity's query, the real `Select<E>`.
#[derive(Debug)]
pub(crate) struct SelectAdapter<E: Entity> {
    pub(crate) query: pgorm::Select<E>,
    pub(crate) info: Arc<EntityInfo>,
}

// [spec:pgorm:req:bindings.entity-reads+5]
impl<E> SelectBackend for SelectAdapter<E>
where
    E: Entity + Send + Sync + 'static,
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
            Change::Join(kind, entity, relation, cx) => match (entity.relation(&relation), cx) {
                (Ok(def), Some(cx)) => query.join_for(&*cx, kind, def),
                (Ok(def), None) => query.join(kind, def),
                (Err(_), _) => query,
            },
        };
        Arc::new(Self {
            query,
            info: self.info.clone(),
        })
    }

    fn compile(&self, read: Read) -> (String, Values) {
        let mut query = self.query.clone();
        if read != Read::All {
            Statement::query(&mut query).narrow_limit_to_one();
        }
        query.build()
    }

    fn run<'a>(&'a self, db: Database<'a>, read: Read) -> BoxFuture<'a, Result<Vec<Model>, Error>> {
        Box::pin(async move {
            let query = self.query.clone();
            let models = match read {
                Read::All => query.all(&db).await?,
                Read::One => vec![query.one(&db).await?],
                Read::Optional => query.one_opt(&db).await?.into_iter().collect(),
            };
            Ok(self.models(models))
        })
    }

    fn projected(&self, columns: &[String], read: Read) -> Result<SelectStatement, Failure> {
        let columns = columns
            .iter()
            .map(|name| column::<E>(&self.info, name))
            .collect::<Result<Vec<_>, _>>()?;
        let mut query = self.query.clone().select_only().columns(columns);
        if read != Read::All {
            Statement::query(&mut query).narrow_limit_to_one();
        }
        Ok(Statement::into_query(query))
    }

    fn statement(&self) -> SelectStatement {
        Statement::as_query(&self.query).clone()
    }

    fn pages(&self) -> Arc<Pages> {
        reads::pages(self)
    }

    fn cursor(&self, columns: &[String]) -> Result<Arc<Keyset>, Failure> {
        reads::cursor(self, columns)
    }

    fn decode(&self) -> Decode {
        let info = self.info.clone();
        Arc::new(move |row| {
            let value = SelectModel::<E::Model>::from_raw_query_result(row)?;
            Ok(vec![Some(model_of::<E>(value, &info))])
        })
    }

    // [spec:pgorm:req:bindings.raw-models]
    fn run_raw<'a>(
        &'a self,
        db: Database<'a>,
        sql: String,
        values: Values,
        read: Read,
    ) -> BoxFuture<'a, Result<Vec<Model>, Error>> {
        Box::pin(async move {
            let query = self.query.clone().from_raw_sql(&sql, values);
            let models = match read {
                Read::All => query.all(&db).await?,
                Read::One => vec![query.one(&db).await?],
                Read::Optional => query.one_opt(&db).await?.into_iter().collect(),
            };
            Ok(self.models(models))
        })
    }
}

impl<E> SelectAdapter<E>
where
    E: Entity + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn models(&self, models: Vec<E::Model>) -> Vec<Model> {
        models
            .into_iter()
            .map(|value| model_of::<E>(value, &self.info))
            .collect()
    }
}
