//! Enum arrays: columns typed with an array of an enum the file creates,
//! schema-qualified or not, nullable or not, and through a domain, and the
//! entity the writer generates for them — each field a `Vec` of the enum's
//! `ActiveEnum`.

mod common;

use common::*;
use pgorm::{ColumnTrait, ColumnTypeTrait, Iterable, StaticName};
use pgorm_codegen::{WriterOutput, entities_from_sql, parse_schema};
use pgorm_query::{ColumnType, Name};
use std::sync::Arc;

const SCHEMA: &str = include_str!("sql/enum_array/schema.sql");

/// The files the bridge generates for [`SCHEMA`], compiled here as well as
/// compared: the derives accept every enum array the writer emits. The live
/// round trip through them is `tests/codegen_enum_array_tests.rs` at the
/// workspace root.
#[path = "sql/enum_array/pgorm_active_enums.rs"]
mod pgorm_active_enums;

#[path = "sql/enum_array/survey.rs"]
mod survey;

/// The columns [`SCHEMA`] declares nullable.
const NULLABLE: [&str; 3] = ["past_moods", "past_tiers", "log"];

fn mood() -> ColumnType {
    ColumnType::Enum {
        name: Name::runtime("mood"),
        schema: None,
        variants: ["sad", "calm", "glad"].map(Name::runtime).to_vec(),
    }
}

fn tier() -> ColumnType {
    ColumnType::Enum {
        name: Name::runtime("tier"),
        schema: Some(Name::runtime("crm")),
        variants: ["free", "pro", "team"].map(Name::runtime).to_vec(),
    }
}

fn array(element: ColumnType) -> ColumnType {
    ColumnType::Array(Arc::new(element))
}

/// The column types of [`SCHEMA`]'s table, in column order.
fn described() -> Vec<(&'static str, ColumnType)> {
    vec![
        ("id", ColumnType::Integer),
        ("moods", array(mood())),
        ("past_moods", array(mood())),
        ("tiers", array(tier())),
        ("past_tiers", array(tier())),
        (
            "log",
            ColumnType::Domain {
                name: Name::runtime("mood_log"),
                schema: None,
                base: Arc::new(array(mood())),
            },
        ),
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

// [spec:pgorm:sem:codegen.ddl.types+10/test]    an array of an enum the file creates reads
// as an array of that enum by its full identity, a schema-qualified one included, and so
// does a domain over one
#[test]
fn the_bridge_reads_an_enum_array_column() {
    let statements = parse_schema(SCHEMA).expect("the schema parses");
    let columns: Vec<_> = statements[0]
        .get_columns()
        .iter()
        .map(|column| column.get_column_type().cloned())
        .collect();
    let expected: Vec<_> = described()
        .into_iter()
        .map(|(_, col_type)| Some(col_type))
        .collect();
    assert_eq!(columns, expected);
}

// [spec:pgorm:sem:codegen.entity.types+12/test]    an enum array column is a `Vec` of the
// enum's `ActiveEnum`, and the compact writer names no `column_type` for one, the derive
// inferring the array of the enum's type, its schema included; a domain over one is
// spelled with the enum's column type as its array's element
#[test]
fn enum_arrays_generate_a_compiling_entity() {
    let generated =
        files(entities_from_sql(SCHEMA, Opts::default()).expect("the schema generates"));
    let written = flat(generated.file("survey.rs"));
    assert!(
        written.contains(&flat(include_str!("sql/enum_array/survey.rs"))),
        "{written}"
    );
    assert!(
        norm(generated.file("pgorm_active_enums.rs"))
            .contains(&norm(include_str!("sql/enum_array/pgorm_active_enums.rs")))
    );

    for (column, (name, col_type)) in survey::Column::iter().zip(described()) {
        assert_eq!(column.as_str(), name);
        let expected = match NULLABLE.contains(&name) {
            true => col_type.def().null(),
            false => col_type.def(),
        };
        assert_eq!(column.def(), expected, "{name}");
    }
}
