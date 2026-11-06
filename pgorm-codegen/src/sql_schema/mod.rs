//! Entities from DDL text, with no database to inspect.
//!
//! [`parse_schema`] reads a `schema.sql` with libpg_query — the PostgreSQL
//! server's own parser — and bridges the parse tree into the
//! [`TableCreateStatement`]s
//! [`EntityTransformer::entities`](crate::EntityTransformer::entities)
//! consumes;
//! `entities_from_sql` runs the whole pipeline through the Rust writer and
//! hands back the generated files.
//!
//! The bridge understands the DDL the entity model has a place for — tables,
//! columns, primary keys, unique constraints, foreign keys, enum and range
//! types, domains, indexes and comments, the columns and constraints an
//! `ALTER TABLE` adds, and the schemas they are created in.
//! Anything else in the file is a named error rather than a silent omission:
//! a schema that generates entities is a schema the bridge understood in
//! full.
#![deny(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

mod expr;
mod objects;
mod table;
mod types;

#[cfg(feature = "rust")]
use crate::{EntityTransformer, EntityWriterContext, EntityWriterOptions, WriterOutput};
use crate::{Error, TableIdent, resolve_reference};
use pg_query::NodeEnum;
use pg_query::protobuf::{AlterTableStmt, CommentStmt, CreateStmt, IndexStmt};
use pgorm_query::{ColumnType, TableCreateStatement};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

/// A declared type's full identity: `(schema, name)`, `None` for an
/// unqualified declaration. A qualified and an unqualified declaration of one
/// name are distinct types, exactly as they are to PostgreSQL.
pub(crate) type TypeIdentity = (Option<String>, String);

/// What a `CREATE TYPE` in the file makes a column of that type.
pub(crate) enum DeclaredType {
    /// An enum, by its values in declaration order.
    Enum(Vec<String>),
    /// A range, by the column type of the subtype it ranges over.
    Range(ColumnType),
    /// The multirange created beside a range, by the same subtype.
    Multirange(ColumnType),
    /// A domain, by the column type of its base.
    Domain(ColumnType),
}

/// Declared types keyed by full identity. An enum, a range and a domain share
/// one namespace, as every type in a schema does.
type Declared = BTreeMap<TypeIdentity, DeclaredType>;

/// Read DDL text as the schema statements the entity transformer consumes.
///
/// Enum types are resolved into the column types that name them, and what
/// `ALTER TABLE`s add, indexes and comments are folded into the table they
/// describe, so the returned statements stand alone.
// [spec:pgorm:def:codegen.ddl+4]
// [spec:pgorm:req:codegen.ddl.unsupported+21]
pub fn parse_schema(sql: &str) -> Result<Vec<TableCreateStatement>, Error> {
    let parsed = pg_query::parse(sql, pg_query::ParserOptions::DEFAULT)
        .map_err(|err| Error::TransformError(format!("schema SQL did not parse: {err}")))?;
    build(collect(&parsed.protobuf)?)
}

/// Generate entity files straight from DDL text.
///
/// The whole pipeline: parse, bridge, [`EntityTransformer::transform`],
/// [`crate::EntityWriter::generate`]. Options are validated first, so an
/// unusable option is reported before the schema is read.
// [spec:pgorm:def:codegen.ddl+4]
#[cfg(feature = "rust")]
pub fn entities_from_sql(sql: &str, options: EntityWriterOptions) -> Result<WriterOutput, Error> {
    let context = EntityWriterContext::new(options)?;
    let writer = EntityTransformer::transform(parse_schema(sql)?, &context)?;
    Ok(writer.generate(&context))
}

/// A construct the bridge does not carry into the entity model.
// [spec:pgorm:req:codegen.ddl.unsupported+21]
fn unsupported(what: impl Display, at: usize) -> Error {
    Error::TransformError(format!("unsupported DDL: {what} at statement {at}"))
}

/// A construct the bridge understands but cannot resolve in this schema.
// [spec:pgorm:req:codegen.ddl.unsupported+21]
fn unresolved(problem: impl Display, at: usize) -> Error {
    Error::TransformError(format!("statement {at}: {problem}"))
}

