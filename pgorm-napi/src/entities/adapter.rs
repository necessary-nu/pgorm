//! The registered entity behind type-erased handles: each adapter holds the
//! real `pgorm::Select<E>`, `E::Model` or `E::ActiveModel`, monomorphized
//! when the application's module is built, and every operation calls the
//! Rust API of the same meaning — `Select::all`, `ModelTrait::set`,
//! `ActiveModelTrait::insert` with its `ActiveModelBehavior` hooks,
//! `UpdateOne::exec_returning_change` — so JavaScript reaches the entity's
//! own semantics rather than a reimplementation of them.

use std::{any::Any, fmt::Debug, marker::PhantomData, sync::Arc};

use futures_util::future::BoxFuture;
use pgorm::pgorm_query::{
    BinOper, CommentStatement, Condition, Expr, IndexCreateStatement, Name, NullOrdering,
    OnConflict, Order, SelectStatement, SimpleExpr, TableCreateStatement, Value, Values,
    extension::TypeCreateStatement,
};
use pgorm::{
    ActiveModelBehavior, ActiveModelTrait, ActiveValue, Change as RustChange, ColumnTrait,
    EntityTrait, Error, Insert, IntoActiveModel, Iterable, ModelTrait, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait, StaticName, Update, Upserted,
};

use super::info::EntityInfo;
use crate::{connect::job::Database, errors::Failure};

pub(crate) type Model = Arc<dyn ModelBackend>;
pub(crate) type Active = Arc<dyn ActiveBackend>;
pub(crate) type Select = Arc<dyn SelectBackend>;

/// A comparison through a column's `ColumnTrait` method, which writes the
/// value through the column's `save_as`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Comparison {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    IsDistinctFrom,
    IsNotDistinctFrom,
    Contains,
    ContainedBy,
    Overlaps,
    StrictlyLeftOf,
    StrictlyRightOf,
    NotExtendRightOf,
    NotExtendLeftOf,
    AdjacentTo,
}

impl Comparison {
    /// The operator a quantified comparison applies.
    fn operator(self) -> BinOper {
        match self {
            Self::Eq => BinOper::Equal,
            Self::Ne => BinOper::NotEqual,
            Self::Gt => BinOper::GreaterThan,
            Self::Ge => BinOper::GreaterThanOrEqual,
            Self::Lt => BinOper::SmallerThan,
            Self::Le => BinOper::SmallerThanOrEqual,
            Self::IsDistinctFrom => BinOper::IsDistinctFrom,
            Self::IsNotDistinctFrom => BinOper::IsNotDistinctFrom,
            Self::Contains => BinOper::Contains,
            Self::ContainedBy => BinOper::Contained,
            Self::Overlaps => BinOper::Overlap,
            Self::StrictlyLeftOf => BinOper::LShift,
            Self::StrictlyRightOf => BinOper::RShift,
            Self::NotExtendRightOf => BinOper::NotExtendRightOf,
            Self::NotExtendLeftOf => BinOper::NotExtendLeftOf,
            Self::AdjacentTo => BinOper::AdjacentTo,
        }
    }
}

/// Which of `[NOT] BETWEEN [SYMMETRIC]` a column is tested by, through its
/// `ColumnTrait` method of that meaning.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Between {
    pub(crate) negated: bool,
    pub(crate) symmetric: bool,
}

/// A read terminal of `Select<E>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Read {
    All,
    One,
    OneOpt,
}

/// A write of an ActiveModel, through `ActiveModelTrait` and its hooks.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Write {
    Insert,
    Update,
    Delete,
}

/// One step a query takes: each is the `Select<E>` method of its meaning.
#[derive(Clone, Debug)]
pub(crate) enum Change {
    Filter(Condition),
    Order(SimpleExpr, Order, Option<NullOrdering>),
    Limit(Option<u64>),
    Offset(Option<u64>),
}

pub(crate) enum Written {
    Model(Model),
    Count(u64),
}

/// What a column's membership is tested against: values of the column's
/// declared kind, or a subquery's rows.
#[derive(Clone, Debug)]
pub(crate) enum Members {
    Values(Vec<Value>),
    Subquery(Box<SelectStatement>),
}

/// A registered entity's column, written under whatever name a pipeline reads
/// its relation by: the entity's own `Named::col`, so a column whose domain is
/// over an enum compares as the enum.
pub(crate) trait PipelineColumn: Debug + Send + Sync {
    fn lower<'brand>(&self, qualifier: Name) -> pgorm::pipeline::Expr<'brand>;
}

/// One of `E`'s columns, resolved when the pipeline expression naming it was
/// built, so lowering it cannot miss.
struct RegisteredColumn<E: EntityTrait>(E::Column);

