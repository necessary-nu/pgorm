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
pub enum Comparison {
    /// `=`, `ColumnTrait::eq`.
    Eq,
    /// `<>`, `ColumnTrait::ne`.
    Ne,
    /// `>`, `ColumnTrait::gt`.
    Gt,
    /// `>=`, `ColumnTrait::gte`.
    Ge,
    /// `<`, `ColumnTrait::lt`.
    Lt,
    /// `<=`, `ColumnTrait::lte`.
    Le,
    /// `IS DISTINCT FROM`: unequal, null comparing as a value.
    IsDistinctFrom,
    /// `IS NOT DISTINCT FROM`: equal, null comparing as a value.
    IsNotDistinctFrom,
    /// `@>`: the column's range or array contains the operand.
    Contains,
    /// `<@`: the operand contains the column's range or array.
    ContainedBy,
    /// `&&`: the two share an element or a point.
    Overlaps,
    /// `<<`: the column's range lies wholly left of the operand.
    StrictlyLeftOf,
    /// `>>`: the column's range lies wholly right of the operand.
    StrictlyRightOf,
    /// `&<`: the column's range reaches no further right than the operand.
    NotExtendRightOf,
    /// `&>`: the column's range reaches no further left than the operand.
    NotExtendLeftOf,
    /// `-|-`: the two ranges meet with no gap or overlap.
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
pub struct Between {
    /// `NOT BETWEEN`: the rows outside the bounds.
    pub negated: bool,
    /// `BETWEEN SYMMETRIC`: the bounds taken in either order.
    pub symmetric: bool,
}

/// What a column's membership is tested against: values of the column's
/// declared kind, or a subquery's rows.
#[derive(Clone, Debug)]
pub enum Members {
    /// A list of values, each bound through the column's `save_as`.
    Values(Vec<Value>),
    /// A subquery whose one column's rows are the members.
    Subquery(Box<SelectStatement>),
}

/// A read terminal of `Select<E>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// `Select::all`: every row.
    All,
    /// `Select::one`, which a missing row fails.
    One,
    /// `Select::one_opt`: the first row, or none.
    Optional,
}

/// A write of an ActiveModel, through `ActiveModelTrait` and its hooks.
#[derive(Clone, Copy, Debug)]
pub enum Write {
    /// `ActiveModelTrait::insert`.
    Insert,
    /// `ActiveModelTrait::update`.
    Update,
    /// `ActiveModelTrait::delete`.
    Delete,
}

/// What an ActiveModel's write gave: the model written, or a delete's count.
#[derive(Debug)]
pub enum Written {
    /// The model an insert or update wrote, as the database returned it.
    Model(Model),
    /// How many rows a delete removed.
    Count(u64),
}

/// One step a query takes: each is the `Select<E>` method of its meaning.
#[derive(Clone, Debug)]
pub enum Change {
    /// `QueryFilter::filter`: the rows the condition keeps.
    Filter(Condition),
    /// `QueryOrder::order_by`, or `order_by_with_nulls` with a placement.
    Order(SimpleExpr, Order, Option<NullOrdering>),
    /// `QuerySelect::limit`, or no limit with `None`.
    Limit(Option<u64>),
    /// `QuerySelect::offset`, or no offset with `None`.
    Offset(Option<u64>),
    /// `QuerySelect::join` of the relation the registered entity names so.
    Join(JoinType, Arc<dyn EntityBackend>, String),
}

