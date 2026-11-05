//! Interval columns: each qualifier PostgreSQL's `interval` takes, read by
//! the DDL bridge, and the entity the writer generates for it — the field an
//! `Interval` whatever the qualifier, the qualifier carried in the column's
//! definition so that the table the entity creates is the table described.

mod common;

use common::*;
use pgorm::{ColumnTrait, ColumnTypeTrait, Iterable, StaticName};
use pgorm_codegen::{Error, WriterOutput, entities_from_sql, parse_schema};
use pgorm_query::{ColumnType, IntervalPrecision, IntervalSpec, PgInterval, TableCreateStatement};
use std::sync::Arc;

const SCHEMA: &str = include_str!("sql/interval/schema.sql");

/// The entity the bridge generates for [`SCHEMA`], compiled here as well as
/// compared: the derive accepts every qualifier the writer emits. The live
/// round trip through it is `tests/codegen_interval_tests.rs` at the
/// workspace root.
#[path = "sql/interval/interval_span.rs"]
mod interval_span;

/// The columns [`SCHEMA`] declares nullable.
const NULLABLE: [&str; 3] = ["maybe", "clock", "budgets"];

fn digits(digits: u8) -> Option<IntervalPrecision> {
    IntervalPrecision::new(digits)
}

fn qualified(fields: PgInterval) -> ColumnType {
    ColumnType::Interval(IntervalSpec::Fields(fields))
}

fn plain() -> ColumnType {
    ColumnType::Interval(IntervalSpec::Any(None))
}

/// The column types of [`SCHEMA`]'s table, in column order.
fn described() -> Vec<(&'static str, ColumnType)> {
    vec![
        ("id", ColumnType::Integer),
        ("plain", plain()),
        ("maybe", plain()),
        (
            "precise",
            ColumnType::Interval(IntervalSpec::Any(digits(3))),
        ),
        ("months", qualified(PgInterval::YearToMonth)),
        ("days", qualified(PgInterval::Day)),
        ("clock", qualified(PgInterval::DayToSecond(digits(3)))),
        ("minutes", qualified(PgInterval::HourToMinute)),
        ("seconds", qualified(PgInterval::Second(digits(0)))),
        ("history", ColumnType::Array(Arc::new(plain()))),
        (
            "budgets",
            ColumnType::Array(Arc::new(qualified(PgInterval::YearToMonth))),
        ),
    ]
}

