//! Table definition & alternations statements.
//!
//! # Usage
//!
//! - Table Create, see [`TableCreateStatement`]
//! - Table Create As, see [`TableCreateAsStatement`]
//! - Table Alter, see [`TableAlterStatement`]
//! - Table Drop, see [`TableDropStatement`]
//! - Table Rename, see [`TableRenameStatement`]
//! - Column Rename, see [`ColumnRenameStatement`]
//! - Constraint Rename, see [`ConstraintRenameStatement`]
//! - Table Truncate, see [`TableTruncateStatement`]

use crate::SelectStatement;
use crate::error::Result;
use crate::types::{IntoName, IntoTableName};

mod alter;
mod check;
mod column;
mod column_collation;
mod column_generated;
mod column_identity;
mod column_not_null;
mod constraint;
mod create;
mod create_as;
mod drop;
mod interval;
mod key;
mod rename;
mod truncate;

pub use alter::*;
pub use check::{Check, IntoCheck};
pub use column::*;
pub use column_generated::GeneratedKind;
pub use constraint::{
    ConstraintChange, ConstraintDrop, DropBehavior, Enforcement, IntoAddedConstraint,
    IntoConstraintDrop, NotNullConstraint, NotValid,
};
pub use create::*;
pub use create_as::TableCreateAsStatement;
pub use drop::*;
pub use interval::*;
pub use key::*;
pub use rename::*;
pub use truncate::*;

/// Helper for constructing any table statement
// [spec:pgorm:req:sql.ddl+12]
#[derive(Debug)]
pub struct Table;

impl Table {
    /// Construct table [`TableCreateStatement`] over the table it creates
    pub fn create<T>(table: T) -> TableCreateStatement
    where
        T: IntoTableName,
    {
        TableCreateStatement::new(table)
    }

    /// Construct a [`TableCreateAsStatement`] over the table it creates and
    /// the query whose columns and rows it is created with.
    ///
    /// # Errors
    ///
    /// The query's tie limit or WINDOW clause, when PostgreSQL's grammar
    /// refuses it, as for [`View::create`](crate::View::create).
    // [spec:pgorm:req:sql.ddl.create-table-as+1]
    pub fn create_as<T>(table: T, body: SelectStatement) -> Result<TableCreateAsStatement>
    where
        T: IntoTableName,
    {
        Ok(TableCreateAsStatement {
            name: table.into_table_name(),
            columns: Vec::new(),
            body: crate::view::definition_body(body)?,
            if_not_exists: false,
            no_data: false,
        })
    }

    /// Name the table a [`TableAlterStatement`] will alter.
    ///
    /// Choosing an action on the returned [`PendingTableAlter`] is what produces
    /// the statement: PostgreSQL has no spelling for an `ALTER TABLE` with no
    /// action, so naming the table alone renders nothing.
    pub fn alter<T>(table: T) -> PendingTableAlter
    where
        T: IntoTableName,
    {
        PendingTableAlter::new(table.into_table_name())
    }

    /// Construct table [`TableDropStatement`] over its first table
    pub fn drop<T>(table: T) -> TableDropStatement
    where
        T: IntoTableName,
    {
        TableDropStatement::new(table)
    }

    /// Construct table [`TableRenameStatement`] from the old and new name
    pub fn rename<T, R>(from_name: T, to_name: R) -> TableRenameStatement
    where
        T: IntoTableName,
        R: IntoName,
    {
        TableRenameStatement::new(from_name, to_name)
    }

    /// Construct column [`ColumnRenameStatement`] over a table and two column names
    pub fn rename_column<T, F, R>(table: T, from_name: F, to_name: R) -> ColumnRenameStatement
    where
        T: IntoTableName,
        F: IntoName,
        R: IntoName,
    {
        ColumnRenameStatement::new(table, from_name, to_name)
    }

    /// Construct constraint [`ConstraintRenameStatement`] over a table and two
    /// constraint names
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn rename_constraint<T, F, R>(
        table: T,
        from_name: F,
        to_name: R,
    ) -> ConstraintRenameStatement
    where
        T: IntoTableName,
        F: IntoName,
        R: IntoName,
    {
        ConstraintRenameStatement::new(table, from_name, to_name)
    }

    /// Construct table [`TableTruncateStatement`] over the table it empties
    pub fn truncate<T>(table: T) -> TableTruncateStatement
    where
        T: IntoTableName,
    {
        TableTruncateStatement::new(table)
    }
}