/// The statements of one parse, sorted by kind and paired with their 1-based
/// position in the file.
#[derive(Default)]
struct Collected<'a> {
    declared: Declared,
    tables: Vec<(usize, &'a CreateStmt)>,
    alters: Vec<(usize, &'a AlterTableStmt)>,
    indexes: Vec<(usize, &'a IndexStmt)>,
    comments: Vec<(usize, &'a CommentStmt)>,
}

/// Sort every statement in the file by what the bridge reads it as, refusing
/// anything else by name.
///
/// A `CREATE SCHEMA` is read for its name alone, to refuse a second creation
/// of one schema as the server does (`42P06`) unless it says `IF NOT EXISTS`.
/// A qualified name is not held to the schemas the file creates: `public`
/// exists unasked, and a dump of some schemas names others it never creates.
// [spec:pgorm:req:codegen.ddl.unsupported+21]
// [spec:pgorm:sem:codegen.ddl.objects+12]
fn collect(parsed: &pg_query::protobuf::ParseResult) -> Result<Collected<'_>, Error> {
    let mut collected = Collected::default();
    let mut schemas = BTreeSet::new();
    for (index, raw) in parsed.stmts.iter().enumerate() {
        let at = index + 1;
        let Some(node) = raw.stmt.as_ref().and_then(|stmt| stmt.node.as_ref()) else {
            return Err(unsupported("an empty statement", at));
        };
        match node {
            NodeEnum::CreateSchemaStmt(stmt) => {
                if let Some(name) = objects::schema(stmt, at)?
                    && !schemas.insert(name.clone())
                    && !stmt.if_not_exists
                {
                    return Err(unresolved(format!("schema `{name}` is declared twice"), at));
                }
            }
            NodeEnum::CreateStmt(stmt) => collected.tables.push((at, stmt)),
            NodeEnum::AlterTableStmt(stmt) => collected.alters.push((at, stmt)),
            NodeEnum::IndexStmt(stmt) => collected.indexes.push((at, stmt.as_ref())),
            NodeEnum::CommentStmt(stmt) => collected.comments.push((at, stmt.as_ref())),
            NodeEnum::CreateEnumStmt(stmt) => {
                let (identity, values) = objects::enum_type(stmt, at)?;
                declare(
                    &mut collected.declared,
                    identity,
                    DeclaredType::Enum(values),
                    at,
                )?;
            }
            NodeEnum::CreateRangeStmt(stmt) => {
                let parsed = objects::range_type(stmt, &collected.declared, at)?;
                let subtype = DeclaredType::Multirange(parsed.subtype.clone());
                declare(
                    &mut collected.declared,
                    parsed.range,
                    DeclaredType::Range(parsed.subtype),
                    at,
                )?;
                declare(&mut collected.declared, parsed.multirange, subtype, at)?;
            }
            NodeEnum::CreateDomainStmt(stmt) => {
                let (identity, base) = objects::domain(stmt, &collected.declared, at)?;
                declare(
                    &mut collected.declared,
                    identity,
                    DeclaredType::Domain(base),
                    at,
                )?;
            }
            other => return Err(unsupported(statement_kind(other), at)),
        }
    }
    Ok(collected)
}

/// Record a type the file declares, refusing a second declaration of one
/// identity — whichever kinds the two are.
// [spec:pgorm:sem:codegen.ddl.objects+12]
fn declare(
    declared: &mut Declared,
    identity: TypeIdentity,
    kind: DeclaredType,
    at: usize,
) -> Result<(), Error> {
    let spelled = types::spell_identity(&identity);
    match declared.insert(identity, kind) {
        Some(_) => Err(unresolved(
            format!("type `{spelled}` is declared twice"),
            at,
        )),
        None => Ok(()),
    }
}

/// Resolve the collected statements against each other: declared types into
/// the columns naming them, what `ALTER TABLE`s add, indexes and comments into
/// the table they describe.
// [spec:pgorm:sem:codegen.ddl.objects+12]
fn build(collected: Collected<'_>) -> Result<Vec<TableCreateStatement>, Error> {
    let Collected {
        declared: types,
        tables,
        alters,
        indexes,
        comments,
    } = collected;

    let mut declared: Vec<TableIdent> = Vec::with_capacity(tables.len());
    let mut positions: BTreeMap<TableIdent, usize> = BTreeMap::new();
    for (position, (at, stmt)) in tables.iter().enumerate() {
        table::name(stmt, *at)?;
        let ident = table::ident(stmt);
        if positions.insert(ident.clone(), position).is_some() {
            return Err(unresolved(
                format!("table `{ident}` is declared twice"),
                *at,
            ));
        }
        declared.push(ident);
    }

    let mut attachments: Vec<table::Attachments> = Vec::new();
    attachments.resize_with(tables.len(), table::Attachments::default);
    let position_of = |table: &TableIdent, at: usize| -> Result<usize, Error> {
        let Some(target) = resolve_reference(&declared, table) else {
            let ambiguous = table.schema.is_none()
                && declared
                    .iter()
                    .any(|declared| declared.table == table.table);
            return Err(unresolved(
                if ambiguous {
                    format!("`{table}` names more than one table")
                } else {
                    format!("no CREATE TABLE for table `{table}`")
                },
                at,
            ));
        };
        positions
            .get(target)
            .copied()
            .ok_or_else(|| unresolved(format!("no CREATE TABLE for table `{table}`"), at))
    };

    for (at, stmt) in alters {
        let parsed = table::alter(stmt, at)?;
        let position = position_of(&parsed.table, at)?;
        if let Some(slot) = attachments.get_mut(position) {
            slot.added
                .extend(parsed.added.into_iter().map(|added| (at, added)));
        }
    }
    for (at, stmt) in indexes {
        let parsed = objects::index(stmt, at)?;
        let position = position_of(&parsed.table, at)?;
        if let (Some(slot), Some(index)) = (attachments.get_mut(position), parsed.constraint) {
            slot.indexes.push(index);
        }
    }
    for (at, stmt) in comments {
        let parsed = objects::comment(stmt, at)?;
        let position = position_of(parsed.table(), at)?;
        let Some(slot) = attachments.get_mut(position) else {
            continue;
        };
        match parsed {
            objects::ParsedComment::Table { text, .. } => slot.table_comment = Some(text),
            objects::ParsedComment::Column { column, text, .. } => {
                slot.column_comments.insert(column, (at, text));
            }
        }
    }

    let mut statements = Vec::with_capacity(tables.len());
    for (position, (at, stmt)) in tables.into_iter().enumerate() {
        let attachment = attachments
            .get_mut(position)
            .map(std::mem::take)
            .unwrap_or_default();
        statements.push(table::build(stmt, at, &types, attachment)?);
    }
    Ok(statements)
}

/// The SQL a statement the bridge does not read was written as, named the way
/// its author wrote it.
// [spec:pgorm:req:codegen.ddl.unsupported+21]
fn statement_kind(node: &NodeEnum) -> &'static str {
    match node {
        NodeEnum::AlterSeqStmt(_) => "ALTER SEQUENCE",
        NodeEnum::AlterTypeStmt(_) => "ALTER TYPE",
        NodeEnum::AlterOwnerStmt(_) => "ALTER ... OWNER TO",
        NodeEnum::AlterObjectSchemaStmt(_) => "ALTER ... SET SCHEMA",
        NodeEnum::RenameStmt(_) => "ALTER ... RENAME",
        NodeEnum::CreateSeqStmt(_) => "CREATE SEQUENCE",
        NodeEnum::CreateTrigStmt(_) => "CREATE TRIGGER",
        NodeEnum::CreateFunctionStmt(_) => "CREATE FUNCTION",
        NodeEnum::CreateExtensionStmt(_) => "CREATE EXTENSION",
        NodeEnum::CreatePolicyStmt(_) => "CREATE POLICY",
        NodeEnum::CreateForeignTableStmt(_) => "CREATE FOREIGN TABLE",
        NodeEnum::CreateTableAsStmt(create)
            if create.objtype == pg_query::protobuf::ObjectType::ObjectMatview as i32 =>
        {
            "CREATE MATERIALIZED VIEW"
        }
        NodeEnum::CreateTableAsStmt(_) => "CREATE TABLE AS",
        NodeEnum::RefreshMatViewStmt(_) => "REFRESH MATERIALIZED VIEW",
        NodeEnum::CreateStatsStmt(_) => "CREATE STATISTICS",
        NodeEnum::CompositeTypeStmt(_) => "CREATE TYPE ... AS",
        NodeEnum::ViewStmt(_) => "CREATE VIEW",
        NodeEnum::RuleStmt(_) => "CREATE RULE",
        NodeEnum::DefineStmt(_) => "CREATE TYPE, OPERATOR or AGGREGATE",
        NodeEnum::GrantStmt(_) => "GRANT or REVOKE",
        NodeEnum::DropStmt(_) => "DROP",
        NodeEnum::TruncateStmt(_) => "TRUNCATE",
        NodeEnum::InsertStmt(_) => "INSERT",
        NodeEnum::UpdateStmt(_) => "UPDATE",
        NodeEnum::DeleteStmt(_) => "DELETE",
        NodeEnum::SelectStmt(_) => "SELECT",
        NodeEnum::CopyStmt(_) => "COPY",
        NodeEnum::DoStmt(_) => "DO",
        NodeEnum::VariableSetStmt(_) => "SET",
        NodeEnum::TransactionStmt(_) => "a transaction statement",
        _ => "an unrecognised statement",
    }
}
