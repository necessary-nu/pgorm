//! `JsonValueType`: the four impls it generates, and the refusal the
//! `From<T> for Value` conversion makes when `serde_json` cannot serialize the
//! value at all.

use pgorm::JsonValueType;
use pgorm::pgorm_query::{ArrayType, ColumnType, Nullable, ValueType};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonValueType)]
struct Meta {
    tags: Vec<String>,
}

/// A JSON object key must be a string; a tuple key is one `serde_json` refuses,
/// so `to_value` on this type always fails.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonValueType)]
struct Unserializable {
    grid: BTreeMap<(i32, i32), i32>,
}

// [spec:pgorm:sem:macros.derive.from-query-result+3/test]    the generated impls
#[test]
fn the_generated_json_impls() {
    let value = pgorm::Value::from(Meta {
        tags: vec!["a".to_owned()],
    });
    match &value {
        pgorm::Value::Json(Some(json)) => assert_eq!(json["tags"][0], "a"),
        other => panic!("expected Value::Json, got {other:?}"),
    }

    assert_eq!(
        <Meta as ValueType>::try_from(value).expect("round trips"),
        Meta {
            tags: vec!["a".to_owned()]
        }
    );
    assert!(<Meta as ValueType>::try_from(pgorm::Value::Json(None)).is_err());
    assert_eq!(<Meta as ValueType>::type_name(), "Meta");
    assert_eq!(<Meta as ValueType>::column_type(), ColumnType::Json);
    assert_eq!(<Meta as ValueType>::array_type(), ArrayType::Json);
    assert_eq!(<Meta as Nullable>::null(), pgorm::Value::Json(None));
}

// [spec:pgorm:sem:macros.derive.from-query-result+3/test]    a serialisation
// failure panics naming the type instead of binding SQL NULL
#[test]
#[should_panic(expected = "`Unserializable` could not be serialized to JSON")]
fn a_serialisation_failure_panics_rather_than_binding_null() {
    let mut grid = BTreeMap::new();
    grid.insert((0, 0), 1);
    let _ = pgorm::Value::from(Unserializable { grid });
}
