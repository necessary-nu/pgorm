//! Domains: `CREATE DOMAIN` read by the DDL bridge, the columns typed by one,
//! and the entity the writer generates for them — each field the Rust type
//! of the domain's base, the domain's name carried in the column's definition.

mod common;

use common::*;
use pgorm::{ColumnTrait, ColumnTypeTrait, Iterable, StaticName};
use pgorm_codegen::{Error, WriterOutput, entities_from_sql, parse_schema};
use pgorm_query::{
    ColumnType, Domain, Expr, ForeignKey, Name, StringLen, TableCreateStatement,
    extension::{RangeDefinition, Type},
};
use std::sync::Arc;

const SCHEMA: &str = include_str!("sql/domain/schema.sql");

/// The files the bridge generates for [`SCHEMA`], compiled here as well as
/// compared: the derives accept every domain the writer emits. The live round
/// trip through them is `tests/codegen_domain_tests.rs` at the workspace
/// root.
#[path = "sql/domain/pgorm_active_enums.rs"]
mod pgorm_active_enums;

#[path = "sql/domain/pgorm_range_types.rs"]
mod pgorm_range_types;

#[path = "sql/domain/stock_line.rs"]
mod stock_line;

/// The columns [`SCHEMA`] declares nullable.
const NULLABLE: [&str; 4] = ["qty", "moods", "labels", "span"];

fn domain(name: &str, schema: Option<&str>, base: ColumnType) -> ColumnType {
    ColumnType::Domain {
        name: Name::runtime(name),
        schema: schema.map(Name::runtime),
        base: Arc::new(base),
    }
}

fn mood() -> ColumnType {
    ColumnType::Enum {
        name: Name::runtime("mood"),
        schema: None,
        variants: ["sad", "calm", "glad"].map(Name::runtime).to_vec(),
    }
}

fn quantity() -> ColumnType {
    domain("quantity", None, ColumnType::Integer)
}

fn code() -> ColumnType {
    domain("code", None, ColumnType::String(StringLen::N(8)))
}

fn cheerful() -> ColumnType {
    domain("cheerful", None, mood())
}

fn floatrange() -> ColumnType {
    ColumnType::CreatedRange {
        name: Name::runtime("floatrange"),
        schema: None,
        subtype: Arc::new(ColumnType::Double),
    }
}

