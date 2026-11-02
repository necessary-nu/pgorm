//! Shared helpers for the `pgorm-codegen` spec-verification tests.
//!
//! Everything here drives the public generation pipeline —
//! `EntityTransformer::transform` followed by `EntityWriter::generate` — which
//! is the only surface reachable from outside the crate: `Entity`, `Column`,
//! `Relation` and friends all keep `pub(crate)` fields.
#![allow(dead_code)]

use pgorm_codegen::{EntityTransformer, EntityWriterContext, EntityWriterOptions, Error};
use pgorm_query::{
    ColumnDef, ColumnType, ForeignKey, ForeignKeyAction, Name, Table, TableCreateStatement,
    TableKey, Unique,
};
use proc_macro2::{Delimiter, TokenStream, TokenTree};

/// The full `EntityWriterContext::new` option set; `Opts::default()` is the
/// shape a caller gets with no flags: no serde, `mod.rs`.
pub type Opts = EntityWriterOptions;

pub fn context(opts: Opts) -> EntityWriterContext {
    EntityWriterContext::new(opts).expect("options should build a context")
}

/// The in-memory `WriterOutput`, keyed for convenient lookup while preserving
/// the emission order (which several rules constrain).
pub struct Generated {
    pub files: Vec<(String, String)>,
}

impl Generated {
    /// File contents by name; panics with the available names when missing.
    pub fn file(&self, name: &str) -> &str {
        self.files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| c.as_str())
            .unwrap_or_else(|| panic!("no generated file {name:?}; got {:?}", self.names()))
    }

    pub fn names(&self) -> Vec<&str> {
        self.files.iter().map(|(n, _)| n.as_str()).collect()
    }

    pub fn has(&self, name: &str) -> bool {
        self.files.iter().any(|(n, _)| n == name)
    }
}

/// Run the whole pipeline: schema statements in, generated files out.
pub fn generate(stmts: Vec<TableCreateStatement>, opts: Opts) -> Generated {
    let ctx = context(opts);
    let writer = EntityTransformer::transform(stmts).expect("transform should succeed");
    Generated {
        files: writer
            .generate(&ctx)
            .files
            .into_iter()
            .map(|f| (f.name, f.content))
            .collect(),
    }
}

/// The transform gate's refusal of a schema, by its exact message.
#[track_caller]
pub fn assert_transform_error(stmts: Vec<TableCreateStatement>, expected: &str) {
    match EntityTransformer::transform(stmts) {
        Err(Error::TransformError(msg)) => assert_eq!(msg, expected),
        other => panic!("expected a TransformError, got {other:?}"),
    }
}

/// Canonical token text — every token separated by exactly one space — so
/// expectations can be written as readable Rust instead of reproducing
/// `TokenStream::to_string`'s joint/alone spacing by hand.
pub fn norm(src: &str) -> String {
    let stream: TokenStream = src
        .parse()
        .expect("generated output should lex as Rust tokens");
    let mut tokens = Vec::new();
    flatten(stream, &mut tokens);
    tokens.join(" ")
}

fn flatten(stream: TokenStream, out: &mut Vec<String>) {
    for tree in stream {
        match tree {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                if !open.is_empty() {
                    out.push(open.to_owned());
                }
                flatten(group.stream(), out);
                if !close.is_empty() {
                    out.push(close.to_owned());
                }
            }
            other => out.push(other.to_string()),
        }
    }
}

#[track_caller]
pub fn assert_contains(haystack: &str, needle: &str) {
    let (h, n) = (norm(haystack), norm(needle));
    assert!(h.contains(&n), "expected to find\n  {n}\nin\n  {h}");
}

#[track_caller]
pub fn assert_not_contains(haystack: &str, needle: &str) {
    let (h, n) = (norm(haystack), norm(needle));
    assert!(!h.contains(&n), "expected NOT to find\n  {n}\nin\n  {h}");
}

#[track_caller]
pub fn assert_starts_with(haystack: &str, prefix: &str) {
    let (h, p) = (norm(haystack), norm(prefix));
    assert!(
        h.starts_with(&p),
        "expected block to start with\n  {p}\ngot\n  {h}"
    );
}

/// Index of a fragment inside the normalized haystack, for ordering assertions.
#[track_caller]
pub fn position_of(haystack: &str, needle: &str) -> usize {
    let (h, n) = (norm(haystack), norm(needle));
    h.find(&n)
        .unwrap_or_else(|| panic!("expected to find\n  {n}\nin\n  {h}"))
}

/// The blank-line-separated blocks of a generated entity file, minus the
/// generated-file header.
pub fn blocks(content: &str) -> Vec<&str> {
    content
        .split("\n\n")
        .map(str::trim)
        .filter(|b| !b.is_empty() && !b.starts_with("//!"))
        .collect()
}

pub fn runtime_name(name: &str) -> Name {
    Name::runtime(name)
}

