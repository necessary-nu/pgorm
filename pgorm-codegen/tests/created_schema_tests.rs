//! `CREATE SCHEMA` in the DDL the bridge reads: a dump that creates its own
//! schemas generates the entities the same file without them does, every
//! table and type in a schema carrying that schema as its qualifier.

mod common;

use common::*;
use pgorm::{ColumnTrait, ColumnTypeTrait, EntityName, Iterable, StaticName};
use pgorm_codegen::{Error, WriterOutput, entities_from_sql};
use pgorm_query::extension::Type;
use pgorm_query::{ColumnDef, ColumnType, Name, Schema, Table, TableCreateStatement};
use std::sync::Arc;

const SCHEMA: &str = include_str!("sql/created_schema/schema.sql");

/// The files the bridge generates for [`SCHEMA`], compiled here as well as
/// compared. The live round trip through them is
/// `tests/codegen_schema_tests.rs` at the workspace root.
#[path = "sql/created_schema/pgorm_active_enums.rs"]
mod pgorm_active_enums;

#[path = "sql/created_schema/deal.rs"]
mod deal;

/// [`SCHEMA`]'s statements after its `CREATE SCHEMA`.
fn in_the_schema() -> &'static str {
    SCHEMA
        .strip_prefix("CREATE SCHEMA sales;\n")
        .expect("the fixture opens by creating its schema")
}

fn stage() -> ColumnType {
    ColumnType::Enum {
        name: Name::runtime("stage"),
        schema: Some(Name::runtime("sales")),
        variants: ["lead", "won", "lost"].map(Name::runtime).to_vec(),
    }
}

/// The column types of [`SCHEMA`]'s table, in column order, and whether each
/// is nullable.
fn described() -> Vec<(&'static str, ColumnType, bool)> {
    vec![
        ("id", ColumnType::Integer, false),
        ("title", ColumnType::Text, false),
        ("stage", stage(), false),
        ("history", ColumnType::Array(Arc::new(stage())), true),
    ]
}

/// Token text with the trailing commas rustfmt writes into a fixture and the
/// writer does not, so the two compare as the same program.
fn flat(src: &str) -> String {
    norm(src).replace(" , }", " }").replace(" , )", " )")
}

fn files(output: WriterOutput) -> Generated {
    Generated {
        files: output
            .files
            .into_iter()
            .map(|file| (file.name, file.content))
            .collect(),
    }
}

#[track_caller]
fn from_sql(sql: &str, opts: Opts) -> Generated {
    files(entities_from_sql(sql, opts).expect("the schema generates"))
}

#[track_caller]
fn error(sql: &str) -> String {
    match entities_from_sql(sql, Opts::default()) {
        Err(Error::TransformError(message)) => message,
        Err(other) => panic!("expected a TransformError, got {other:?}"),
        Ok(_) => panic!("expected an error, got generated entities"),
    }
}

// [spec:pgorm:sem:codegen.ddl.objects+12/test]    a file that creates its schema generates
// what the file without the `CREATE SCHEMA` does: the table and enum in the schema are
// qualified by it, and the schema adds nothing else
// [spec:pgorm:req:codegen.ddl.unsupported+21/test]
#[test]
fn a_created_schema_generates_a_compiling_entity() {
    let generated = from_sql(SCHEMA, Opts::default());
    assert_eq!(
        generated.files,
        from_sql(in_the_schema(), Opts::default()).files
    );
    let written = flat(generated.file("deal.rs"));
    assert!(
        written.contains(&flat(include_str!("sql/created_schema/deal.rs"))),
        "{written}"
    );
    assert!(
        norm(generated.file("pgorm_active_enums.rs")).contains(&norm(include_str!(
            "sql/created_schema/pgorm_active_enums.rs"
        )))
    );

    assert_eq!(deal::Entity.schema_name(), Some("sales"));
    for (column, (name, col_type, nullable)) in deal::Column::iter().zip(described()) {
        assert_eq!(column.as_str(), name);
        let expected = match nullable {
            true => col_type.def().null(),
            false => col_type.def(),
        };
        assert_eq!(column.def(), expected, "{name}");
    }
}

