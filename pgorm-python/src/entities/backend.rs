use crate::execution::Database;
use std::{fmt::Debug, sync::Arc};

use futures_util::future::BoxFuture;
use pgorm::pgorm_query::{
    BinOper, Condition, JoinType, NullOrdering, OnConflict, Order, SelectStatement, SimpleExpr,
    Value, Values,
};
use pgorm::{ActiveValue, Error, RelationDef};

use super::{
    metadata::EntityInfo,
    reads::{Decode, Keyset, Pages},
};

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
    /// The operator the comparison is, for an operand compared as written
    /// and for a quantified comparison.
    pub(crate) fn operator(self) -> BinOper {
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

    /// Whether PostgreSQL reads `ANY` or `ALL` after the operator: every one
    /// but the distinctness tests.
    pub(crate) fn quantifiable(self) -> bool {
        !matches!(self, Self::IsDistinctFrom | Self::IsNotDistinctFrom)
    }
}

/// Which of `[NOT] BETWEEN [SYMMETRIC]` a column is tested by.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Between {
    pub(crate) negated: bool,
    pub(crate) symmetric: bool,
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
    /// `QuerySelect::join` of the relation the registered entity names so.
    Join(JoinType, Arc<dyn EntityBackend>, String),
}

pub(crate) trait EntityBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    fn schema(&self) -> Result<crate::schema::PyEntitySchema, Error>;
    fn select(&self) -> Select;
    fn active(&self) -> Active;
    /// The `RelationDef` of the `Relation` variant whose name is `name`.
    fn relation(&self, name: &str) -> Result<RelationDef, Error>;
    fn expression(&self, column: &str) -> Result<SimpleExpr, Error>;
    /// The column as a pipeline reads it, compared as the entity types it.
    fn pipeline_column(&self, column: &str) -> Result<Arc<dyn PipelineColumn>, Error>;
    fn compare(
        &self,
        column: &str,
        comparison: Comparison,
        value: Value,
    ) -> Result<SimpleExpr, Error>;
    fn between(
        &self,
        column: &str,
        between: Between,
        lower: Value,
        upper: Value,
    ) -> Result<SimpleExpr, Error>;
    /// `column <comparison> ANY (array)`, or `ALL` with `all`, through the
    /// column's `ColumnTrait::compare_any` / `compare_all`.
    fn quantified(
        &self,
        column: &str,
        comparison: Comparison,
        all: bool,
        array: Value,
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
    /// `Insert::one` of one ActiveModel or `Insert::many` of a batch, with
    /// the conflict clause if given, through the terminal `insertion` names.
    fn insert<'a>(
        &'a self,
        db: Database<'a>,
        insertion: Insertion,
    ) -> BoxFuture<'a, Result<Insertions, Error>>;
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
}

/// An insert of a registration's ActiveModels: `Insert::one` of one when
/// `one`, `Insert::many` of a batch otherwise, its conflict clause, and the
/// terminal that runs it.
#[derive(Clone, Debug)]
pub(crate) struct Insertion {
    pub(crate) actives: Vec<Active>,
    pub(crate) conflict: Option<Box<OnConflict>>,
    pub(crate) one: bool,
    pub(crate) answer: InsertAnswer,
}

/// Which of Rust's insert terminals runs, each the row count's own: `exec`,
/// `exec_returning_pk(s)`, `exec_returning_model(s)` or
/// `exec_returning_upsert(s)`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum InsertAnswer {
    Count,
    Keys,
    Models,
    Upserts,
}

/// What an insert's terminal gave.
pub(crate) enum Insertions {
    Count(u64),
    /// Each written row's key, its values in the primary key's column order.
    Keys(Vec<Vec<Value>>),
    Models(Vec<Model>),
    Upserts(Vec<Versions>),
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
    /// The statement of `Select::select` over `columns`, `one` and `optional`
    /// narrowed to one row.
    fn projected(&self, columns: &[String], terminal: Terminal) -> Result<(String, Values), Error>;
    fn pages(&self) -> Arc<dyn Pages>;
    /// `Select::cursor_by` over `columns`, in order.
    fn cursor(&self, columns: &[String]) -> Result<Arc<dyn Keyset>, Error>;
    /// The decode `Select::stream` runs on each row.
    fn decode(&self) -> Decode;
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
    /// `ActiveModelTrait::set_if_not_equals` on a copy.
    fn set_if_not_equals(&self, column: &str, value: Value) -> Result<Active, Error>;
    fn is_changed(&self) -> bool;
    fn run<'a>(&'a self, db: Database<'a>, write: Write) -> BoxFuture<'a, Result<Written, Error>>;
    /// The concrete adapter, so an entity's batch write can take its own
    /// models back out of type-erased handles.
    fn as_any(&self) -> &dyn std::any::Any;
}
