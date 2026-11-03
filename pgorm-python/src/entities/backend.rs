use crate::execution::Database;
use std::{fmt::Debug, sync::Arc};

use futures_util::future::BoxFuture;
use pgorm::pgorm_query::{
    Condition, NullOrdering, OnConflict, Order, SelectStatement, SimpleExpr, Value, Values,
};
use pgorm::{ActiveValue, Error};

use super::metadata::EntityInfo;

pub(crate) type Model = Arc<dyn ModelBackend>;
pub(crate) type Active = Arc<dyn ActiveBackend>;
pub(crate) type Select = Arc<dyn SelectBackend>;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Comparison {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

/// What a column's membership is tested against: values of the column's
/// declared kind, or a subquery's rows.
#[derive(Clone, Debug)]
pub(crate) enum Members {
    Values(Vec<Value>),
    Subquery(Box<SelectStatement>),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Terminal {
    All,
    One,
    Optional,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Write {
    Insert,
    Update,
    Delete,
}

#[derive(Clone, Debug)]
pub(crate) enum Change {
    Filter(Condition),
    Order(SimpleExpr, Order, Option<NullOrdering>),
    Limit(Option<u64>),
    Offset(Option<u64>),
}

pub(crate) trait EntityBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    fn schema(&self) -> Result<crate::schema::PyEntitySchema, Error>;
    fn select(&self) -> Select;
    fn active(&self) -> Active;
    fn expression(&self, column: &str) -> Result<SimpleExpr, Error>;
    /// The column as a pipeline reads it, compared as the entity types it.
    fn pipeline_column(&self, column: &str) -> Result<Arc<dyn PipelineColumn>, Error>;
    fn compare(
        &self,
        column: &str,
        comparison: Comparison,
        value: Value,
    ) -> Result<SimpleExpr, Error>;
    fn membership(
        &self,
        column: &str,
        negated: bool,
        members: Members,
    ) -> Result<SimpleExpr, Error>;
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

/// What an `UpdateMany` sets a column to: a value, written through the
/// column's `save_as`, or an expression, written as it is.
#[derive(Clone, Debug)]
pub(crate) enum Assignment {
    Value(Value),
    Expr(SimpleExpr),
}

/// A write whose terminal reads the written rows' two versions.
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

/// A written row's versions: `old` absent for a row an upsert inserted.
pub(crate) struct Versions {
    pub(crate) old: Option<Model>,
    pub(crate) new: Model,
}

/// A registered entity's column, written under whatever name a pipeline reads
/// its relation by: the entity's own `Named::col`, so a column whose domain is
/// over an enum compares as the enum.
pub(crate) trait PipelineColumn: Debug + Send + Sync {
    fn lower<'brand>(&self, qualifier: &str) -> pgorm::pipeline::Expr<'brand>;
}

pub(crate) trait SelectBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    fn change(&self, change: Change) -> Select;
    fn compile(&self, terminal: Terminal) -> (String, Values);
    fn run<'a>(
        &'a self,
        db: Database<'a>,
        terminal: Terminal,
    ) -> BoxFuture<'a, Result<Vec<Model>, Error>>;
}

pub(crate) trait ModelBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    fn get(&self, column: &str) -> Result<Value, Error>;
    fn set(&self, column: &str, value: Value) -> Result<Model, Error>;
    fn active(&self) -> Active;
}

pub(crate) enum Written {
    Model(Model),
    Count(u64),
}

pub(crate) trait ActiveBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    fn get(&self, column: &str) -> Result<ActiveValue<Value>, Error>;
    fn set(&self, column: &str, value: Value) -> Result<Active, Error>;
    fn not_set(&self, column: &str) -> Result<Active, Error>;
    fn reset(&self, column: &str) -> Result<Active, Error>;
    fn run<'a>(&'a self, db: Database<'a>, write: Write) -> BoxFuture<'a, Result<Written, Error>>;
    /// The concrete adapter, so an entity's batch write can take its own
    /// models back out of type-erased handles.
    fn as_any(&self) -> &dyn std::any::Any;
}