impl<E: EntityTrait> Debug for RegisteredColumn<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("RegisteredColumn").field(&self.0).finish()
    }
}

// [spec:pgorm:req:bindings.pipeline-sources+3]
impl<E: EntityTrait + Send + Sync> PipelineColumn for RegisteredColumn<E> {
    fn lower<'brand>(&self, qualifier: Name) -> pgorm::pipeline::Expr<'brand> {
        pgorm::pipeline::named_runtime(E::default(), qualifier).col(self.0)
    }
}

/// What `UpdateMany::col_expr` sets a column to: a value, written through
/// the column's `save_as`, or an expression, as written.
#[derive(Clone, Debug)]
pub(crate) enum Assignment {
    Value(Value),
    Expr(SimpleExpr),
}

/// A write whose terminal reads each written row's two versions.
#[derive(Clone, Debug)]
pub(crate) enum VersionWrite {
    /// `UpdateOne::exec_returning_change`.
    Change(Active),
    /// `UpdateMany::exec_returning_changes`.
    Changes(Vec<(String, Assignment)>, Condition),
    /// `Insert::exec_returning_upsert` for one model, `exec_returning_upserts`
    /// for a batch.
    Upserts {
        actives: Vec<Active>,
        conflict: Option<Box<OnConflict>>,
        one: bool,
    },
}

/// A written row's two versions, `old` absent for a row an upsert inserted.
pub(crate) struct Versions {
    pub(crate) old: Option<Model>,
    pub(crate) new: Model,
}

/// A registered entity's schema as pgorm's own entity schema path writes it:
/// its table, the enum types its columns name, its indexes and its comments.
#[derive(Debug)]
pub(crate) struct EntitySchema {
    pub(crate) table: TableCreateStatement,
    pub(crate) enums: Vec<TypeCreateStatement>,
    pub(crate) indexes: Vec<IndexCreateStatement>,
    pub(crate) comments: Vec<CommentStatement>,
}

pub(crate) trait EntityBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    /// The entity's schema, or the error `Schema::create_table_from_entity`
    /// refuses its table with.
    fn schema(&self) -> Result<EntitySchema, Error>;
    /// The entity as a pipeline source, by its own `IntoSource`.
    fn source(&self) -> pgorm::pipeline::Source;
    fn select(&self) -> Select;
    fn active(&self) -> Active;
    fn column(&self, column: &str) -> Result<SimpleExpr, Failure>;
    fn compare(
        &self,
        column: &str,
        comparison: Comparison,
        value: Value,
    ) -> Result<SimpleExpr, Failure>;
    fn between(
        &self,
        column: &str,
        between: Between,
        lower: Value,
        upper: Value,
    ) -> Result<SimpleExpr, Failure>;
    /// `column <comparison> ANY (array)`, or `ALL` with `all`, through the
    /// column's `ColumnTrait::compare_any` / `compare_all`.
    fn quantified(
        &self,
        column: &str,
        comparison: Comparison,
        all: bool,
        array: Value,
    ) -> Result<SimpleExpr, Failure>;
    fn membership(
        &self,
        column: &str,
        negated: bool,
        members: Members,
    ) -> Result<SimpleExpr, Failure>;
    /// The column as a pipeline reads it, compared as the entity types it.
    fn pipeline_column(&self, column: &str) -> Result<Arc<dyn PipelineColumn>, Failure>;
    fn versions<'a>(
        &'a self,
        db: Database<'a>,
        write: VersionWrite,
    ) -> BoxFuture<'a, Result<Vec<Versions>, Error>>;
    /// The model `row` holds under `prefix`, as the entity's own
    /// `FromQueryResult` reads it: one source of a read that decodes several,
    /// a disjoint one's.
    fn decode(&self, row: &pgorm::QueryResult, prefix: &str) -> Result<Model, Error>;
}

pub(crate) trait SelectBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    fn change(&self, change: Change) -> Select;
    fn compile(&self, read: Read) -> (String, Values);
    fn run<'a>(&'a self, db: Database<'a>, read: Read) -> BoxFuture<'a, Result<Vec<Model>, Error>>;
}

pub(crate) trait ModelBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    fn get(&self, column: &str) -> Result<Value, Failure>;
    fn set(&self, column: &str, value: Value) -> Result<Model, Failure>;
    fn active(&self) -> Active;
}

pub(crate) trait ActiveBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    fn get(&self, column: &str) -> Result<ActiveValue<Value>, Failure>;
    fn set(&self, column: &str, value: Value) -> Result<Active, Failure>;
    fn not_set(&self, column: &str) -> Result<Active, Failure>;
    fn reset(&self, column: &str) -> Result<Active, Failure>;
    fn run<'a>(&'a self, db: Database<'a>, write: Write) -> BoxFuture<'a, Result<Written, Error>>;
    /// The concrete adapter, so an entity's batch write can take its own
    /// ActiveModels back out of type-erased handles.
    fn as_any(&self) -> &dyn Any;
}

