//! Rust entities an application registers with a binding, behind type-erased
//! handles: each adapter holds the real `pgorm::Select<E>`, `E::Model` or
//! `E::ActiveModel`, monomorphized when the application's module is built,
//! and every operation calls the Rust API of the same meaning,
//! `Select::all`, `ModelTrait::set`, `ActiveModelTrait::insert` with its
//! `ActiveModelBehavior` hooks, `UpdateOne::exec_returning_change`, so the
//! language reaches the entity's own semantics rather than a
//! reimplementation of them.

mod adapter;
mod disjoint;
mod graph;
mod info;
mod one_of;
mod reads;
mod registry;
mod sources;

use std::{any::Any, fmt::Debug, sync::Arc};

use futures_util::future::BoxFuture;
use pgorm::pgorm_query::{
    BinOper, CommentStatement, Condition, IndexCreateStatement, JoinType, Name, NullOrdering,
    Order, SelectStatement, SimpleExpr, TableCreateStatement, Value, Values,
    extension::TypeCreateStatement,
};
use pgorm::{ActiveValue, Error, RelationDef};

pub use adapter::{EntityAdapter, model_of};
pub use disjoint::{
    DisjointCore, Entity, EntityRow, Judged, KindRow, Kinded, RowCodec, active_values,
    foreign_active,
};
pub use graph::{
    GraphBackend, GraphBindings, GraphChange, GraphInfo, GraphQuery, GraphQueryBackend, GraphSlots,
    Grouped, RegisteredSlot, Source,
};
pub use info::{ColumnInfo, EntityInfo, object, registration_name};
pub use one_of::{DescribedTarget, OneOfCore};
pub use reads::{Decode, KeyColumn, Keyset, Pages, Row};
pub use registry::Registry;
pub use sources::{
    Selected, SelectedBackend, SourceBindings, SourceTypes, SourcesBackend, SourcesInfo,
};

use crate::{Database, Failure};

/// A registered entity's model behind its type-erased handle.
pub type Model = Arc<dyn ModelBackend>;
/// A registered entity's ActiveModel behind its type-erased handle.
pub type Active = Arc<dyn ActiveBackend>;
/// A registered entity's query behind its type-erased handle.
pub type Select = Arc<dyn SelectBackend>;

/// A comparison through a column's `ColumnTrait` method, which writes the
/// value through the column's `save_as`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Comparison {
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
    pub fn operator(self) -> BinOper {
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
    pub fn quantifiable(self) -> bool {
        !matches!(self, Self::IsDistinctFrom | Self::IsNotDistinctFrom)
    }
}

/// Which of `[NOT] BETWEEN [SYMMETRIC]` a column is tested by, through its
/// `ColumnTrait` method of that meaning.
#[derive(Clone, Copy, Debug)]
#[allow(missing_docs)]
pub struct Between {
    pub negated: bool,
    pub symmetric: bool,
}

/// What a column's membership is tested against: values of the column's
/// declared kind, or a subquery's rows.
#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub enum Members {
    Values(Vec<Value>),
    Subquery(Box<SelectStatement>),
}

/// A read terminal of `Select<E>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Read {
    All,
    /// `Select::one`, which a missing row fails.
    One,
    Optional,
}

/// A write of an ActiveModel, through `ActiveModelTrait` and its hooks.
#[derive(Clone, Copy, Debug)]
#[allow(missing_docs)]
pub enum Write {
    Insert,
    Update,
    Delete,
}

/// What an ActiveModel's write gave: the model written, or a delete's count.
#[derive(Debug)]
#[allow(missing_docs)]
pub enum Written {
    Model(Model),
    Count(u64),
}

/// One step a query takes: each is the `Select<E>` method of its meaning.
#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub enum Change {
    Filter(Condition),
    /// `QueryOrder::order_by`, or `order_by_with_nulls` with a placement.
    Order(SimpleExpr, Order, Option<NullOrdering>),
    Limit(Option<u64>),
    Offset(Option<u64>),
    /// `QuerySelect::join` of the relation the registered entity names so.
    Join(JoinType, Arc<dyn EntityBackend>, String),
}

/// What `UpdateMany::col_expr` sets a column to: a value, written through
/// the column's `save_as`, or an expression, as written.
#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub enum Assignment {
    Value(Value),
    Expr(SimpleExpr),
}

/// A write whose terminal reads each written row's two versions.
#[derive(Clone, Debug)]
pub enum VersionWrite {
    /// `UpdateOne::exec_returning_change`.
    Change(Active),
    /// `UpdateMany::exec_returning_changes`.
    Changes(Vec<(String, Assignment)>, Condition),
}

