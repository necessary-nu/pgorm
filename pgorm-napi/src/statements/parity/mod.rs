//! Parity between the JavaScript builders and pgorm-query's own: each family's
//! cases, built here directly with pgorm-query, must build exactly the SQL and
//! values `tests/parity/<family>.json` holds, which the JavaScript suite holds
//! its own builders to in both runtimes. A case missing on either side fails.

mod copy;
mod json;
mod merge;
mod pipeline;
mod schema;
mod select;
mod views;
mod windows;
mod writes;

use std::collections::BTreeMap;

use pgorm::pgorm_query::{Name, Value, Values};
use serde_json::Value as Json;

use crate::values::value_tag;

/// A name the binding would take as an identifier: 1–63 bytes without NUL.
pub(crate) fn n(name: &str) -> Name {
    assert!(
        (1..=63).contains(&name.len()) && !name.contains('\0'),
        "{name:?} is no identifier the binding takes"
    );
    Name::runtime(name)
}

/// A value as the golden file writes it: its kind, and a text the JavaScript
/// suite writes the same way from the `Value` it inspects.
fn canonical(value: &Value) -> [String; 2] {
    [value_tag(value).name().to_owned(), text(value)]
}

fn text(value: &Value) -> String {
    match value {
        Value::Bool(Some(b)) => b.to_string(),
        Value::TinyInt(Some(v)) => v.to_string(),
        Value::SmallInt(Some(v)) => v.to_string(),
        Value::Int(Some(v)) => v.to_string(),
        Value::BigInt(Some(v)) => v.to_string(),
        Value::Unsigned(Some(v)) => v.to_string(),
        Value::BigUnsigned(Some(v)) => v.to_string(),
        Value::Float(Some(v)) => f64::from(*v).to_string(),
        Value::Double(Some(v)) => v.to_string(),
        Value::String(Some(v)) => v.to_string(),
        Value::Char(Some(v)) => v.to_string(),
        Value::Json(Some(v)) => v.to_string(),
        Value::Decimal(Some(v)) => v.to_string(),
        Value::Uuid(Some(v)) => v.to_string(),
        Value::Date(Some(v)) => v.to_string(),
        Value::Time(Some(v)) => v.to_string(),
        Value::DateTime(Some(v)) => v.to_string(),
        Value::DateTimeWithTimeZone(Some(v)) => v.to_string(),
        Value::Array(_, Some(items)) => {
            let items: Vec<String> = items.iter().map(text).collect();
            format!("[{}]", items.join(","))
        }
        Value::Range(_, Some(range)) => range_text(range),
        Value::Multirange(_, Some(ranges)) => {
            let ranges: Vec<String> = ranges.iter().map(range_text).collect();
            format!("{{{}}}", ranges.join(","))
        }
        other if other.is_null() => "null".to_owned(),
        other => panic!("no canonical text for {other:?}"),
    }
}

/// A range as the module's `Range` writes itself: `[1,5)`, an unbounded side
/// empty, or `empty`.
fn range_text(range: &pgorm::pgorm_query::Range<Value>) -> String {
    use std::ops::Bound;
    let pgorm::pgorm_query::Range::Bounds { lower, upper } = range else {
        return "empty".to_owned();
    };
    let (open, lower) = match lower {
        Bound::Included(value) => ("[", text(value)),
        Bound::Excluded(value) => ("(", text(value)),
        Bound::Unbounded => ("(", String::new()),
    };
    let (close, upper) = match upper {
        Bound::Included(value) => ("]", text(value)),
        Bound::Excluded(value) => (")", text(value)),
        Bound::Unbounded => (")", String::new()),
    };
    format!("{open}{lower},{upper}{close}")
}