/// [`SCHEMA`]'s table, built with pgorm-query rather than written as text.
fn built() -> TableCreateStatement {
    let columns = described()
        .into_iter()
        .map(|(name, col_type)| match NULLABLE.contains(&name) {
            true => typed_null(name, col_type),
            false => typed(name, col_type),
        })
        .collect();
    keyed_with("interval_span", &["id"], columns)
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
fn error(sql: &str) -> String {
    match entities_from_sql(sql, Opts::default()) {
        Err(Error::TransformError(message)) => message,
        Err(other) => panic!("expected a TransformError, got {other:?}"),
        Ok(_) => panic!("expected an error, got generated entities"),
    }
}

#[track_caller]
fn column_type_of(sql: &str) -> ColumnType {
    let statements = parse_schema(sql).expect("the schema parses");
    statements[0].get_columns()[0]
        .get_column_type()
        .cloned()
        .expect("a column has a type")
}

// [spec:pgorm:sem:codegen.ddl.types+10/test]    each qualifier and precision
// `interval` takes reads back as the `IntervalSpec` that renders it, through
// an array as on its own
#[test]
fn the_bridge_reads_each_interval_qualifier() {
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

    for (spelled, fields) in [
        ("interval YEAR", PgInterval::Year),
        ("interval MONTH", PgInterval::Month),
        ("interval HOUR", PgInterval::Hour),
        ("interval MINUTE", PgInterval::Minute),
        ("interval SECOND", PgInterval::Second(None)),
        ("interval DAY TO HOUR", PgInterval::DayToHour),
        ("interval DAY TO MINUTE", PgInterval::DayToMinute),
        ("interval DAY TO SECOND", PgInterval::DayToSecond(None)),
        (
            "interval HOUR TO SECOND(6)",
            PgInterval::HourToSecond(digits(6)),
        ),
        (
            "interval MINUTE TO SECOND",
            PgInterval::MinuteToSecond(None),
        ),
        (
            "interval MINUTE TO SECOND(1)",
            PgInterval::MinuteToSecond(digits(1)),
        ),
    ] {
        assert_eq!(
            column_type_of(&format!("CREATE TABLE t (c {spelled});")),
            qualified(fields),
            "{spelled}"
        );
    }
    assert_eq!(
        column_type_of("CREATE TABLE t (c interval(0));"),
        ColumnType::Interval(IntervalSpec::Any(digits(0)))
    );
    // The generic spelling carries the server's own modifier, which reads as
    // the server reads it: a field mask of YEAR and MONTH.
    assert_eq!(
        column_type_of("CREATE TABLE t (c pg_catalog.interval(6));"),
        qualified(PgInterval::YearToMonth)
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+21/test]    a qualifier no
// `IntervalSpec` holds is named, as is a precision the server would quietly
// reduce
// [spec:pgorm:sem:codegen.ddl.types+10/test]
#[test]
fn an_interval_modifier_beyond_the_spec_is_named() {
    for (sql, what) in [
        (
            "CREATE TABLE t (c interval(7));",
            "an `interval` precision of 7, above the 6 PostgreSQL keeps, on column `t`.`c`",
        ),
        (
            "CREATE TABLE t (c interval DAY TO SECOND(9));",
            "an `interval` precision of 9, above the 6 PostgreSQL keeps, on column `t`.`c`",
        ),
        (
            "CREATE TABLE t (c pg_catalog.interval(4, 3));",
            "`interval` with a type modifier on column `t`.`c`",
        ),
        (
            "CREATE TABLE t (c pg_catalog.interval(3));",
            "`interval` with a type modifier on column `t`.`c`",
        ),
        (
            "CREATE TABLE t (c pg_catalog.interval(32767, 3, 1));",
            "`interval` with a type modifier on column `t`.`c`",
        ),
    ] {
        assert_eq!(
            error(sql),
            format!("unsupported DDL: {what} at statement 1"),
            "{sql}"
        );
    }
}

// [spec:pgorm:sem:codegen.entity.types+12/test]    an interval column is an
// `Interval` field, an array of them a `Vec<Interval>`, whatever the qualifier
// [spec:pgorm:sem:codegen.entity.compact.attrs+13/test]    the compact writer
// names the column type exactly where the field's `Interval` does not: a
// qualifier or a precision, alone or as an array's element
// [spec:pgorm:req:codegen.entity.types.unsupported+11/test]    no interval
// shape is refused
#[test]
fn interval_columns_generate_a_compiling_entity() {
    let generated =
        files(entities_from_sql(SCHEMA, Opts::default()).expect("the schema generates"));
    let written = norm(generated.file("interval_span.rs")).replace(", )", ")");
    assert!(
        written.contains(&norm(include_str!("sql/interval/interval_span.rs"))),
        "{written}"
    );

    for (column, (name, col_type)) in interval_span::Column::iter().zip(described()) {
        assert_eq!(column.as_str(), name);
        let expected = match NULLABLE.contains(&name) {
            true => col_type.def().null(),
            false => col_type.def(),
        };
        assert_eq!(column.def(), expected, "{name}");
    }
}

// [spec:pgorm:def:codegen.ddl+4/test]    a table of interval columns built
// with pgorm-query and rendered generates, read back, the entity the
// statement itself generates
#[test]
fn rendered_interval_columns_read_back_whole() {
    let text = format!("{};", built());
    assert_eq!(
        files(entities_from_sql(&text, Opts::default()).expect("the rendering generates")).files,
        generate(vec![built()], Opts::default()).files,
    );
}