// [spec:pgorm:sem:codegen.ddl.objects+12/test]    every form of `CREATE SCHEMA` without
// elements is read and contributes nothing: `IF NOT EXISTS`, an `AUTHORIZATION` role set
// aside, a schema named for its role, and one named for a role keyword, which is read with
// no name; a second creation saying `IF NOT EXISTS` is read too
// [spec:pgorm:req:codegen.ddl.unsupported+21/test]
#[test]
fn every_create_schema_form_is_read() {
    let bare = from_sql(in_the_schema(), Opts::default()).files;
    for schema in [
        "CREATE SCHEMA IF NOT EXISTS sales;",
        "CREATE SCHEMA sales AUTHORIZATION sales_owner;",
        "CREATE SCHEMA IF NOT EXISTS sales AUTHORIZATION CURRENT_USER;",
        "CREATE SCHEMA AUTHORIZATION sales;",
        "CREATE SCHEMA IF NOT EXISTS AUTHORIZATION sales;",
        "CREATE SCHEMA AUTHORIZATION CURRENT_USER; CREATE SCHEMA AUTHORIZATION CURRENT_USER;",
        "CREATE SCHEMA sales; CREATE SCHEMA IF NOT EXISTS sales;",
        "CREATE SCHEMA \"Sales\"; CREATE SCHEMA sales;",
    ] {
        assert_eq!(
            from_sql(&format!("{schema}\n{}", in_the_schema()), Opts::default()).files,
            bare,
            "{schema}"
        );
    }
}

// [spec:pgorm:req:codegen.ddl.unsupported+21/test]    a schema created twice without
// `IF NOT EXISTS` is refused as the server refuses it, a schema named for its role
// included, and a statement written inside a `CREATE SCHEMA` is named by its kind
#[test]
fn refused_schema_shapes_are_named() {
    for (sql, expected) in [
        (
            "CREATE SCHEMA sales; CREATE SCHEMA sales;",
            "statement 2: schema `sales` is declared twice",
        ),
        (
            "CREATE SCHEMA IF NOT EXISTS sales; CREATE SCHEMA sales;",
            "statement 2: schema `sales` is declared twice",
        ),
        (
            "CREATE SCHEMA AUTHORIZATION sales; CREATE SCHEMA sales AUTHORIZATION joe;",
            "statement 2: schema `sales` is declared twice",
        ),
        (
            "CREATE SCHEMA sales CREATE TABLE deal (id integer PRIMARY KEY);",
            "unsupported DDL: CREATE TABLE inside CREATE SCHEMA `sales` at statement 1",
        ),
        (
            "CREATE SCHEMA sales CREATE INDEX deal_id ON deal (id);",
            "unsupported DDL: CREATE INDEX inside CREATE SCHEMA `sales` at statement 1",
        ),
        (
            "CREATE SCHEMA sales CREATE VIEW v AS SELECT 1;",
            "unsupported DDL: CREATE VIEW inside CREATE SCHEMA `sales` at statement 1",
        ),
        (
            "CREATE SCHEMA AUTHORIZATION joe CREATE SEQUENCE s;",
            "unsupported DDL: CREATE SEQUENCE inside CREATE SCHEMA `joe` at statement 1",
        ),
        (
            "CREATE SCHEMA AUTHORIZATION CURRENT_USER GRANT SELECT ON deal TO joe;",
            "unsupported DDL: GRANT or REVOKE inside CREATE SCHEMA at statement 1",
        ),
    ] {
        assert_eq!(error(sql), expected, "{sql}");
    }
}

/// [`SCHEMA`]'s statements built with pgorm-query rather than written as
/// text, the schema created twice as a migration might, the second time only
/// if it does not exist.
fn built() -> (Vec<String>, TableCreateStatement) {
    let n = Name::runtime;
    let mut table = Table::create((n("sales"), n("deal")));
    for (name, col_type, nullable) in described() {
        let mut column = ColumnDef::new_with_type(n(name), col_type);
        if !nullable {
            column.not_null();
        }
        table.col(column);
    }
    table.primary_key(n("id"));
    let statements = vec![
        Schema::create(n("sales")).to_string(),
        Schema::create(n("sales")).if_not_exists().to_string(),
        Type::create((n("sales"), n("stage")))
            .values(["lead", "won", "lost"])
            .to_string(),
    ];
    (statements, table)
}

// [spec:pgorm:def:codegen.ddl+4/test]    a schema, an enum and a table built with
// pgorm-query and rendered generate, read back, the entity the table statement itself
// generates
#[test]
fn rendered_created_schema_reads_back_whole() {
    let (statements, table) = built();
    let text = statements
        .iter()
        .map(|statement| format!("{statement};"))
        .chain([format!("{table};")])
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        from_sql(&text, Opts::default()).files,
        generate(vec![table], Opts::default()).files,
    );
}
