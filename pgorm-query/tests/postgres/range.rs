use super::*;
use crate::oracle::{assert_eq, assert_parses, parsed_nodes};
use pgorm_query::extension::{RangeDefinition, Type};
use std::ops::Bound::{Excluded, Included, Unbounded};

fn n(name: &str) -> Name {
    Name::runtime(name)
}

/// The one `CreateRangeStmt` in `sql`, as PostgreSQL's parser read it.
fn range_statement(sql: &str) -> serde_json::Value {
    let mut found = parsed_nodes(sql, "CreateRangeStmt");
    assert_eq!(found.len(), 1, "exactly one range type in {sql}");
    found.remove(0)
}

fn parts(list: &serde_json::Value) -> Vec<String> {
    list.as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|part| {
            part["String"]["sval"]
                .as_str()
                .expect("a name part")
                .to_owned()
        })
        .collect()
}

/// Each option the parser read: its name, and the name parts of the type or
/// object it names.
fn options(statement: &serde_json::Value) -> Vec<(String, Vec<String>)> {
    statement["params"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|param| {
            let param = &param["DefElem"];
            (
                param["defname"]
                    .as_str()
                    .expect("an option name")
                    .to_owned(),
                parts(&param["arg"]["TypeName"]["names"]),
            )
        })
        .collect()
}

fn s(text: &str) -> String {
    text.to_owned()
}

// [spec:pgorm:req:sql.ddl.type-range+2/test]    the subtype, then each option that is set, every
// name quoted and the multirange's qualified as the range's can be
#[test]
fn a_range_type_writes_its_options() {
    let sql = Type::create((n("app"), n("Span")))
        .as_range(
            RangeDefinition::new(ColumnType::Text)
                .subtype_opclass(n("text_pattern_ops"))
                .collation((n("pg_catalog"), n("C")))
                .subtype_diff(n("text_diff"))
                .multirange_type_name((n("app"), n("Spans"))),
        )
        .to_string();
    assert_eq!(
        sql,
        [
            r#"CREATE TYPE "app"."Span" AS RANGE (SUBTYPE = text,"#,
            r#"SUBTYPE_OPCLASS = "text_pattern_ops", COLLATION = "pg_catalog"."C","#,
            r#"SUBTYPE_DIFF = "text_diff", MULTIRANGE_TYPE_NAME = "app"."Spans")"#,
        ]
        .join(" ")
    );
    let read = range_statement(&sql);
    assert_eq!(parts(&read["type_name"]), [s("app"), s("Span")]);
    assert_eq!(
        options(&read),
        [
            (s("subtype"), vec![s("text")]),
            (s("subtype_opclass"), vec![s("text_pattern_ops")]),
            (s("collation"), vec![s("pg_catalog"), s("C")]),
            (s("subtype_diff"), vec![s("text_diff")]),
            (s("multirange_type_name"), vec![s("app"), s("Spans")]),
        ]
    );
}