fn golden(file: &str) -> BTreeMap<String, (String, Vec<[String; 2]>)> {
    let cases: BTreeMap<String, Json> =
        serde_json::from_str(file).expect("the golden file is JSON");
    cases
        .into_iter()
        .map(|(name, case)| {
            let sql = case["sql"].as_str().expect("a case has its SQL").to_owned();
            let values = case["values"]
                .as_array()
                .expect("a case has its values")
                .iter()
                .map(|pair| {
                    let pair = pair.as_array().expect("a value is [kind, text]");
                    [
                        pair[0].as_str().expect("a kind").to_owned(),
                        pair[1].as_str().expect("a text").to_owned(),
                    ]
                })
                .collect();
            (name, (sql, values))
        })
        .collect()
}

/// Hold `built`, a family's cases built with pgorm-query, to its golden file.
pub(crate) fn check(file: &str, built: Vec<(&str, (String, Values))>) {
    let golden = golden(file);
    let names: Vec<&str> = built.iter().map(|(name, _)| *name).collect();
    let expected: Vec<&str> = golden.keys().map(String::as_str).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted, expected,
        "the Rust cases and the golden file name the same cases"
    );
    for (name, (sql, values)) in built {
        let (golden_sql, golden_values) = &golden[name];
        assert_eq!(&sql, golden_sql, "{name}: SQL");
        let values: Vec<[String; 2]> = values.0.iter().map(canonical).collect();
        assert_eq!(&values, golden_values, "{name}: values");
    }
}

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:bindings.expressions+3/test]
// [spec:pgorm:req:bindings.quantifiers+3/test]
// [spec:pgorm:req:bindings.cte-options/test]
#[test]
fn select_family_matches_its_golden_file() {
    check(
        include_str!("../../../tests/parity/select.json"),
        select::cases(),
    );
}

// [spec:pgorm:req:bindings.writes+2/test]
// [spec:pgorm:req:bindings.write-sources/test]
// [spec:pgorm:req:bindings.conflict-arbiters/test]
// [spec:pgorm:req:bindings.write-ctes/test]
#[test]
fn writes_family_matches_its_golden_file() {
    check(
        include_str!("../../../tests/parity/writes.json"),
        writes::cases(),
    );
}

// [spec:pgorm:req:bindings.merge+2/test]
// [spec:pgorm:req:bindings.merge-sources/test]
#[test]
fn merge_family_matches_its_golden_file() {
    check(
        include_str!("../../../tests/parity/merge.json"),
        merge::cases(),
    );
}

// [spec:pgorm:req:bindings.sql-json/test]
#[test]
fn json_family_matches_its_golden_file() {
    check(
        include_str!("../../../tests/parity/json.json"),
        json::cases(),
    );
}

// [spec:pgorm:req:bindings.windows/test]
// [spec:pgorm:req:bindings.ranges/test]
#[test]
fn window_and_range_family_matches_its_golden_file() {
    check(
        include_str!("../../../tests/parity/windows.json"),
        windows::cases(),
    );
}

// [spec:pgorm:req:bindings.schema/test]
// [spec:pgorm:req:bindings.schema-tables/test]
// [spec:pgorm:req:bindings.schema-alter/test]
// [spec:pgorm:req:bindings.schema-indexes/test]
// [spec:pgorm:req:bindings.schema-types/test]
// [spec:pgorm:req:bindings.schema-sequences/test]
// [spec:pgorm:req:bindings.schema-views+2/test]
// [spec:pgorm:req:bindings.schema-schemas/test]
// [spec:pgorm:req:bindings.schema-domains+2/test]
#[test]
fn schema_family_matches_its_golden_file() {
    let mut cases = schema::cases();
    cases.extend(views::cases());
    check(include_str!("../../../tests/parity/schema.json"), cases);
}

// [spec:pgorm:req:bindings.pipeline/test]
// [spec:pgorm:req:bindings.pipeline-expressions+3/test]
// [spec:pgorm:req:bindings.pipeline-binder/test]
#[test]
fn pipeline_family_matches_its_golden_file() {
    check(
        include_str!("../../../tests/parity/pipeline.json"),
        pipeline::cases(),
    );
}

// [spec:pgorm:req:bindings.copy/test]
#[test]
fn copy_family_matches_its_golden_file() {
    check(
        include_str!("../../../tests/parity/copy.json"),
        copy::cases(),
    );
}
