//! Query statements (select, insert, update, delete & merge).
//!
//! # Usage
//!
//! - Query Select, see [`SelectStatement`]
//! - Query Insert, see [`InsertStatement`]
//! - Query Update, see [`UpdateStatement`]
//! - Query Delete, see [`DeleteStatement`]
//! - Query Merge, see [`MergeStatement`]

use crate::{IntoFromItem, IntoNamedTable};

mod case;
mod condition;
mod delete;
mod frame;
mod grouping;
mod insert;
mod merge;
mod on_conflict;
mod ordered;
mod returning;
mod select;
mod select_expr;
mod table_function;
mod table_sample;
mod tie_limit;
mod traits;
mod update;
mod window;
mod window_clause;
mod with;

pub use case::*;
pub use condition::*;
pub use delete::*;
pub use frame::*;
pub use grouping::*;
pub use insert::*;
pub use merge::*;
pub use on_conflict::*;
pub use ordered::*;
pub use returning::*;
pub use select::*;
pub use select_expr::*;
pub use table_function::TableFunction;
pub use table_sample::{SampleMethod, TableSample};
pub use traits::*;
pub use update::*;
pub use window::*;
pub use window_clause::{ExtendedWindow, WindowDefinition};

pub(crate) use frame::FrameBound;
pub(crate) use grouping::GroupingKind;
pub(crate) use tie_limit::RowLimit;
pub(crate) use window_clause::NamedWindow;
pub use with::*;

/// Shorthand for constructing any table query
// [spec:pgorm:req:sql.ast+4]
#[derive(Debug, Clone)]
pub struct Query;

// [spec:pgorm:req:sql.ast+4]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SubQueryStatement {
    SelectStatement(SelectStatement),
    InsertStatement(InsertStatement),
    UpdateStatement(UpdateStatement),
    DeleteStatement(DeleteStatement),
    MergeStatement(Box<MergeStatement>),
}

/// The wrapped statement's own rendering, its values inlined as each
/// statement's `Display` writes them, so a dependent renders one without
/// matching an enum it cannot match exhaustively.
// [spec:pgorm:req:sql.ast+4]
impl std::fmt::Display for SubQueryStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        self.prepare_statement(&mut sql);
        f.write_str(&sql)
    }
}

impl Query {
    /// Construct table [`SelectStatement`]
    pub fn select() -> SelectStatement {
        SelectStatement::new()
    }

    /// Construct table [`InsertStatement`]
    pub fn insert() -> InsertStatement {
        InsertStatement::new()
    }

    /// Construct table [`UpdateStatement`]
    pub fn update() -> UpdateStatement {
        UpdateStatement::new()
    }

    /// Construct table [`DeleteStatement`]
    pub fn delete() -> DeleteStatement {
        DeleteStatement::new()
    }

    /// Begin a `MERGE` into `target` from `source`, pairing their rows where
    /// `on` holds. The result is a [`PendingMerge`], which becomes a
    /// [`MergeStatement`] when its first `WHEN` arm is added.
    ///
    /// The target is a table name, optionally schema-qualified and aliased,
    /// and the source is any relation a `FROM` clause takes: a table, a
    /// subquery, a values list, a function call or a validated fragment.
    // [spec:pgorm:req:sql.ast.merge+2]
    pub fn merge<T, S, C>(target: T, source: S, on: C) -> PendingMerge
    where
        T: IntoNamedTable,
        S: IntoFromItem,
        C: IntoCondition,
    {
        PendingMerge {
            target: target.into_named_table(),
            source: source.into_from_item(),
            on: on.into_condition(),
        }
    }

    /// Construct [`WithClause`] around its first common table expression
    pub fn with(cte: CommonTableExpression) -> WithClause {
        WithClause::new(cte)
    }

    /// Construct [`RecursiveWithClause`] around the one common table expression it may hold
    pub fn with_recursive(cte: CommonTableExpression) -> RecursiveWithClause {
        RecursiveWithClause::new(cte)
    }

    /// Construct [`Returning`]
    pub fn returning() -> Returning {
        Returning::new()
    }
}
