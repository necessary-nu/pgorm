//! pgorm-query's DDL builders, from JavaScript: schemas, tables and their
//! columns, keys and constraints, their alterations, indexes, types,
//! domains, sequences, views, materialized views and tables created from a
//! query, extensions and comments.
//!
//! A DDL statement takes no parameters — PostgreSQL accepts none there — so
//! every builder here renders through pgorm-query's own `Display`, which
//! writes each value it carries as an escaped literal, and the statement runs
//! as SQL text with no values beside it. Nothing here concatenates a name or a
//! value into SQL: identifiers are minted with `Name::runtime` and quoted where
//! pgorm-query writes them, and literals are pgorm-query's.
//!
//! The typestates pgorm-query holds — an `ALTER TABLE`, `ALTER TYPE`,
//! `ALTER DOMAIN` or `ALTER SEQUENCE` before its first action — are parts of
//! their own, which no terminal runs and `inspect()` refuses.

mod alter;
mod column;
mod domain;
mod index;
mod namespace;
mod options;
mod sequence;
mod table;
mod types;
mod view;

pub(crate) use options::{ACTIONS as FOREIGN_KEY_ACTIONS, relation};
pub(super) use options::{get, integer, names, object, pick, text};

use pgorm::pgorm_query::{
    ColumnDef, ColumnRenameStatement, CommentStatement, ConstraintRenameStatement,
    DomainAlterStatement, DomainCreateStatement, DomainDropStatement, IndexCreateStatement,
    IndexDropStatement, MaterializedViewCreateStatement, MaterializedViewDropStatement,
    MaterializedViewRefreshStatement, PendingDomainAlter, PendingSequenceAlter, PendingTableAlter,
    RecursiveViewCreateStatement, SchemaCreateStatement, SchemaDropStatement,
    SchemaRenameStatement, SequenceAlterStatement, SequenceCreateStatement, SequenceDropStatement,
    SequenceRenameStatement, TableAlterStatement, TableCreateAsStatement, TableCreateStatement,
    TableDropStatement, TableName, TableRenameStatement, TableTruncateStatement,
    ViewCreateStatement, ViewDropStatement,
    extension::{
        AttributeRenameStatement, CompositeAlterStatement, ExtensionCreateStatement,
        ExtensionDropStatement, PendingTypeAlter, TypeAlterStatement, TypeCreateStatement,
        TypeDropStatement,
    },
};

use neon::prelude::*;

use super::{Node, args::this};

/// Every schema builder export, by module.
pub(super) const EXPORTS: &[&[(&str, super::Build)]] = &[
    column::EXPORTS,
    table::EXPORTS,
    alter::EXPORTS,
    index::EXPORTS,
    types::EXPORTS,
    sequence::EXPORTS,
    view::EXPORTS,
    namespace::EXPORTS,
    domain::EXPORTS,
];

/// A complete DDL statement, which runs as it renders.
// Each variant is a whole pgorm-query statement; the node boxes the lot.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub(crate) enum Ddl {
    CreateSchema(SchemaCreateStatement),
    DropSchema(SchemaDropStatement),
    RenameSchema(SchemaRenameStatement),
    CreateTable(TableCreateStatement),
    /// The table is kept beside the statement for the foreign keys a later
    /// action adds, which name it.
    AlterTable(TableName, TableAlterStatement),
    DropTable(TableDropStatement),
    RenameTable(TableRenameStatement),
    RenameColumn(ColumnRenameStatement),
    RenameConstraint(ConstraintRenameStatement),
    Truncate(TableTruncateStatement),
    CreateIndex(IndexCreateStatement),
    DropIndex(IndexDropStatement),
    CreateType(TypeCreateStatement),
    AlterType(TypeAlterStatement),
    AlterComposite(CompositeAlterStatement),
    RenameAttribute(AttributeRenameStatement),
    DropType(TypeDropStatement),
    CreateDomain(DomainCreateStatement),
    AlterDomain(DomainAlterStatement),
    DropDomain(DomainDropStatement),
    CreateSequence(SequenceCreateStatement),
    AlterSequence(SequenceAlterStatement),
    DropSequence(SequenceDropStatement),
    RenameSequence(SequenceRenameStatement),
    /// The four statements over a query are boxed, the query being several
    /// times the size of any other statement here.
    CreateView(Box<ViewCreateStatement>),
    CreateRecursiveView(Box<RecursiveViewCreateStatement>),
    DropView(ViewDropStatement),
    CreateMaterializedView(Box<MaterializedViewCreateStatement>),
    RefreshMaterializedView(MaterializedViewRefreshStatement),
    DropMaterializedView(MaterializedViewDropStatement),
    CreateTableAs(Box<TableCreateAsStatement>),
    CreateExtension(ExtensionCreateStatement),
    DropExtension(ExtensionDropStatement),
    Comment(CommentStatement),
}

