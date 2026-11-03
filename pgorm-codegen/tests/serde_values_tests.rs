//! Interval, range and multirange columns under each `with_serde` mode: the
//! entity and range newtypes the writer generates for every one of them,
//! compiled here as well as compared, so the serde derives it emits are
//! derives those field types satisfy.

mod common;

use std::ops::Bound::{Excluded, Included, Unbounded};

use common::*;
use pgorm::entity::prelude::{
    Date, DateTime, DateTimeWithTimeZone, Decimal, Interval, Multirange, Range,
};
use pgorm::{ActiveModelTrait, ActiveValue, Iterable, ModelTrait};
use pgorm_codegen::{WithSerde, entities_from_sql};
use serde_json::{Value, json};

const SCHEMA: &str = include_str!("sql/serde_values/schema.sql");

/// The two files the writer generates for [`SCHEMA`] under one mode, kept as
/// written: included rather than declared as modules, so rustfmt leaves them
/// in the order the writer emits.
macro_rules! generated_under {
    ($mode:ident) => {
        mod $mode {
            pub mod pgorm_range_types {
                include!(concat!(
                    "sql/serde_values/",
                    stringify!($mode),
                    "/pgorm_range_types.rs"
                ));
            }
            pub mod ledger {
                include!(concat!(
                    "sql/serde_values/",
                    stringify!($mode),
                    "/ledger.rs"
                ));
            }
        }
    };
}

generated_under!(none);
generated_under!(serialize);
generated_under!(deserialize);
generated_under!(both);

/// One row of the ledger in a mode's generated types: the interval
/// infinities, an empty range, unbounded sides, an empty multirange and one
/// holding an empty range.
macro_rules! ledger {
    ($mode:ident) => {{
        use $mode::pgorm_range_types::{Floatrange, Slot, SlotMultirange};
        $mode::ledger::Model {
            id: 1,
            span: Interval::new(i32::MAX, i32::MAX, i64::MAX),
            maybe_span: None,
            months: Interval::new(14, 0, 0),
            spans: vec![
                Interval::new(1, -2, 3_000_004),
                Interval::new(i32::MIN, i32::MIN, i64::MIN),
            ],
            ints: Range::from(1..5),
            bigs: None,
            nums: Range::new(Included(Decimal::new(150, 2)), Unbounded),
            days: Range::Empty,
            stamps: Range::from(..=DateTime::constant(2024, 1, 1, 10, 0, 0, 0)),
            instants: Some(Range::from(
                DateTimeWithTimeZone::constant(1_704_103_200, 0)..,
            )),
            int_set: [Range::from(5..8), Range::Empty, Range::from(1..3)]
                .into_iter()
                .collect(),
            big_set: Some(Multirange::default()),
            num_set: Multirange::from(vec![Range::from(Decimal::new(1, 0)..Decimal::new(2, 0))]),
            day_set: Multirange::from(vec![Range::from(
                Date::constant(2024, 1, 1)..Date::constant(2024, 2, 1),
            )]),
            stamp_set: Multirange::default(),
            instant_set: None,
            int_ranges: vec![Range::Empty, Range::from(..)],
            day_sets: Some(vec![Multirange::default()]),
            float_set: None,
            floats: Floatrange(Range::new(Included(0.5), Excluded(2.5))),
            slot: Some(Slot(Range::from(..))),
            slots: SlotMultirange(Multirange::from(vec![Range::from(1..3)])),
        }
    }};
}

