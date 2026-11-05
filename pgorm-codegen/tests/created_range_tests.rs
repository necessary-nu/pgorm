//! Range types a schema creates: `CREATE TYPE ... AS RANGE` read by the DDL
//! bridge, its columns typed by it, and the newtype each one generates.

mod common;

use common::*;
use pgorm::{ColumnTrait, ColumnTypeTrait, EntityTrait, Iterable, QueryTrait};
use pgorm_codegen::parse_schema;
use pgorm_query::{ColumnType, Name, StringLen};
use std::sync::Arc;

const SCHEMA: &str = include_str!("sql/created_range/schema.sql");

/// The two files the bridge generates for [`SCHEMA`], compiled here as well
/// as compared: the derives accept what the writer emits. The live round trip
/// through them is `tests/created_range_tests.rs` at the workspace root.
#[path = "sql/created_range/pgorm_range_types.rs"]
mod pgorm_range_types;

#[path = "sql/created_range/measurement.rs"]
mod measurement;

fn multi(name: &str, schema: Option<&str>, subtype: ColumnType) -> ColumnType {
    ColumnType::CreatedMultirange {
        name: Name::runtime(name),
        schema: schema.map(Name::runtime),
        subtype: Arc::new(subtype),
    }
}

fn created(name: &str, schema: Option<&str>, subtype: ColumnType) -> ColumnType {
    ColumnType::CreatedRange {
        name: Name::runtime(name),
        schema: schema.map(Name::runtime),
        subtype: Arc::new(subtype),
    }
}

// [spec:pgorm:sem:codegen.ddl.objects+12/test]    a range type the file creates
// types the columns naming it, by its full identity and with its subtype read
// as a column's type is
// [spec:pgorm:sem:codegen.ddl.types+10/test]
#[test]
fn a_created_range_types_its_columns() {
    let statements = parse_schema(SCHEMA).expect("the schema parses");
    let columns: Vec<_> = statements[0]
        .get_columns()
        .iter()
        .map(|column| column.get_column_type().cloned())
        .collect();
    assert_eq!(
        columns,
        [
            Some(ColumnType::Integer),
            Some(multi("floatmultirange", None, ColumnType::Double)),
            Some(created("floatrange", None, ColumnType::Double)),
            Some(created("slot", Some("booking"), ColumnType::Integer)),
            Some(multi(
                "slot_multirange",
                Some("booking"),
                ColumnType::Integer
            )),
            Some(created("textrange", None, ColumnType::Text)),
            Some(ColumnType::Array(Arc::new(created(
                "floatrange",
                None,
                ColumnType::Double
            )))),
            Some(ColumnType::Array(Arc::new(multi(
                "floatmultirange",
                None,
                ColumnType::Double
            )))),
        ]
    );
}

// [spec:pgorm:sem:codegen.ddl.objects+12/test]    a range type declares its
// multirange too: under the name PostgreSQL derives, in the range's schema, or
// under MULTIRANGE_TYPE_NAME as written
#[test]
fn a_created_range_declares_its_multirange() {
    let column_of = |sql: &str| {
        parse_schema(sql).expect("the schema parses")[0].get_columns()[1]
            .get_column_type()
            .cloned()
    };
    for (declared, column, name, schema) in [
        (
            "CREATE TYPE x_range_y_range AS RANGE (SUBTYPE = int4);",
            "x_multirange_y_range",
            "x_multirange_y_range",
            None,
        ),
        (
            "CREATE TYPE \"FloatRange\" AS RANGE (SUBTYPE = int4);",
            "\"FloatRange_multirange\"",
            "FloatRange_multirange",
            None,
        ),
        (
            "CREATE TYPE mr.plain AS RANGE (SUBTYPE = int4);",
            "mr.plain_multirange",
            "plain_multirange",
            Some("mr"),
        ),
        (
            "CREATE TYPE mr.q AS RANGE (SUBTYPE = int4, MULTIRANGE_TYPE_NAME = public.qm);",
            "public.qm",
            "qm",
            Some("public"),
        ),
        (
            "CREATE TYPE mr.q AS RANGE (SUBTYPE = int4, MULTIRANGE_TYPE_NAME = qm);",
            "qm",
            "qm",
            None,
        ),
    ] {
        assert_eq!(
            column_of(&format!(
                "{declared} CREATE TABLE t (id int PRIMARY KEY, m {column});"
            )),
            Some(multi(name, schema, ColumnType::Integer)),
            "{declared}"
        );
    }
    let long = "r".repeat(53);
    assert_eq!(
        column_of(&format!(
            "CREATE TYPE {long} AS RANGE (SUBTYPE = int4); \
             CREATE TABLE t (id int PRIMARY KEY, m {}_multirange);",
            "r".repeat(52)
        )),
        Some(multi(
            &format!("{}_multirange", "r".repeat(52)),
            None,
            ColumnType::Integer
        )),
    );
    let clipped = format!("{}range", "a".repeat(55));
    assert_eq!(
        column_of(&format!(
            "CREATE TYPE {clipped} AS RANGE (SUBTYPE = int4); \
             CREATE TABLE t (id int PRIMARY KEY, m {}multiran);",
            "a".repeat(55)
        )),
        Some(multi(
            &format!("{}multiran", "a".repeat(55)),
            None,
            ColumnType::Integer
        )),
    );
    assert_eq!(
        refused(
            "CREATE TYPE floatrange AS RANGE (SUBTYPE = float8); CREATE TYPE floatmultirange AS ENUM ('a');"
        ),
        "statement 2: type `floatmultirange` is declared twice"
    );
}