/// The column types of [`SCHEMA`]'s table, in column order.
fn described() -> Vec<(&'static str, ColumnType)> {
    vec![
        ("id", ColumnType::Integer),
        ("qty", quantity()),
        ("small", domain("small_quantity", None, quantity())),
        ("code", code()),
        ("codes", ColumnType::Array(Arc::new(code()))),
        ("mood", cheerful()),
        ("moods", ColumnType::Array(Arc::new(cheerful()))),
        (
            "labels",
            domain(
                "labels",
                None,
                ColumnType::Array(Arc::new(ColumnType::Text)),
            ),
        ),
        ("span", domain("span", None, floatrange())),
        (
            "amount",
            domain(
                "amount",
                Some("billing"),
                ColumnType::Decimal(Some((12, 2))),
            ),
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

#[track_caller]
fn error(sql: &str) -> String {
    match entities_from_sql(sql, Opts::default()) {
        Err(Error::TransformError(message)) => message,
        Err(other) => panic!("expected a TransformError, got {other:?}"),
        Ok(_) => panic!("expected an error, got generated entities"),
    }
}

// [spec:pgorm:sem:codegen.ddl.types+9/test]    a column named by a domain the file creates
// reads as `ColumnType::Domain`, carrying the domain's identity and its base read as a
// column's type is — another domain, an enum, an array and a range type among them —
// through an array as on its own
// [spec:pgorm:sem:codegen.ddl.objects+10/test]
#[test]
fn the_bridge_reads_a_domain_column() {
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

// [spec:pgorm:sem:codegen.ddl.objects+10/test]    a domain's COLLATE, DEFAULT, NOT NULL, NULL
// and CHECKs are read and set aside: the entity a domain generates is the one its bare
// declaration does
#[test]
fn domain_constraints_are_set_aside() {
    let bare = "CREATE DOMAIN d AS text; CREATE TABLE t (id integer PRIMARY KEY, c d NOT NULL);";
    let dressed = "CREATE DOMAIN d AS text COLLATE \"C\" DEFAULT 'x' NOT NULL \
                   CONSTRAINT filled CHECK (VALUE <> '') CHECK (length(VALUE) < 9); \
                   CREATE TABLE t (id integer PRIMARY KEY, c d NOT NULL);";
    let nullable = "CREATE DOMAIN d AS text NULL; \
                    CREATE TABLE t (id integer PRIMARY KEY, c d NOT NULL);";
    let generate = |sql| files(entities_from_sql(sql, Opts::default()).expect("generates")).files;
    assert_eq!(generate(dressed), generate(bare));
    assert_eq!(generate(nullable), generate(bare));
}

// [spec:pgorm:req:codegen.ddl.unsupported+17/test]    a constraint PostgreSQL refuses on a
// domain is named, as are a serial base, a base the file does not declare and a domain
// column under a qualification the domain was not declared with
#[test]
fn refused_domain_shapes_are_named() {
    for (sql, what) in [
        (
            "CREATE DOMAIN d AS integer UNIQUE;",
            "a UNIQUE constraint on domain `d`",
        ),
        (
            "CREATE DOMAIN d AS integer PRIMARY KEY;",
            "a PRIMARY KEY on domain `d`",
        ),
        (
            "CREATE DOMAIN d AS integer REFERENCES t (id);",
            "a REFERENCES clause on domain `d`",
        ),
        (
            "CREATE DOMAIN d AS integer GENERATED ALWAYS AS (1) STORED;",
            "a GENERATED clause on domain `d`",
        ),
        (
            "CREATE DOMAIN d AS integer GENERATED ALWAYS AS IDENTITY;",
            "an identity clause on domain `d`",
        ),
        (
            "CREATE DOMAIN d AS integer CHECK (VALUE > 0) NO INHERIT;",
            "a NO INHERIT constraint on domain `d`",
        ),
        (
            "CREATE DOMAIN d AS integer CHECK (VALUE > 0) DEFERRABLE;",
            "a deferrable constraint on domain `d`",
        ),
        (
            "CREATE DOMAIN d AS integer CHECK (VALUE > 0) NOT ENFORCED;",
            "a NOT ENFORCED constraint on domain `d`",
        ),
        (
            "CREATE DOMAIN d AS serial;",
            "a serial type as the base type of domain `d`",
        ),
        (
            "CREATE DOMAIN d AS mood;",
            "type `mood` on the base type of domain `d`",
        ),
        (
            "CREATE DOMAIN a.b.d AS integer;",
            "a catalog-qualified type name in CREATE DOMAIN",
        ),
    ] {
        assert_eq!(
            error(sql),
            format!("unsupported DDL: {what} at statement 1"),
            "{sql}"
        );
    }

    assert_eq!(
        error(
            "CREATE DOMAIN billing.amount AS integer; \
             CREATE TABLE t (id integer PRIMARY KEY, a amount);"
        ),
        "statement 2: type `amount` on column `t`.`a` is not declared; a same-named domain \
         under a different qualification does not resolve it"
    );
    assert_eq!(
        error("CREATE TYPE d AS ENUM ('a'); CREATE DOMAIN d AS integer;"),
        "statement 2: type `d` is declared twice"
    );
}

// [spec:pgorm:req:codegen.entity.types.unsupported+10/test]    an array of a domain over an
// array, and an array of a domain over a range type a schema created, are refused by name;
// a domain over either alone generates
#[test]
fn unholdable_domain_arrays_are_named() {
    assert_eq!(
        error(
            "CREATE DOMAIN labels AS text[]; \
             CREATE TABLE t (id integer PRIMARY KEY, a labels[]);"
        ),
        "table `t` column `a`: an array of a domain over an array is not supported by \
         codegen; an entity field holds one dimension"
    );
    assert_eq!(
        error(
            "CREATE TYPE floatrange AS RANGE (SUBTYPE = float8); \
             CREATE DOMAIN span AS floatrange; \
             CREATE TABLE t (id integer PRIMARY KEY, a span[]);"
        ),
        "table `t` column `a`: an array of range type `floatrange` is not supported by \
         codegen; the newtype a created range generates has no array conversion"
    );
}

// [spec:pgorm:sem:codegen.entity.types+12/test]    a domain column is a field of its base's
// Rust type, an array of one a `Vec` of it, an enum or a created range's newtype included
// [spec:pgorm:sem:codegen.entity.compact.attrs+12/test]    the compact writer names every
// domain column's type, the domain and its base spelled in full
#[test]
fn domain_columns_generate_a_compiling_entity() {
    let generated =
        files(entities_from_sql(SCHEMA, Opts::default()).expect("the schema generates"));
    let written = flat(generated.file("stock_line.rs"));
    assert!(
        written.contains(&flat(include_str!("sql/domain/stock_line.rs"))),
        "{written}"
    );
    assert!(
        norm(generated.file("pgorm_active_enums.rs"))
            .contains(&norm(include_str!("sql/domain/pgorm_active_enums.rs")))
    );
    assert!(
        norm(generated.file("pgorm_range_types.rs"))
            .contains(&norm(include_str!("sql/domain/pgorm_range_types.rs")))
    );

    for (column, (name, col_type)) in stock_line::Column::iter().zip(described()) {
        assert_eq!(column.as_str(), name);
        let expected = match NULLABLE.contains(&name) {
            true => col_type.def().null(),
            false => col_type.def(),
        };
        assert_eq!(column.def(), expected, "{name}");
    }
}

/// [`SCHEMA`]'s types and table, built with pgorm-query rather than written
/// as text: the enum, the range type, each domain with its constraints, and
/// the table whose columns they type.
fn built() -> (Vec<String>, TableCreateStatement) {
    let n = Name::runtime;
    let types = vec![
        Type::create(n("mood"))
            .values(["sad", "calm", "glad"])
            .to_string(),
        Type::create(n("floatrange"))
            .as_range(RangeDefinition::new(ColumnType::Double))
            .to_string(),
    ];
    let mut statements = types;
    statements.extend([
        Domain::create(n("quantity"), ColumnType::Integer)
            .not_null()
            .default(1)
            .check(Expr::domain_value().gt(0))
            .to_string(),
        Domain::create(n("small_quantity"), quantity())
            .check(Expr::domain_value().lt(100))
            .to_string(),
        Domain::create(n("code"), ColumnType::String(StringLen::N(8)))
            .collate(n("C"))
            .to_string(),
        Domain::create(n("cheerful"), mood())
            .check(Expr::domain_value().ne("sad"))
            .to_string(),
        Domain::create(n("labels"), ColumnType::Array(Arc::new(ColumnType::Text))).to_string(),
        Domain::create(n("span"), floatrange()).to_string(),
        Domain::create(
            (n("billing"), n("amount")),
            ColumnType::Decimal(Some((12, 2))),
        )
        .to_string(),
    ]);
    let columns = described()
        .into_iter()
        .map(|(name, col_type)| match NULLABLE.contains(&name) {
            true => typed_null(name, col_type),
            false => typed(name, col_type),
        })
        .collect();
    (statements, keyed_with("stock_line", &["id"], columns))
}

// [spec:pgorm:def:codegen.ddl+4/test]    domains and a table of domain columns built with
// pgorm-query and rendered generate, read back, the entity the statement itself generates
#[test]
fn rendered_domain_columns_read_back_whole() {
    let (statements, table) = built();
    let text = statements
        .iter()
        .map(|statement| format!("{statement};"))
        .chain([format!("{table};")])
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        files(entities_from_sql(&text, Opts::default()).expect("the rendering generates")).files,
        generate(vec![table], Opts::default()).files,
    );
}

/// A table `p` keyed by `k` and a table `c` whose `r` references it, with
/// `k` and `r` of the types given, over the enum `mood` and the types
/// `declared` creates.
fn referencing(declared: &str, key: &str, reference: &str) -> String {
    format!(
        "CREATE TYPE mood AS ENUM ('sad', 'glad'); {declared} \
         CREATE TABLE p (k {key} PRIMARY KEY); \
         CREATE TABLE c (id integer PRIMARY KEY, r {reference} REFERENCES p (k));"
    )
}

const UNKEYABLE: &str = "and PostgreSQL implements no foreign key with a column typed as a \
                         domain over an enum on either side (42804)";

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a foreign key matching a column typed
// as a domain over an enum, through further domains too, is refused on either side whatever
// the other side is, in a composite key and a self-reference as alone
#[test]
fn keys_over_an_enum_domain_are_refused() {
    let one = "CREATE DOMAIN cheerful AS mood;";
    let two = "CREATE DOMAIN cheerful AS mood; CREATE DOMAIN content AS mood;";
    let nested = "CREATE DOMAIN cheerful AS mood; CREATE DOMAIN beaming AS cheerful;";
    for (declared, key, reference) in [
        (one, "cheerful", "cheerful"),
        (two, "cheerful", "content"),
        (one, "mood", "cheerful"),
        (one, "cheerful", "mood"),
        (nested, "cheerful", "beaming"),
    ] {
        assert_eq!(
            error(&referencing(declared, key, reference)),
            format!("table `c`: relation to `p` matches column `r` with `k`, {UNKEYABLE}"),
            "`{reference}` referencing `{key}`"
        );
    }
    assert_eq!(
        error(
            "CREATE TYPE mood AS ENUM ('sad'); CREATE DOMAIN cheerful AS mood; \
             CREATE TABLE p (n integer, k cheerful, PRIMARY KEY (n, k)); \
             CREATE TABLE c (id integer PRIMARY KEY, n integer, r cheerful, \
             FOREIGN KEY (n, r) REFERENCES p (n, k));"
        ),
        format!("table `c`: relation to `p` matches column `r` with `k`, {UNKEYABLE}")
    );
    assert_eq!(
        error(
            "CREATE TYPE mood AS ENUM ('sad'); CREATE DOMAIN cheerful AS mood; \
             CREATE TABLE t (k cheerful PRIMARY KEY, up cheerful REFERENCES t (k));"
        ),
        format!("table `t`: relation to `t` matches column `up` with `k`, {UNKEYABLE}")
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a statement built in code is held to
// the same refusal, and every pairing PostgreSQL keys still generates its relation: an enum
// with itself, a domain over an integer with the integer or itself, a domain over an array of
// an enum with itself
#[test]
fn keyable_domain_pairs_generate_their_relation() {
    let cheerful = || domain("cheerful", None, mood());
    assert_transform_error(
        vec![
            keyed_with("p", &["k"], vec![typed("k", cheerful())]),
            keyed_with(
                "c",
                &["id"],
                vec![typed("id", ColumnType::Integer), typed("r", mood())],
            )
            .foreign_key(ForeignKey::create(
                Name::runtime("c"),
                Name::runtime("r"),
                Name::runtime("p"),
                Name::runtime("k"),
            ))
            .to_owned(),
        ],
        &format!("table `c`: relation to `p` matches column `r` with `k`, {UNKEYABLE}"),
    );

    let tally = "CREATE DOMAIN tally AS integer;";
    let moods = "CREATE DOMAIN moods AS mood[];";
    for (declared, key, reference) in [
        ("", "mood", "mood"),
        (tally, "tally", "integer"),
        (tally, "integer", "tally"),
        (tally, "tally", "tally"),
        (moods, "moods", "moods"),
    ] {
        let generated = files(
            entities_from_sql(&referencing(declared, key, reference), Opts::default())
                .unwrap_or_else(|error| panic!("`{reference}` referencing `{key}`: {error}")),
        );
        assert_contains(
            &flat(generated.file("c.rs")),
            &flat(
                r#"#[pgorm(belongs_to = super::p::Entity, from = Column::R, to = super::p::Column::K)]"#,
            ),
        );
    }
}
