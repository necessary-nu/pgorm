use pgorm_query::{
    TableAlterStatement, TableCreateStatement, TableDropStatement,
    extension::{TypeAlterStatement, TypeCreateStatement, TypeDropStatement},
};
use std::fmt;

/// One statement of a list [`Schema`](crate::Schema) returns when the list
/// holds statements of more than one kind, such as
/// [`Schema::create_disjoint`](crate::Schema::create_disjoint)'s: each runs
/// after the ones before it, and renders as its statement does.
///
/// The enum is `#[non_exhaustive]` because a later list may hold a kind of
/// statement none does yet.
// [spec:pgorm:sem:disjoint.schema+2]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum DdlStatement {
    /// A type, such as the PostgreSQL enum a disjoint entity's kinds are.
    CreateType(TypeCreateStatement),
    /// A table, with those of its foreign keys whose tables exist before it.
    CreateTable(TableCreateStatement),
    /// A change to a table created before it, such as a foreign key to a
    /// table created after that one.
    AlterTable(TableAlterStatement),
    /// A change to a type, such as a value added to an enum.
    // [spec:pgorm:sem:disjoint.evolve+2]
    AlterType(TypeAlterStatement),
    /// A type dropped.
    // [spec:pgorm:sem:disjoint.evolve+2]
    DropType(TypeDropStatement),
    /// A table dropped.
    // [spec:pgorm:sem:disjoint.evolve+2]
    DropTable(TableDropStatement),
}

impl fmt::Display for DdlStatement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateType(statement) => statement.fmt(f),
            Self::CreateTable(statement) => statement.fmt(f),
            Self::AlterTable(statement) => statement.fmt(f),
            Self::AlterType(statement) => statement.fmt(f),
            Self::DropType(statement) => statement.fmt(f),
            Self::DropTable(statement) => statement.fmt(f),
        }
    }
}