// [spec:pgorm:def:codegen.entity.serde+1/test]    each mode's import heads
// the entity and the range newtypes' file alike
// [spec:pgorm:sem:codegen.entity.serde.derives+3/test]    and its derives
// reach the Model and every range newtype
// [spec:pgorm:sem:codegen.entity.imports+4/test]
#[test]
fn each_serde_mode_generates_the_compiled_fixtures() {
    for (mode, ledger, range_types) in [
        (
            "none",
            include_str!("sql/serde_values/none/ledger.rs"),
            include_str!("sql/serde_values/none/pgorm_range_types.rs"),
        ),
        (
            "serialize",
            include_str!("sql/serde_values/serialize/ledger.rs"),
            include_str!("sql/serde_values/serialize/pgorm_range_types.rs"),
        ),
        (
            "deserialize",
            include_str!("sql/serde_values/deserialize/ledger.rs"),
            include_str!("sql/serde_values/deserialize/pgorm_range_types.rs"),
        ),
        (
            "both",
            include_str!("sql/serde_values/both/ledger.rs"),
            include_str!("sql/serde_values/both/pgorm_range_types.rs"),
        ),
    ] {
        let with_serde: WithSerde = mode.parse().expect("a mode name");
        let output = entities_from_sql(
            SCHEMA,
            Opts {
                with_serde,
                ..Default::default()
            },
        )
        .expect("the schema generates");
        let generated = Generated {
            files: output
                .files
                .into_iter()
                .map(|file| (file.name, file.content))
                .collect(),
        };
        for (file, fixture) in [("ledger.rs", ledger), ("pgorm_range_types.rs", range_types)] {
            let (header, written) = generated
                .file(file)
                .split_once('\n')
                .expect("a header line");
            assert!(header.starts_with("//! `pgorm` Entity"), "{mode} {file}");
            assert_eq!(norm(written), norm(fixture), "{mode} {file}");
        }
    }
}

// [spec:pgorm:def:sql.value.interval+2/test]    a generated Model serializes
// an interval as its three fields and reads them back
// [spec:pgorm:def:sql.value.range+5/test]    and a range, a multirange and a
// created range's newtype in the ranges' enum form
#[test]
fn a_generated_model_round_trips_through_json() -> Result<(), serde_json::Error> {
    let model = ledger!(both);
    let json = serde_json::to_value(&model)?;

    assert_eq!(
        json["span"],
        json!({"months": i32::MAX, "days": i32::MAX, "microseconds": i64::MAX})
    );
    assert_eq!(json["maybe_span"], Value::Null);
    assert_eq!(
        json["ints"],
        json!({"Bounds": {"lower": {"Included": 1}, "upper": {"Excluded": 5}}})
    );
    assert_eq!(
        json["nums"],
        json!({"Bounds": {"lower": {"Included": "1.50"}, "upper": "Unbounded"}})
    );
    assert_eq!(json["days"], json!("Empty"));
    assert_eq!(
        json["stamps"],
        json!({"Bounds": {"lower": "Unbounded", "upper": {"Included": "2024-01-01T10:00:00"}}})
    );
    assert_eq!(
        json["int_set"],
        json!([
            {"Bounds": {"lower": {"Included": 5}, "upper": {"Excluded": 8}}},
            "Empty",
            {"Bounds": {"lower": {"Included": 1}, "upper": {"Excluded": 3}}},
        ])
    );
    assert_eq!(json["big_set"], json!([]));
    assert_eq!(
        json["floats"],
        json!({"Bounds": {"lower": {"Included": 0.5}, "upper": {"Excluded": 2.5}}})
    );
    assert_eq!(
        json["slot"],
        json!({"Bounds": {"lower": "Unbounded", "upper": "Unbounded"}})
    );
    assert_eq!(
        serde_json::from_value::<both::ledger::Model>(json.clone())?,
        model
    );

    // The one-sided modes write and read the same text.
    assert_eq!(serde_json::to_value(ledger!(serialize))?, json);
    let read: deserialize::ledger::Model = serde_json::from_value(json)?;
    assert_eq!(format!("{read:?}"), format!("{model:?}"));
    assert_eq!(format!("{:?}", ledger!(none)), format!("{model:?}"));
    Ok(())
}

// [spec:pgorm:req:entity.active-model.json+4/test]    `from_json` sets every
// interval, range and multirange column of a generated entity to the value
// the Model serialized
#[test]
fn from_json_sets_each_interval_and_range_column() -> Result<(), pgorm::Error> {
    let model = ledger!(both);
    let json = serde_json::to_value(&model).expect("the model serializes");
    let active = both::ledger::ActiveModel::from_json(json)?;
    for column in both::ledger::Column::iter() {
        assert_eq!(active.get(column)?, ActiveValue::Set(model.get(column)));
    }
    Ok(())
}