/// What `UpdateMany::col_expr` sets a column to: a value, written through
/// the column's `save_as`, or an expression, as written.
#[derive(Clone, Debug)]
pub enum Assignment {
    /// A value, bound through the column's `save_as`.
    Value(Value),
    /// An expression, written as the program built it.
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
pub struct Versions {
    /// The row as it was, unless the write inserted it.
    pub old: Option<Model>,
    /// The row as the write left it.
    pub new: Model,
}

/// A registered entity's schema as pgorm's own entity schema path writes it:
/// its table, the enum types its columns name, its indexes and its comments.
#[derive(Debug)]
pub struct EntitySchema {
    /// `Schema::create_table_from_entity`'s table.
    pub table: TableCreateStatement,
    /// `Schema::create_enum_from_entity`'s types, one per enum a column
    /// names.
    pub enums: Vec<TypeCreateStatement>,
    /// `Schema::create_index_from_entity`'s indexes.
    pub indexes: Vec<IndexCreateStatement>,
    /// `Schema::create_comments_from_entity`'s comments on the table and its
    /// columns.
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
pub trait EntityBackend: Debug + Send + Sync {
    /// What the binding knows of the entity: its table, columns and key.
    fn info(&self) -> &Arc<EntityInfo>;
    /// The entity's schema, or the error `Schema::create_table_from_entity`
    /// refuses its table with.
    fn schema(&self) -> Result<EntitySchema, Error>;
    /// The entity as a pipeline source, by its own `IntoSource`.
    fn source(&self) -> pgorm::pipeline::Source;
    /// `EntityTrait::find_in` of the registered entity value.
    fn select(&self) -> Select;
    /// `ActiveModelBehavior::new`: an ActiveModel no column of which is set
    /// but what the entity's hooks set.
    fn active(&self) -> Active;
    /// The `RelationDef` of the `Relation` variant whose name is `name`.
    fn relation(&self, name: &str) -> Result<RelationDef, Failure>;
    /// The column as an expression, qualified by the entity's table.
    fn column(&self, column: &str) -> Result<SimpleExpr, Failure>;
    /// `column <comparison> value` through the column's `ColumnTrait`
    /// method, the value written through its `save_as`.
    fn compare(
        &self,
        column: &str,
        comparison: Comparison,
        value: Value,
    ) -> Result<SimpleExpr, Failure>;
    /// `column [NOT] BETWEEN [SYMMETRIC] lower AND upper` through
    /// `ColumnTrait::between` and its kin, each bound through `save_as`.
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
    /// `column [NOT] IN (members)` through `ColumnTrait::is_in` /
    /// `is_not_in` or `in_subquery` / `not_in_subquery`.
    fn membership(
        &self,
        column: &str,
        negated: bool,
        members: Members,
    ) -> Result<SimpleExpr, Failure>;
    /// The column as a pipeline reads it, compared as the entity types it.
    fn pipeline_column(&self, column: &str) -> Result<Arc<dyn PipelineColumn>, Failure>;
    /// The write `write` names, each written row read back as it was and
    /// as it became.
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
pub trait SelectBackend: Debug + Send + Sync {
    /// What the queried entity is.
    fn info(&self) -> &Arc<EntityInfo>;
    /// The query with `change` applied, this one left as it is.
    fn change(&self, change: Change) -> Select;
    /// The statement, `one` and `optional` narrowed to one row.
    fn compile(&self, read: Read) -> (String, Values);
    /// The read terminal `read` names, its rows decoded as the entity's
    /// models.
    fn run<'a>(&'a self, db: Database<'a>, read: Read) -> BoxFuture<'a, Result<Vec<Model>, Error>>;
    /// The statement of `select_only` and `columns` over `columns`, `one` and
    /// `optional` narrowed to one row.
    fn projected(&self, columns: &[String], read: Read) -> Result<SelectStatement, Failure>;
    /// The query's `SELECT`, as `QueryTrait::as_query` gives it.
    fn statement(&self) -> SelectStatement;
    /// The query's pages, through pgorm's erased paginator.
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
pub trait ModelBackend: Debug + Send + Sync {
    /// What the model's entity is.
    fn info(&self) -> &Arc<EntityInfo>;
    /// `ModelTrait::get` of the column.
    fn get(&self, column: &str) -> Result<Value, Failure>;
    /// `ModelTrait::set` on a copy.
    fn set(&self, column: &str, value: Value) -> Result<Model, Failure>;
    /// `ModelTrait::into_active`: every column set unchanged.
    fn active(&self) -> Active;
}

/// A registered entity's ActiveModel.
pub trait ActiveBackend: Debug + Send + Sync {
    /// What the ActiveModel's entity is.
    fn info(&self) -> &Arc<EntityInfo>;
    /// `ActiveModelTrait::get` of the column: set, unchanged or not set.
    fn get(&self, column: &str) -> Result<ActiveValue<Value>, Failure>;
    /// `ActiveModelTrait::set` on a copy.
    fn set(&self, column: &str, value: Value) -> Result<Active, Failure>;
    /// `ActiveModelTrait::not_set` on a copy.
    fn not_set(&self, column: &str) -> Result<Active, Failure>;
    /// `ActiveModelTrait::reset` on a copy: the column's value marked set.
    fn reset(&self, column: &str) -> Result<Active, Failure>;
    /// `ActiveModelTrait::set_if_not_equals` on a copy.
    fn set_if_not_equals(&self, column: &str, value: Value) -> Result<Active, Failure>;
    /// `ActiveModelTrait::is_changed`: whether any column is set.
    fn is_changed(&self) -> bool;
    /// The write `write` names, through the entity's `ActiveModelBehavior`
    /// hooks.
    fn run<'a>(&'a self, db: Database<'a>, write: Write) -> BoxFuture<'a, Result<Written, Error>>;
    /// The concrete adapter, so an entity's batch write can take its own
    /// ActiveModels back out of type-erased handles.
    fn as_any(&self) -> &dyn Any;
}