pub fn col(name: &str) -> ColumnDef {
    ColumnDef::new(Name::runtime(name))
}

/// `name` integer, not null, auto-increment: the column a serial key is over.
pub fn serial(name: &str) -> ColumnDef {
    ColumnDef::new(Name::runtime(name))
        .integer()
        .not_null()
        .auto_increment()
        .to_owned()
}

/// `cake`: serial pk + nullable text name.
pub fn cake() -> TableCreateStatement {
    Table::create(Name::runtime("cake"))
        .col(serial("id"))
        .primary_key(Name::runtime("id"))
        .col(ColumnDef::new(Name::runtime("name")).text().to_owned())
        .to_owned()
}

/// `fruit`: serial pk, not-null name, nullable `cake_id` FK to `cake`.
pub fn fruit() -> TableCreateStatement {
    Table::create(Name::runtime("fruit"))
        .col(serial("id"))
        .primary_key(Name::runtime("id"))
        .col(
            ColumnDef::new(Name::runtime("name"))
                .string()
                .not_null()
                .to_owned(),
        )
        .col(
            ColumnDef::new(Name::runtime("cake_id"))
                .integer()
                .to_owned(),
        )
        .foreign_key(
            ForeignKey::create(
                Name::runtime("fruit"),
                Name::runtime("cake_id"),
                Name::runtime("cake"),
                Name::runtime("id"),
            )
            .on_delete(ForeignKeyAction::Cascade)
            .on_update(ForeignKeyAction::Cascade)
            .to_owned(),
        )
        .to_owned()
}

/// `filling`: serial pk + not-null name.
pub fn filling() -> TableCreateStatement {
    Table::create(Name::runtime("filling"))
        .col(serial("id"))
        .primary_key(Name::runtime("id"))
        .col(
            ColumnDef::new(Name::runtime("name"))
                .string()
                .not_null()
                .to_owned(),
        )
        .to_owned()
}

/// `cake_filling`: the classic junction table — two FK columns that together
/// form the primary key.
pub fn cake_filling() -> TableCreateStatement {
    Table::create(Name::runtime("cake_filling"))
        .col(
            ColumnDef::new(Name::runtime("cake_id"))
                .integer()
                .not_null()
                .to_owned(),
        )
        .col(
            ColumnDef::new(Name::runtime("filling_id"))
                .integer()
                .not_null()
                .to_owned(),
        )
        .primary_key(TableKey::new(Name::runtime("cake_id")).col(Name::runtime("filling_id")))
        .foreign_key(ForeignKey::create(
            Name::runtime("cake_filling"),
            Name::runtime("cake_id"),
            Name::runtime("cake"),
            Name::runtime("id"),
        ))
        .foreign_key(ForeignKey::create(
            Name::runtime("cake_filling"),
            Name::runtime("filling_id"),
            Name::runtime("filling"),
            Name::runtime("id"),
        ))
        .to_owned()
}

/// The cake / fruit / filling / cake_filling schema used by most tests.
pub fn cake_schema() -> Vec<TableCreateStatement> {
    vec![cake(), cake_filling(), filling(), fruit()]
}

/// A single table built from an explicit column list.
pub fn table_with(table: &str, columns: Vec<ColumnDef>) -> TableCreateStatement {
    let mut stmt = Table::create(Name::runtime(table));
    for column in columns {
        stmt.col(column);
    }
    stmt
}

/// A single table built from an explicit column list, keyed by the columns
/// `key` names.
pub fn keyed_with(table: &str, key: &[&str], columns: Vec<ColumnDef>) -> TableCreateStatement {
    let mut stmt = table_with(table, columns);
    let (first, rest) = key.split_first().expect("a key names a column");
    stmt.primary_key(
        TableKey::new(Name::runtime(*first)).cols(rest.iter().map(|column| Name::runtime(*column))),
    );
    stmt
}

/// A named one-column unique key over `column`, which the transformer reads
/// to mark the column unique.
pub fn unique_key(table: &str, column: &str) -> TableKey<Unique> {
    TableKey::new(Name::runtime(column)).name(Name::runtime(format!("idx_{table}_{column}")))
}

pub fn enum_col(name: &str, enum_name: &str, variants: &[&str]) -> ColumnDef {
    ColumnDef::new(Name::runtime(name))
        .enumeration(
            Name::runtime(enum_name),
            variants
                .iter()
                .map(|v| Name::runtime(*v))
                .collect::<Vec<_>>(),
        )
        .not_null()
        .to_owned()
}

pub fn typed(name: &str, ty: ColumnType) -> ColumnDef {
    ColumnDef::new_with_type(Name::runtime(name), ty)
        .not_null()
        .to_owned()
}

pub fn typed_null(name: &str, ty: ColumnType) -> ColumnDef {
    ColumnDef::new_with_type(Name::runtime(name), ty)
}