/// The entity's column of SQL name `name`.
fn column<E: EntityTrait>(info: &EntityInfo, name: &str) -> Result<E::Column, Failure> {
    <E::Column as Iterable>::iter()
        .find(|column| column.as_str() == name)
        .ok_or_else(|| Failure::Construction(format!("{} has no column {name:?}", info.name)))
}

#[derive(Debug)]
pub(crate) struct EntityAdapter<E> {
    pub(crate) info: Arc<EntityInfo>,
    pub(crate) entity: PhantomData<E>,
}

// [spec:pgorm:req:bindings.entities+5]
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
    fn schema(&self) -> Result<EntitySchema, Error> {
        let schema = pgorm::Schema::new();
        Ok(EntitySchema {
            table: schema.create_table_from_entity(E::default())?,
            enums: schema.create_enum_from_entity(E::default()),
            indexes: schema.create_index_from_entity(E::default()),
            comments: schema.create_comments_from_entity(E::default()),
        })
    }

    fn source(&self) -> pgorm::pipeline::Source {
        pgorm::pipeline::IntoSource::into_source(E::default())
    }

    fn select(&self) -> Select {
        Arc::new(SelectAdapter::<E> {
            query: E::find(),
            info: self.info.clone(),
        })
    }

    fn active(&self) -> Active {
        Arc::new(ActiveAdapter::<E> {
            value: E::ActiveModel::new(),
            info: self.info.clone(),
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

    // [spec:pgorm:req:bindings.entity-reads+3]
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

    // [spec:pgorm:req:bindings.column-operators+3]
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

    // [spec:pgorm:req:bindings.entity-reads+3]
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

    fn pipeline_column(&self, name: &str) -> Result<Arc<dyn PipelineColumn>, Failure> {
        Ok(Arc::new(RegisteredColumn::<E>(column::<E>(
            &self.info, name,
        )?)))
    }

    fn versions<'a>(
        &'a self,
        db: Database<'a>,
        write: VersionWrite,
    ) -> BoxFuture<'a, Result<Vec<Versions>, Error>> {
        Box::pin(self.write_versions(db, write))
    }

    // [spec:pgorm:req:bindings.disjoint+1]
    fn decode(&self, row: &pgorm::QueryResult, prefix: &str) -> Result<Model, Error> {
        <E::Model as pgorm::FromQueryResult>::from_query_result(row, prefix)
            .map(|value| model_of::<E>(value, &self.info))
    }
}

/// The compiled ActiveModel a type-erased handle holds, refused if another
/// registration made it.
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
        .ok_or_else(|| Error::Custom("the ActiveModel belongs to another registration".to_owned()))
}