/// An insert of a registration's ActiveModels, the statement and terminal
/// [`crate::insert`] runs.
pub type Insertion = crate::Insertion<Active>;

/// What an insert's terminal gave, as a `TryInsertResult` says: a plain
/// insert's always `Inserted`.
pub type Insertions = pgorm::TryInsertResult<crate::Inserted<Model, Versions>>;

/// A written row's two versions, `old` absent for a row an upsert inserted.
#[derive(Debug)]
#[allow(missing_docs)]
pub struct Versions {
    /// The row as it was, unless the write inserted it.
    pub old: Option<Model>,
    pub new: Model,
}

/// A registered entity's schema as pgorm's own entity schema path writes it:
/// its table, the enum types its columns name, its indexes and its comments.
#[derive(Debug)]
#[allow(missing_docs)]
pub struct EntitySchema {
    pub table: TableCreateStatement,
    pub enums: Vec<TypeCreateStatement>,
    pub indexes: Vec<IndexCreateStatement>,
    pub comments: Vec<CommentStatement>,
}

/// A registered entity's column, written under whatever name a pipeline reads
/// its relation by: the entity's own `Named::col`, so a column whose domain is
/// over an enum compares as the enum.
pub trait PipelineColumn: Debug + Send + Sync {
    /// The column, qualified by the name a pipeline reads its relation by.
    fn lower<'brand>(&self, qualifier: Name) -> pgorm::pipeline::Expr<'brand>;
}

/// A registered entity.
#[allow(missing_docs)]
pub trait EntityBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    /// The entity's schema, or the error `Schema::create_table_from_entity`
    /// refuses its table with.
    fn schema(&self) -> Result<EntitySchema, Error>;
    /// The entity as a pipeline source, by its own `IntoSource`.
    fn source(&self) -> pgorm::pipeline::Source;
    fn select(&self) -> Select;
    fn active(&self) -> Active;
    /// The `RelationDef` of the `Relation` variant whose name is `name`.
    fn relation(&self, name: &str) -> Result<RelationDef, Failure>;
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

/// A registered entity's query.
#[allow(missing_docs)]
pub trait SelectBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    fn change(&self, change: Change) -> Select;
    /// The statement, `one` and `optional` narrowed to one row.
    fn compile(&self, read: Read) -> (String, Values);
    fn run<'a>(&'a self, db: Database<'a>, read: Read) -> BoxFuture<'a, Result<Vec<Model>, Error>>;
    /// The statement of `select_only` and `columns` over `columns`, `one` and
    /// `optional` narrowed to one row.
    fn projected(&self, columns: &[String], read: Read) -> Result<SelectStatement, Failure>;
    fn statement(&self) -> SelectStatement;
    fn pages(&self) -> Arc<Pages>;
    /// `Select::cursor_by` over `columns`, in order.
    fn cursor(&self, columns: &[String]) -> Result<Arc<Keyset>, Failure>;
    /// The decode `Select::stream` runs on each row.
    fn decode(&self) -> Decode;
    /// `sql` and its values in place of the query, decoded into the entity's
    /// models through `Select::from_raw_sql`.
    fn run_raw<'a>(
        &'a self,
        db: Database<'a>,
        sql: String,
        values: Values,
        read: Read,
    ) -> BoxFuture<'a, Result<Vec<Model>, Error>>;
}

/// A registered entity's model.
#[allow(missing_docs)]
pub trait ModelBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    fn get(&self, column: &str) -> Result<Value, Failure>;
    /// `ModelTrait::set` on a copy.
    fn set(&self, column: &str, value: Value) -> Result<Model, Failure>;
    fn active(&self) -> Active;
}

/// A registered entity's ActiveModel.
#[allow(missing_docs)]
pub trait ActiveBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<EntityInfo>;
    fn get(&self, column: &str) -> Result<ActiveValue<Value>, Failure>;
    fn set(&self, column: &str, value: Value) -> Result<Active, Failure>;
    fn not_set(&self, column: &str) -> Result<Active, Failure>;
    fn reset(&self, column: &str) -> Result<Active, Failure>;
    /// `ActiveModelTrait::set_if_not_equals` on a copy.
    fn set_if_not_equals(&self, column: &str, value: Value) -> Result<Active, Failure>;
    fn is_changed(&self) -> bool;
    fn run<'a>(&'a self, db: Database<'a>, write: Write) -> BoxFuture<'a, Result<Written, Error>>;
    /// The concrete adapter, so an entity's batch write can take its own
    /// ActiveModels back out of type-erased handles.
    fn as_any(&self) -> &dyn Any;
}