impl Ddl {
    /// The statement's SQL, every value in it an escaped literal: pgorm-query's
    /// one rendering of a DDL statement that runs.
    // [spec:pgorm:req:bindings.schema]
    fn sql(&self) -> String {
        match self {
            Self::CreateSchema(statement) => statement.to_string(),
            Self::DropSchema(statement) => statement.to_string(),
            Self::RenameSchema(statement) => statement.to_string(),
            Self::CreateTable(statement) => statement.to_string(),
            Self::AlterTable(_, statement) => statement.to_string(),
            Self::DropTable(statement) => statement.to_string(),
            Self::RenameTable(statement) => statement.to_string(),
            Self::RenameColumn(statement) => statement.to_string(),
            Self::RenameConstraint(statement) => statement.to_string(),
            Self::Truncate(statement) => statement.to_string(),
            Self::CreateIndex(statement) => statement.to_string(),
            Self::DropIndex(statement) => statement.to_string(),
            Self::CreateType(statement) => statement.to_string(),
            Self::AlterType(statement) => statement.to_string(),
            Self::AlterComposite(statement) => statement.to_string(),
            Self::RenameAttribute(statement) => statement.to_string(),
            Self::DropType(statement) => statement.to_string(),
            Self::CreateDomain(statement) => statement.to_string(),
            Self::AlterDomain(statement) => statement.to_string(),
            Self::DropDomain(statement) => statement.to_string(),
            Self::CreateSequence(statement) => statement.to_string(),
            Self::AlterSequence(statement) => statement.to_string(),
            Self::DropSequence(statement) => statement.to_string(),
            Self::RenameSequence(statement) => statement.to_string(),
            Self::CreateView(statement) => statement.to_string(),
            Self::CreateRecursiveView(statement) => statement.to_string(),
            Self::DropView(statement) => statement.to_string(),
            Self::CreateMaterializedView(statement) => statement.to_string(),
            Self::RefreshMaterializedView(statement) => statement.to_string(),
            Self::DropMaterializedView(statement) => statement.to_string(),
            Self::CreateTableAs(statement) => statement.to_string(),
            Self::CreateExtension(statement) => statement.to_string(),
            Self::DropExtension(statement) => statement.to_string(),
            Self::Comment(statement) => statement.to_string(),
        }
    }
}

/// The schema builder state one JavaScript object owns.
#[derive(Debug, Clone)]
pub(crate) enum Part {
    Statement(Ddl),
    Column(ColumnDef),
    /// An `ALTER TABLE` naming its table and no action yet.
    PendingAlterTable(TableName, PendingTableAlter),
    /// An `ALTER TYPE` naming its type and no change yet.
    PendingAlterType(PendingTypeAlter),
    /// An `ALTER DOMAIN` naming its domain and no action yet.
    PendingAlterDomain(PendingDomainAlter),
    /// An `ALTER SEQUENCE` naming its sequence and no clause yet.
    PendingAlterSequence(PendingSequenceAlter),
}

impl Part {
    /// What the part is, as an error names it.
    pub(crate) fn describe(&self) -> &'static str {
        match self {
            Self::Statement(_) => "a DDL statement",
            Self::Column(_) => "a ColumnDef",
            Self::PendingAlterTable(..) => "an ALTER TABLE with no action",
            Self::PendingAlterType(_) => "an ALTER TYPE with no change",
            Self::PendingAlterDomain(_) => "an ALTER DOMAIN with no action",
            Self::PendingAlterSequence(_) => "an ALTER SEQUENCE with no clause",
        }
    }

    /// The SQL the part runs as, or why it runs as none: PostgreSQL parses no
    /// `ALTER` without an action, and a column is no statement.
    // [spec:pgorm:req:bindings.schema]
    pub(crate) fn statement(&self) -> Result<String, &'static str> {
        match self {
            Self::Statement(ddl) => Ok(ddl.sql()),
            Self::Column(_) => Err("a ColumnDef is not a statement to run: add it to a table"),
            Self::PendingAlterTable(..) => Err(
                "an ALTER TABLE needs an action before it can be inspected or run: addColumn(..), \
                 dropColumn(..), addCheck(..) and the rest",
            ),
            Self::PendingAlterType(_) => Err(
                "an ALTER TYPE needs a change before it can be inspected or run: addValue(..), \
                 renameTo(..), addAttribute(..) and the rest",
            ),
            Self::PendingAlterDomain(_) => Err(
                "an ALTER DOMAIN needs an action before it can be inspected or run: setDefault(..), \
                 addCheck(..), renameTo(..) and the rest",
            ),
            Self::PendingAlterSequence(_) => Err(
                "an ALTER SEQUENCE needs a clause before it can be inspected or run: options(..), \
                 restart(..), ownedBy(..) and the rest",
            ),
        }
    }
}

impl From<Part> for Node {
    fn from(part: Part) -> Self {
        Node::Schema(Box::new(part))
    }
}

impl From<Ddl> for Node {
    fn from(ddl: Ddl) -> Self {
        Part::Statement(ddl).into()
    }
}

/// The receiver, argument 0, as the part `pick` takes, or a refusal naming
/// `what` was expected and what came instead, which `pick` describes.
pub(super) fn receiver<T>(
    cx: &mut FunctionContext,
    what: &str,
    pick: impl FnOnce(Part) -> Result<T, &'static str>,
) -> NeonResult<T> {
    let found = match this(cx, 0)? {
        Node::Schema(part) => match pick(*part) {
            Ok(found) => return Ok(found),
            Err(other) => other,
        },
        other => other.describe(),
    };
    cx.throw_type_error(format!("expected {what}, got {found}"))
}