impl<E> EntityAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn change(&self, change: RustChange<E::Model>) -> Versions {
        Versions {
            old: Some(model_of::<E>(change.old, &self.info)),
            new: model_of::<E>(change.new, &self.info),
        }
    }

    fn upserted(&self, upserted: Upserted<E::Model>) -> Versions {
        match upserted {
            Upserted::Inserted(model) => Versions {
                old: None,
                new: model_of::<E>(model, &self.info),
            },
            Upserted::Updated(change) => self.change(change),
        }
    }

    // [spec:pgorm:req:bindings.entity-versions+2]
    async fn write_versions(
        &self,
        db: Database<'_>,
        write: VersionWrite,
    ) -> Result<Vec<Versions>, Error> {
        match write {
            VersionWrite::Change(active) => {
                let change = Update::one(concrete::<E>(&active)?)?
                    .exec_returning_change(&db)
                    .await?;
                Ok(vec![self.change(change)])
            }
            VersionWrite::Changes(assignments, filter) => {
                let mut update = Update::many(E::default());
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
            VersionWrite::Upserts {
                actives,
                conflict,
                one,
            } => {
                let models = actives
                    .iter()
                    .map(concrete::<E>)
                    .collect::<Result<Vec<_>, _>>()?;
                let upserted: Vec<Upserted<E::Model>> = if one {
                    let model = models.into_iter().next().ok_or_else(|| {
                        Error::Custom("an upsert of one row needs one ActiveModel".to_owned())
                    })?;
                    let mut insert = Insert::one(model);
                    if let Some(conflict) = conflict {
                        insert = insert.on_conflict(*conflict);
                    }
                    insert
                        .exec_returning_upsert(&db)
                        .await?
                        .into_iter()
                        .collect()
                } else {
                    let mut insert = Insert::many(models);
                    if let Some(conflict) = conflict {
                        insert = insert.on_conflict(*conflict);
                    }
                    insert.exec_returning_upserts(&db).await?
                };
                Ok(upserted.into_iter().map(|row| self.upserted(row)).collect())
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct SelectAdapter<E: EntityTrait> {
    pub(crate) query: pgorm::Select<E>,
    pub(crate) info: Arc<EntityInfo>,
}

// [spec:pgorm:req:bindings.entity-reads+3]
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

    fn compile(&self, read: Read) -> (String, Values) {
        let mut query = self.query.clone();
        if read != Read::All {
            QueryTrait::query(&mut query).narrow_limit_to_one();
        }
        query.build()
    }

    fn run<'a>(&'a self, db: Database<'a>, read: Read) -> BoxFuture<'a, Result<Vec<Model>, Error>> {
        Box::pin(async move {
            let query = self.query.clone();
            let models = match read {
                Read::All => query.all(&db).await?,
                Read::One => vec![query.one(&db).await?],
                Read::OneOpt => query.one_opt(&db).await?.into_iter().collect(),
            };
            Ok(models
                .into_iter()
                .map(|value| model_of::<E>(value, &self.info))
                .collect())
        })
    }
}

#[derive(Debug)]
pub(crate) struct ModelAdapter<E: EntityTrait> {
    pub(crate) value: E::Model,
    pub(crate) info: Arc<EntityInfo>,
}

/// `value`, a model the registration `info` describes, behind its
/// type-erased handle.
pub(crate) fn model_of<E>(value: E::Model, info: &Arc<EntityInfo>) -> Model
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    Arc::new(ModelAdapter::<E> {
        value,
        info: Arc::clone(info),
    })
}

// [spec:pgorm:req:bindings.entity-writes]
impl<E> ModelBackend for ModelAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn info(&self) -> &Arc<EntityInfo> {
        &self.info
    }

    fn get(&self, name: &str) -> Result<Value, Failure> {
        Ok(self.value.get(column::<E>(&self.info, name)?))
    }

    fn set(&self, name: &str, value: Value) -> Result<Model, Failure> {
        let mut model = self.value.clone();
        model
            .set(column::<E>(&self.info, name)?, value)
            .map_err(|error| Failure::Construction(error.to_string()))?;
        Ok(model_of::<E>(model, &self.info))
    }

    fn active(&self) -> Active {
        Arc::new(ActiveAdapter::<E> {
            value: self.value.clone().into_active_model(),
            info: self.info.clone(),
        })
    }
}

#[derive(Debug)]
pub(crate) struct ActiveAdapter<E: EntityTrait> {
    pub(crate) value: E::ActiveModel,
    pub(crate) info: Arc<EntityInfo>,
}

impl<E> ActiveAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    /// A copy of the ActiveModel with `change` made to its column `name`.
    fn changed(
        &self,
        name: &str,
        change: impl FnOnce(&mut E::ActiveModel, E::Column) -> Result<(), Error>,
    ) -> Result<Active, Failure> {
        let column = column::<E>(&self.info, name)?;
        let mut value = self.value.clone();
        change(&mut value, column).map_err(|error| Failure::Construction(error.to_string()))?;
        Ok(Arc::new(Self {
            value,
            info: self.info.clone(),
        }))
    }
}

// [spec:pgorm:req:bindings.entity-writes]
impl<E> ActiveBackend for ActiveAdapter<E>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    fn info(&self) -> &Arc<EntityInfo> {
        &self.info
    }

    fn get(&self, name: &str) -> Result<ActiveValue<Value>, Failure> {
        self.value
            .get(column::<E>(&self.info, name)?)
            .map_err(|error| Failure::Construction(error.to_string()))
    }

    fn set(&self, name: &str, value: Value) -> Result<Active, Failure> {
        self.changed(name, |active, column| active.set(column, value))
    }

    fn not_set(&self, name: &str) -> Result<Active, Failure> {
        self.changed(name, |active, column| {
            active.not_set(column);
            Ok(())
        })
    }

    fn reset(&self, name: &str) -> Result<Active, Failure> {
        self.changed(name, |active, column| {
            active.reset(column);
            Ok(())
        })
    }

    fn run<'a>(&'a self, db: Database<'a>, write: Write) -> BoxFuture<'a, Result<Written, Error>> {
        Box::pin(async move {
            let model = match write {
                Write::Insert => self.value.clone().insert(&db).await?,
                Write::Update => self.value.clone().update(&db).await?,
                Write::Delete => return self.value.clone().delete(&db).await.map(Written::Count),
            };
            Ok(Written::Model(model_of::<E>(model, &self.info)))
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