// [spec:pgorm:req:sql.ddl.type-range+2/test]    the subtype alone is a range type, written as a
// column's type is
#[test]
fn a_range_type_needs_only_its_subtype() {
    for (subtype, rendered, read) in [
        (
            ColumnType::Double,
            "double precision",
            vec![s("pg_catalog"), s("float8")],
        ),
        (
            ColumnType::TimestampWithTimeZone,
            "timestamp with time zone",
            vec![s("pg_catalog"), s("timestamptz")],
        ),
        (
            ColumnType::named("My Type"),
            r#""My Type""#,
            vec![s("My Type")],
        ),
        (
            ColumnType::Array(std::sync::Arc::new(ColumnType::Integer)),
            "integer[]",
            vec![s("pg_catalog"), s("int4")],
        ),
    ] {
        let sql = Type::create(n("r"))
            .as_range(RangeDefinition::new(subtype))
            .to_string();
        assert_eq!(
            sql,
            format!(r#"CREATE TYPE "r" AS RANGE (SUBTYPE = {rendered})"#)
        );
        assert_eq!(options(&range_statement(&sql)), [(s("subtype"), read)]);
    }
}

// [spec:pgorm:req:sql.ddl.type-range+2/test]    a range is a third kind of type, and choosing a
// kind replaces the others
#[test]
fn a_range_is_one_kind_of_type() {
    let range = || RangeDefinition::new(ColumnType::Integer);
    let sql = Type::create(n("t"))
        .values(["a"])
        .attribute(n("x"), ColumnType::Integer)
        .as_range(range())
        .to_string();
    assert_eq!(sql, r#"CREATE TYPE "t" AS RANGE (SUBTYPE = integer)"#);
    let sql = Type::create(n("t")).as_range(range()).as_enum().to_string();
    assert_eq!(sql, r#"CREATE TYPE "t" AS ENUM ()"#);
    let sql = Type::create(n("t"))
        .as_range(range())
        .attribute(n("x"), ColumnType::Text)
        .to_string();
    assert_eq!(sql, r#"CREATE TYPE "t" AS ("x" text)"#);
    let sql = Type::create(n("t"))
        .as_range(range())
        .as_range(RangeDefinition::new(ColumnType::BigInteger))
        .to_string();
    assert_eq!(sql, r#"CREATE TYPE "t" AS RANGE (SUBTYPE = bigint)"#);
}

// [spec:pgorm:req:sql.ddl.type-range+2/test]    a range type binds nothing, so its two renderings
// agree
#[test]
fn a_range_type_binds_nothing() {
    let statement = Type::create(n("t"))
        .as_range(RangeDefinition::new(ColumnType::Date).subtype_diff(n("date_diff")))
        .to_owned();
    let (sql, values) = statement.build();
    assert_eq!(sql, statement.to_string());
    assert!(values.0.is_empty());
}

// [spec:pgorm:def:sql.value.range+5/test]    a column of a built-in range type is written by the
// catalogue name
#[test]
fn a_range_column_type_is_its_catalogue_name() {
    let sql = Table::create(n("t"))
        .col(ColumnDef::new_with_type(
            n("a"),
            ColumnType::Range(RangeType::Int4),
        ))
        .col(ColumnDef::new_with_type(
            n("b"),
            ColumnType::Range(RangeType::TimestampTz),
        ))
        .col(ColumnDef::new_with_type(
            n("c"),
            ColumnType::Multirange(RangeType::Numeric),
        ))
        .col(ColumnDef::new_with_type(
            n("d"),
            ColumnType::Array(std::sync::Arc::new(ColumnType::Range(RangeType::Date))),
        ))
        .to_string();
    assert_eq!(
        sql,
        r#"CREATE TABLE "t" ( "a" int4range, "b" tstzrange, "c" nummultirange, "d" daterange[] )"#
    );
    assert_parses(&sql);
}

// [spec:pgorm:sem:sql.value.render+2/test]    a range literal is its type's constructor called with
// each bound's own literal, NULL for no bound, and the brackets for inclusivity
#[test]
fn a_range_literal_calls_its_constructor() {
    let cases: [(Value, &str); 9] = [
        (Range::from(1i32..5).into(), "int4range(1, 5, '[)')"),
        (
            Range::new(Excluded(1i64), Included(5)).into(),
            "int8range(1, 5, '(]')",
        ),
        (Range::<i32>::from(..).into(), "int4range(NULL, NULL, '()')"),
        (Range::<i32>::Empty.into(), "'empty'::int4range"),
        (
            Range::new(Included(rust_decimal::Decimal::new(-150, 2)), Unbounded).into(),
            "numrange(-1.50, NULL, '[)')",
        ),
        (
            Range::from(jiff::civil::date(2024, 1, 1)..=jiff::civil::date(2024, 1, 31)).into(),
            "daterange('2024-01-01', '2024-01-31', '[]')",
        ),
        (
            Range::from(jiff::civil::date(2024, 1, 1).at(12, 0, 0, 500_000_000)..).into(),
            "tsrange('2024-01-01 12:00:00.5', NULL, '[)')",
        ),
        (
            [Range::from(1i32..3), Range::Empty]
                .into_iter()
                .collect::<Multirange<i32>>()
                .into(),
            "int4multirange(int4range(1, 3, '[)'), 'empty'::int4range)",
        ),
        (
            Multirange::<jiff::Timestamp>::default().into(),
            "tstzmultirange()",
        ),
    ];
    for (value, literal) in cases {
        assert_eq!(value.to_string(), literal);
        assert_parses(&format!("SELECT {literal}"));
    }
    assert_eq!(Value::Range(RangeType::Int4, None).to_string(), "NULL");
    assert_eq!(Value::Multirange(RangeType::Date, None).to_string(), "NULL");
    let null_lower = Value::Range(
        RangeType::Int4,
        Some(Box::new(Range::new(
            Included(Value::Int(None)),
            Excluded(Value::Int(Some(5))),
        ))),
    );
    assert_eq!(null_lower.to_string(), "int4range(NULL, 5, '[)')");
}

// [spec:pgorm:req:sql.render.cast-param-type+4/test]    a range pins to its range type, a
// multirange to its multirange type, and an array of ranges to the range type's array
#[test]
fn a_range_parameter_pins_to_its_range_type() {
    let pinned = |value: Value| {
        Query::select()
            .expr(Expr::val(value).cast_as(alias("anything")))
            .build()
            .0
    };
    assert_eq!(
        pinned(Range::from(1i32..5).into()),
        "SELECT CAST($1::int4range AS anything)"
    );
    assert_eq!(
        pinned(Multirange::<rust_decimal::Decimal>::default().into()),
        "SELECT CAST($1::nummultirange AS anything)"
    );
    assert_eq!(
        pinned(Value::array([Range::from(1i64..2)])),
        "SELECT CAST($1::int8range[] AS anything)"
    );
    assert_eq!(
        pinned(Value::Range(RangeType::TimestampTz, None)),
        "SELECT CAST($1::tstzrange AS anything)"
    );
}

// [spec:pgorm:def:sql.value+3/test]    range values compare and hash structurally, bound by bound,
// and NULL keeps its range type
#[test]
fn range_values_agree_on_identity() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let hash = |value: &Value| {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    };
    let a = Value::from(Range::from(1i32..5));
    let b = Value::from(Range::from(1i32..5));
    assert_eq!(a, b);
    assert_eq!(hash(&a), hash(&b));
    assert_ne!(a, Value::from(Range::from(1i32..=5)));
    assert_ne!(a, Value::from(Range::from(1i64..5)));
    assert_ne!(
        Value::Range(RangeType::Int4, None),
        Value::Range(RangeType::Int8, None)
    );
    assert_ne!(
        Value::Range(RangeType::Int4, None),
        Value::Multirange(RangeType::Int4, None)
    );
    assert!(std::mem::size_of::<Value>() <= 4 * std::mem::size_of::<usize>());
}

// [spec:pgorm:req:sql.ast.expr.operators+6/test]    `overlaps` is `&&` between its operands, bound
// or inlined, and the parser reads one operator expression
#[test]
fn overlaps_renders_the_overlap_operator() {
    let query = Query::select()
        .column(n("id"))
        .from(n("t"))
        .and_where(Expr::col(n("span")).overlaps(Range::from(1i32..5)))
        .and_where(
            Expr::col(n("tags")).overlaps(Value::array([String::from("a"), String::from("b")])),
        )
        .to_owned();
    let (sql, values) = query.build();
    assert_eq!(
        sql,
        r#"SELECT "id" FROM "t" WHERE "span" && $1 AND "tags" && $2"#
    );
    assert_eq!(
        values,
        Values(vec![
            Range::from(1i32..5).into(),
            Value::array([String::from("a"), String::from("b")])
        ])
    );
    let inline = query.to_string();
    assert_eq!(
        inline,
        r#"SELECT "id" FROM "t" WHERE "span" && int4range(1, 5, '[)') AND "tags" && ARRAY ['a','b']"#
    );
    for sql in [&sql, &inline] {
        let overlaps: Vec<_> = parsed_nodes(sql, "AExpr")
            .into_iter()
            .filter(|node| node["name"][0]["String"]["sval"] == "&&")
            .collect();
        assert_eq!(overlaps.len(), 2, "{sql}");
    }
}