// [spec:pgorm:sem:codegen.entity.types+13/test]    one newtype per range type,
// named in UpperCamelCase and deriving `CreatedRange` under its name and
// schema, `Eq` but for a float subtype; the entity's fields name the newtypes
// [spec:pgorm:req:codegen.entity.files+2/test]    `pgorm_range_types.rs` is
// written and declared beside the entities
// [spec:pgorm:sem:codegen.entity.imports+4/test]    each entity imports the
// newtypes its columns name
// [spec:pgorm:sem:codegen.entity.compact.attrs+13/test]    a range over a subtype
// its newtype's `E` does not infer names its whole type
#[test]
fn a_created_range_generates_a_compiling_newtype() {
    let generated = from_sql(SCHEMA);
    assert_eq!(
        generated.names(),
        [
            "measurement.rs",
            "mod.rs",
            "prelude.rs",
            "pgorm_range_types.rs"
        ]
    );
    assert_contains(generated.file("mod.rs"), "pub mod pgorm_range_types;");
    for (file, fixture) in [
        (
            "pgorm_range_types.rs",
            include_str!("sql/created_range/pgorm_range_types.rs"),
        ),
        (
            "measurement.rs",
            include_str!("sql/created_range/measurement.rs"),
        ),
    ] {
        generated.holds(file, fixture);
    }

    assert_eq!(
        measurement::Column::Slot.def(),
        created("slot", Some("booking"), ColumnType::Integer)
            .def()
            .null()
    );
    assert_eq!(
        measurement::Column::Slots.def(),
        multi("slot_multirange", Some("booking"), ColumnType::Integer)
            .def()
            .null()
    );
    // `label`'s newtype holds a `Range<String>`, which infers a range over
    // `varchar`; the entity carries the `text` the schema names.
    let statements = parse_schema(SCHEMA).expect("the schema parses");
    for (column, described) in measurement::Column::iter().zip(statements[0].get_columns()) {
        assert_eq!(
            Some(column.def().get_column_type()),
            described.get_column_type(),
            "{column:?}"
        );
    }
    assert_eq!(
        <pgorm_range_types::Floatrange as pgorm::CreatedRange>::name().to_sql_string(),
        "floatrange"
    );
    assert_eq!(
        measurement::Entity::find().build().0,
        r#"SELECT "measurement"."id", CAST("measurement"."spans" AS text), "measurement"."span", "measurement"."slot", CAST("measurement"."slots" AS text), "measurement"."label", "measurement"."spares", CAST("measurement"."ledgers" AS text[]) FROM "measurement""#
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+21/test]    what the bridge cannot
// carry out of a range type is named: a subtype no newtype can hold, an
// option PostgreSQL does not have, a range with no subtype, a serial subtype,
// and a type declared twice
// [spec:pgorm:req:codegen.entity.types.unsupported+12/test]
#[test]
fn what_a_created_range_cannot_carry_is_named() {
    assert_eq!(
        refused(
            "CREATE TYPE flags AS RANGE (SUBTYPE = bool); CREATE TABLE t (id int PRIMARY KEY, f flags);"
        ),
        "table `t` column `f`: range type `flags` over Boolean is not supported by codegen; \
         a created range's subtype is one pgorm's `RangeSubtype` covers"
    );
    assert_eq!(
        refused("CREATE TYPE floatrange AS RANGE (SUBTYPE = float8, FLAVOUR = sweet);"),
        "unsupported DDL: option `flavour` on range type `floatrange` at statement 1"
    );
    assert_eq!(
        refused("CREATE TYPE floatrange AS RANGE (SUBTYPE_DIFF = float8mi);"),
        "statement 1: range type `floatrange` has no SUBTYPE"
    );
    assert_eq!(
        refused("CREATE TYPE serials AS RANGE (SUBTYPE = serial);"),
        "unsupported DDL: a serial type as the SUBTYPE of range type `serials` at statement 1"
    );
    assert_eq!(
        refused("CREATE TYPE span AS ENUM ('a'); CREATE TYPE span AS RANGE (SUBTYPE = int4);"),
        "statement 2: type `span` is declared twice"
    );
    assert_eq!(
        refused(
            "CREATE TYPE app.slot AS RANGE (SUBTYPE = int4); CREATE TABLE t (id int PRIMARY KEY, s slot);"
        ),
        "statement 2: type `slot` on column `t`.`s` is not declared; a same-named range type \
         under a different qualification does not resolve it"
    );
    assert_eq!(
        refused(
            "CREATE TYPE a.slot AS RANGE (SUBTYPE = int4); CREATE TYPE b.slot AS RANGE (SUBTYPE = int4); \
               CREATE TABLE t (id int PRIMARY KEY, x a.slot, y b.slot);"
        ),
        "range type `slot` is used under two qualifications; the generated Rust newtype can carry only one"
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+21/test]    a type modifier on a range
// type's SUBTYPE, which PostgreSQL drops, is named rather than carried, `char`'s
// implicit length among them; the subtype written without one is read
#[test]
fn a_dropped_subtype_modifier_is_named() {
    for subtype in [
        "numeric(10, 2)",
        "varchar(5)",
        "char(3)",
        "char",
        "character",
        "timestamp(3)",
        "interval YEAR TO MONTH",
    ] {
        assert_eq!(
            refused(&format!("CREATE TYPE r AS RANGE (SUBTYPE = {subtype});")),
            "unsupported DDL: a type modifier PostgreSQL drops on the SUBTYPE of range \
             type `r` at statement 1",
            "{subtype}"
        );
    }
    for (subtype, read) in [
        ("numeric", ColumnType::Decimal(None)),
        ("varchar", ColumnType::String(StringLen::None)),
        ("bpchar", ColumnType::Char(None)),
    ] {
        let statements = parse_schema(&format!(
            "CREATE TYPE r AS RANGE (SUBTYPE = {subtype}); \
             CREATE TABLE t (id int PRIMARY KEY, a r);"
        ))
        .expect("the schema parses");
        assert_eq!(
            statements[0].get_columns()[1].get_column_type(),
            Some(&created("r", None, read)),
            "{subtype}"
        );
    }
}

// [spec:pgorm:req:codegen.entity.types.unsupported+12/test]    a described range
// type whose subtype carries a type modifier fails the run by name, since
// PostgreSQL keeps none on a range's subtype
#[test]
fn a_described_subtype_modifier_is_refused() {
    for subtype in [
        ColumnType::Decimal(Some((10, 2))),
        ColumnType::String(StringLen::N(5)),
        ColumnType::Char(Some(1)),
    ] {
        let expected = format!(
            "table `t` column `a`: range type `r` over {subtype:?} is not supported by \
             codegen; PostgreSQL keeps no type modifier on a range's subtype"
        );
        assert_transform_error(
            vec![keyed_with(
                "t",
                &["id"],
                vec![serial("id"), typed("a", created("r", None, subtype))],
            )],
            &expected,
        );
    }
}

// [spec:pgorm:sem:codegen.ddl.objects+12/test]    the options that do not change
// a row's value are read and set aside, and a range over an enum the file
// declared reads its subtype as that enum, which no newtype holds
#[test]
fn range_options_are_read_and_set_aside() {
    let statements = parse_schema(
        "CREATE TYPE textrange AS RANGE (SUBTYPE = text, SUBTYPE_OPCLASS = text_pattern_ops, \
         COLLATION = \"C\", MULTIRANGE_TYPE_NAME = texts); \
         CREATE TABLE t (id int PRIMARY KEY, r textrange);",
    )
    .expect("the schema parses");
    assert_eq!(
        statements[0].get_columns()[1].get_column_type(),
        Some(&created("textrange", None, ColumnType::Text))
    );
    assert!(
        refused(
            "CREATE TYPE mood AS ENUM ('sad', 'happy'); \
             CREATE TYPE moodrange AS RANGE (SUBTYPE = mood); \
             CREATE TABLE t (id int PRIMARY KEY, r moodrange);"
        )
        .starts_with("table `t` column `r`: range type `moodrange` over Enum")
    );
}
