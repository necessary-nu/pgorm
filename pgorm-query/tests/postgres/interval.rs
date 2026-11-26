use super::*;
use crate::oracle::{assert_eq, parsed_nodes};

fn mixed() -> Interval {
    Interval::new(1, -2, 3_000_004)
}

/// The string constants PostgreSQL's parser read out of `sql`, in order.
fn string_constants(sql: &str) -> Vec<String> {
    parsed_nodes(sql, "AConst")
        .iter()
        .filter_map(|node| node["val"]["Sval"]["sval"].as_str().map(str::to_owned))
        .collect()
}

// [spec:pgorm:sem:sql.value.interval-text+2/test]    the literal is the text form in single
// quotes cast to `interval`, and the parser reads back exactly that text as one
// string constant
#[test]
fn interval_literal_is_its_text_quoted() {
    let cases = [
        (
            mixed(),
            "'+1 months -2 days +3000004 microseconds'::interval",
        ),
        (Interval::default(), "'+0 days'::interval"),
        (Interval::new(0, 0, -1), "'-1 microseconds'::interval"),
        (
            Interval::new(i32::MAX, i32::MAX, i64::MAX),
            "'infinity'::interval",
        ),
        (
            Interval::new(i32::MIN, i32::MIN, i64::MIN),
            "'-infinity'::interval",
        ),
        (
            Interval::new(i32::MIN, i32::MAX, i64::MIN),
            "'-2147483648 months +2147483647 days -9223372036854775808 microseconds'::interval",
        ),
    ];
    for (interval, literal) in cases {
        let rendered = Query::select().expr(interval).to_string();
        assert_eq!(rendered, format!("SELECT {literal}"));
        assert_eq!(string_constants(&rendered), vec![interval.to_string()]);
        assert_eq!(parsed_nodes(&rendered, "TypeCast").len(), 1);
    }
    assert_eq!(Value::Interval(None).to_string(), "NULL");
}

// [spec:pgorm:sem:sql.value.interval-text+2/test]    an array of intervals renders element by
// element, and an empty one names `interval[]`
#[test]
fn interval_array_literals_render_each_element() {
    let rendered = Query::select()
        .expr(Value::array([mixed(), Interval::new(0, 0, -1)]))
        .to_string();
    assert_eq!(
        rendered,
        "SELECT ARRAY ['+1 months -2 days +3000004 microseconds'::interval,'-1 microseconds'::interval]"
    );
    assert_eq!(
        string_constants(&rendered),
        vec![mixed().to_string(), "-1 microseconds".to_owned()]
    );
    assert_eq!(
        Query::select()
            .expr(Value::array(Vec::<Interval>::new()))
            .to_string(),
        "SELECT ARRAY []::interval[]"
    );
}

// [spec:pgorm:def:sql.value.interval+3/test]    a built statement binds the interval as one
// parameter, never as text
#[test]
fn built_statement_binds_an_interval() {
    assert_eq!(
        Query::select().expr(Expr::val(mixed())).build(),
        (
            "SELECT $1".to_owned(),
            Values(vec![Value::Interval(Some(Box::new(mixed())))])
        )
    );
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect])
            .values_panic([mixed().into()])
            .build(),
        (
            r#"INSERT INTO "glyph" ("aspect") VALUES ($1)"#.to_owned(),
            Values(vec![Value::from(mixed())])
        )
    );
}

// [spec:pgorm:def:sql.value.interval+3/test]    a cast operand pins its placeholder to
// `interval`, and an array of them to `interval[]`
#[test]
fn interval_parameter_pins_to_interval() {
    let pinned = |value: Value| {
        Query::select()
            .expr(Expr::val(value).cast_as(alias("anything")))
            .build()
            .0
    };
    assert_eq!(
        pinned(mixed().into()),
        "SELECT CAST($1::interval AS anything)"
    );
    assert_eq!(
        pinned(Value::Interval(None)),
        "SELECT CAST($1::interval AS anything)"
    );
    assert_eq!(
        pinned(Value::array([mixed()])),
        "SELECT CAST($1::interval[] AS anything)"
    );
}
