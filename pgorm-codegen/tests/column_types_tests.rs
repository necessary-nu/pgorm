//! A table of every kind of column type the DDL bridge reads and codegen
//! generates, and the compact entity generated for it: each column's type
//! stated wherever the field's Rust type alone would infer less — a length,
//! a precision and scale, `char` or `text`, an interval qualifier and a
//! created range's subtype.
//!
//! The files the writer is held to here are compiled, created as a table and
//! written through against a live server at the workspace root
//! (`tests/codegen_column_types_tests.rs`), whose module tree they are.

mod common;

use common::*;
use pgorm_codegen::entities_from_sql;

const SCHEMA: &str = include_str!("../../tests/codegen_column_types/schema.sql");

// [spec:pgorm:sem:codegen.entity.compact.attrs+11/test]    the compact writer states
// each column's whole type wherever its field's Rust type infers another: a
// `varchar(n)` or `char(n)` that `String` would infer as an unbounded `varchar`, a
// `numeric(p, s)`, and arrays of each, and a range over `text` whose newtype's
// `Range<String>` would infer one over `varchar`; every type its field infers states
// none
// [spec:pgorm:sem:codegen.entity.types+12/test]
#[test]
fn compact_entities_state_every_uninferred_type() {
    let generated = entities_from_sql(SCHEMA, Opts::default()).expect("the schema generates");
    let generated = Generated {
        files: generated
            .files
            .into_iter()
            .map(|file| (file.name, file.content))
            .collect(),
    };
    for (file, fixture) in [
        (
            "specimen.rs",
            include_str!("../../tests/codegen_column_types/specimen.rs"),
        ),
        (
            "pgorm_active_enums.rs",
            include_str!("../../tests/codegen_column_types/pgorm_active_enums.rs"),
        ),
        (
            "pgorm_range_types.rs",
            include_str!("../../tests/codegen_column_types/pgorm_range_types.rs"),
        ),
    ] {
        // rustfmt and the writer differ only on trailing commas.
        let flat = |src: &str| norm(src).replace(" , )", " )").replace(" , }", " }");
        let written = flat(generated.file(file));
        assert!(written.contains(&flat(fixture)), "{file}: {written}");
    }
}
