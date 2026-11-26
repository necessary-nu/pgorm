//! Encoder tests, written against the payloads `pgorm_campaign.wire` accepts.
//!
//! Every expected document here has been checked against the Python source of
//! truth by feeding it to `wire.validate`; the assertions below pin the exact
//! text so a drift in jiff's `Display`, in `Decimal`'s scale handling or in
//! the float bit formatting fails here rather than in the oracle.

use std::error::Error;

use jiff::{Timestamp, civil::DateTime, tz::TimeZone};
use pgorm::pgorm_query::{ArrayType, Interval, MacAddress, Value, Vector, tests_cfg::Variants};
use rust_decimal::Decimal;
use serde_json::{Value as Json, json};

use crate::{
    ObservedError, Report, entities, observe,
    wire::{Tag, Tagged, TypeName, parse_datetime_utc, parse_naive_datetime},
    wire::{parse_date, parse_time, temporal_text, validate},
};

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

fn encoded(value: Value) -> Result<Json, Box<dyn Error + Send + Sync>> {
    Ok(Tagged::from_value(value).encode_checked()?)
}

fn text(value: &str) -> Value {
    Value::String(Some(Box::new(value.to_owned())))
}

fn moment() -> DateTime {
    jiff::civil::datetime(2024, 1, 2, 3, 4, 5, 123_456_000)
}

fn instant() -> Result<Timestamp, Box<dyn Error + Send + Sync>> {
    Ok(TimeZone::UTC.to_timestamp(moment())?)
}

#[test]
fn typed_null_keeps_its_kind_without_payload() -> TestResult {
    assert_eq!(
        encoded(Value::Int(None))?,
        json!({"version": 1, "type": {"kind": "i32"}, "sql_null": true, "data": Json::Null})
    );
    assert_eq!(
        encoded(Value::Array(ArrayType::String, None))?,
        json!({
            "version": 1,
            "type": {"kind": "array", "element": {"kind": "text"}},
            "sql_null": true,
            "data": Json::Null,
        })
    );
    Ok(())
}

#[test]
fn sql_null_and_json_null_stay_distinct() -> TestResult {
    let absent = encoded(Value::Json(None))?;
    let present = encoded(Value::Json(Some(Box::new(Json::Null))))?;
    assert_eq!(absent["sql_null"], json!(true));
    assert_eq!(present["sql_null"], json!(false));
    assert_eq!(absent["data"], present["data"]);
    assert_eq!(absent["type"], present["type"]);
    Ok(())
}

#[test]
fn an_empty_array_is_a_present_value() -> TestResult {
    assert_eq!(
        encoded(Value::Array(ArrayType::String, Some(Box::new(Vec::new()))))?,
        json!({
            "version": 1,
            "type": {"kind": "array", "element": {"kind": "text"}},
            "sql_null": false,
            "data": [],
        })
    );
    Ok(())
}

#[test]
fn array_elements_keep_their_own_null_flags() -> TestResult {
    let values = vec![text("%_"), Value::String(None)];
    assert_eq!(
        encoded(Value::Array(ArrayType::String, Some(Box::new(values))))?,
        json!({
            "version": 1,
            "type": {"kind": "array", "element": {"kind": "text"}},
            "sql_null": false,
            "data": [
                {"version": 1, "type": {"kind": "text"}, "sql_null": false, "data": "%_"},
                {"version": 1, "type": {"kind": "text"}, "sql_null": true, "data": Json::Null},
            ],
        })
    );
    Ok(())
}

#[test]
fn floats_travel_as_ieee_bits() -> TestResult {
    for (value, bits) in [
        (0.0f32, "00000000"),
        (-0.0f32, "80000000"),
        (f32::NAN, "7fc00000"),
        (f32::INFINITY, "7f800000"),
        (1.0f32, "3f800000"),
    ] {
        assert_eq!(encoded(Value::Float(Some(value)))?["data"], json!(bits));
    }
    for (value, bits) in [
        (-0.0f64, "8000000000000000"),
        (f64::NAN, "7ff8000000000000"),
        (f64::NEG_INFINITY, "fff0000000000000"),
        (0.1f64, "3fb999999999999a"),
    ] {
        assert_eq!(encoded(Value::Double(Some(value)))?["data"], json!(bits));
    }
    Ok(())
}

#[test]
fn a_nan_payload_survives_the_bit_pattern() -> TestResult {
    // Signalling NaN: the reason floats are not encoded as JSON numbers.
    let signalling = f32::from_bits(0x7f80_0001);
    assert!(signalling.is_nan());
    assert_eq!(
        encoded(Value::Float(Some(signalling)))?["data"],
        json!("7f800001")
    );
    Ok(())
}

#[test]
fn a_vector_is_a_list_of_bit_patterns() -> TestResult {
    let vector = Vector::from(vec![-0.0f32, 1.0f32]);
    assert_eq!(
        encoded(Value::Vector(Some(Box::new(vector))))?["data"],
        json!(["80000000", "3f800000"])
    );
    Ok(())
}

#[test]
fn integers_travel_as_canonical_decimal_text() -> TestResult {
    assert_eq!(encoded(Value::TinyInt(Some(-128)))?["data"], json!("-128"));
    assert_eq!(
        encoded(Value::SmallInt(Some(i16::MIN)))?["data"],
        json!("-32768")
    );
    assert_eq!(encoded(Value::Int(Some(0)))?["data"], json!("0"));
    assert_eq!(
        encoded(Value::BigInt(Some(i64::MIN)))?["data"],
        json!("-9223372036854775808")
    );
    assert_eq!(
        encoded(Value::BigUnsigned(Some(u64::MAX)))?["data"],
        json!("18446744073709551615")
    );
    assert_eq!(
        encoded(Value::Unsigned(Some(u32::MAX)))?["data"],
        json!("4294967295")
    );
    Ok(())
}

#[test]
fn a_decimal_keeps_its_trailing_scale() -> TestResult {
    for input in [
        "123.4500",
        "0.0000",
        "-0.0001",
        "79228162514264337593543950335",
    ] {
        let value = Decimal::from_str_exact(input)?;
        assert_eq!(
            encoded(Value::Decimal(Some(Box::new(value))))?["data"],
            json!(input)
        );
    }
    Ok(())
}

#[test]
fn bytes_and_mac_addresses_travel_as_integer_arrays() -> TestResult {
    assert_eq!(
        encoded(Value::Bytes(Some(Box::new(vec![0, 127, 255]))))?,
        json!({
            "version": 1,
            "type": {"kind": "bytes"},
            "sql_null": false,
            "data": [0, 127, 255],
        })
    );
    let mac = MacAddress::new([0x01, 0x02, 0x03, 0xfd, 0xfe, 0xff]);
    assert_eq!(
        encoded(Value::MacAddress(Some(Box::new(mac))))?["data"],
        json!([1, 2, 3, 253, 254, 255])
    );
    Ok(())
}

#[test]
fn text_and_char_carry_hostile_content_unchanged() -> TestResult {
    assert_eq!(
        encoded(text("O'Brien \" 雪\\\0"))?["data"],
        json!("O'Brien \" 雪\\\0")
    );
    assert_eq!(encoded(Value::Char(Some('雪')))?["data"], json!("雪"));
    Ok(())
}

#[test]
fn every_temporal_kind_matches_the_python_normalisation() -> TestResult {
    let naive = moment();
    let cases: Vec<(Value, &str, &str)> = vec![
        (
            Value::Date(Some(Box::new(naive.date()))),
            "date",
            "2024-01-02",
        ),
        (
            Value::Time(Some(Box::new(naive.time()))),
            "time",
            "03:04:05.123456",
        ),
        (
            Value::DateTime(Some(Box::new(naive))),
            "datetime",
            "2024-01-02T03:04:05.123456",
        ),
        (
            Value::DateTimeWithTimeZone(Some(Box::new(instant()?))),
            "datetime_utc",
            "2024-01-02T03:04:05.123456Z",
        ),
    ];
    for (value, kind, rendered) in cases {
        let document = encoded(value)?;
        assert_eq!(document["type"], json!({"kind": kind}), "{kind}");
        assert_eq!(document["data"], json!(rendered), "{kind}");
    }
    Ok(())
}

#[test]
fn temporal_text_reads_what_jiff_prints() -> TestResult {
    for (printed, normalized) in [
        ("2024-01-02", "2024-01-02"),
        ("03:04:05.123456", "03:04:05.123456"),
        // jiff writes the fewest fractional digits that round-trip; Python
        // renders exactly six, or none at all.
        ("03:04:05.123", "03:04:05.123000"),
        ("03:04:05.12", "03:04:05.120000"),
        // An all-zero fraction is dropped, not padded.
        ("03:04:05.000000", "03:04:05"),
        (
            "2024-01-02T03:04:05.123456Z",
            "2024-01-02 03:04:05.123456+00:00",
        ),
        ("2024-01-02T03:04:05", "2024-01-02 03:04:05"),
    ] {
        assert_eq!(temporal_text(printed)?, normalized, "{printed}");
    }
    assert!(temporal_text("03:04:05.123456789").is_err());
    Ok(())
}

#[test]
fn temporal_parsers_read_back_what_the_encoder_wrote() -> TestResult {
    let naive = moment();
    let instant = instant()?;
    assert_eq!(parse_date(&naive.date().to_string())?, naive.date());
    assert_eq!(parse_time(&naive.time().to_string())?, naive.time());
    assert_eq!(parse_naive_datetime(&naive.to_string())?, naive);
    assert_eq!(parse_datetime_utc(&instant.to_string())?, instant);
    // The payload is native `Display` output, so the parsers only have a
    // source of truth to read back because jiff reads what it writes.
    assert_eq!(naive.to_string().parse::<DateTime>()?, naive);
    assert_eq!(instant.to_string().parse::<Timestamp>()?, instant);
    Ok(())
}

#[test]
fn a_datetime_kind_must_agree_with_its_offset() {
    assert!(parse_naive_datetime("2024-01-02 03:04:05+00:00").is_err());
    assert!(parse_datetime_utc("2024-01-02 03:04:05").is_err());
    assert!(parse_datetime_utc("2024-01-02 03:04:05+05:30").is_err());
    assert!(parse_time("03:04:05+00:00").is_err());
    assert!(parse_datetime_utc("2024-01-02T03:04:05Z").is_ok());
}

#[test]
fn an_enum_carries_its_qualified_identity() -> TestResult {
    let name = TypeName::new("State\" 雪").in_schema("fixture");
    let label = Tagged::from_enum(text("O'Brien 雪"), name.clone(), false);
    assert_eq!(
        label.encode_checked()?,
        json!({
            "version": 1,
            "type": {"kind": "enum", "name": "State\" 雪", "schema": "fixture"},
            "sql_null": false,
            "data": "O'Brien 雪",
        })
    );

    let values = vec![text("calm"), Value::String(None)];
    let array = Tagged::from_enum(
        Value::Array(ArrayType::String, Some(Box::new(values))),
        name,
        true,
    );
    let document = array.encode_checked()?;
    assert_eq!(
        document["type"],
        json!({
            "kind": "array",
            "element": {"kind": "enum", "name": "State\" 雪", "schema": "fixture"},
        })
    );
    // The element tag is the enum's, not the `text` the payload alone implies.
    assert_eq!(document["data"][0]["type"], document["type"]["element"]);
    assert_eq!(document["data"][1]["sql_null"], json!(true));
    Ok(())
}

#[test]
fn an_unqualified_enum_records_a_null_schema() -> TestResult {
    let label = Tagged::from_enum(text("calm"), TypeName::new("mood"), false);
    assert_eq!(
        label.encode_checked()?["type"],
        json!({"kind": "enum", "name": "mood", "schema": Json::Null})
    );
    Ok(())
}

#[test]
fn validation_rejects_what_the_python_format_rejects() {
    let malformed = [
        json!({"version": 2, "type": {"kind": "i32"}, "sql_null": true, "data": Json::Null}),
        json!({"version": 1, "type": {"kind": "i32"}, "sql_null": true, "data": "0"}),
        json!({"version": 1, "type": {"kind": "i32"}, "sql_null": false, "data": 0}),
        json!({"version": 1, "type": {"kind": "i8"}, "sql_null": false, "data": "128"}),
        json!({"version": 1, "type": {"kind": "i32"}, "sql_null": false, "data": "-0"}),
        json!({"version": 1, "type": {"kind": "f32"}, "sql_null": false, "data": "7FC00000"}),
        json!({"version": 1, "type": {"kind": "f32"}, "sql_null": false, "data": "7fc0000"}),
        json!({"version": 1, "type": {"kind": "char"}, "sql_null": false, "data": "ab"}),
        json!({"version": 1, "type": {"kind": "bytes"}, "sql_null": false, "data": [256]}),
        json!({"version": 1, "type": {"kind": "mac_address"}, "sql_null": false, "data": [1, 2]}),
        json!({"version": 1, "type": {"kind": "decimal"}, "sql_null": false, "data": "01.5"}),
        json!({"version": 1, "type": {"kind": "decimal"}, "sql_null": false, "data": "1."}),
        json!({"version": 1, "type": {"kind": "uuid"}, "sql_null": false,
               "data": "00000000-0000-0000-0000-00000000000A"}),
        json!({"version": 1, "type": {"kind": "ipnetwork"}, "sql_null": false, "data": "10.0.0.1"}),
        json!({"version": 1, "type": {"kind": "date"}, "sql_null": false, "data": "2024-1-2"}),
        json!({"version": 1, "type": {"kind": "unknown"}, "sql_null": true, "data": Json::Null}),
        json!({
            "version": 1,
            "type": {"kind": "array", "element": {"kind": "array", "element": {"kind": "i32"}}},
            "sql_null": true,
            "data": Json::Null,
        }),
        // An element whose tag differs from the one the array declares.
        json!({
            "version": 1,
            "type": {"kind": "array", "element": {"kind": "text"}},
            "sql_null": false,
            "data": [{"version": 1, "type": {"kind": "i32"}, "sql_null": false, "data": "1"}],
        }),
        // An enum name past PostgreSQL's identifier limit.
        json!({
            "version": 1,
            "type": {"kind": "enum", "name": "n".repeat(64), "schema": Json::Null},
            "sql_null": true,
            "data": Json::Null,
        }),
    ];
    for document in malformed {
        assert!(validate(&document).is_err(), "{document}");
    }
}

#[test]
fn validation_accepts_the_canonical_spellings() -> TestResult {
    let well_formed = [
        json!({"version": 1, "type": {"kind": "uuid"}, "sql_null": false,
               "data": "00000000-0000-0000-0000-000000000001"}),
        json!({"version": 1, "type": {"kind": "ipnetwork"}, "sql_null": false,
               "data": "10.0.0.1/24"}),
        json!({"version": 1, "type": {"kind": "json"}, "sql_null": false,
               "data": {"owner": "O'Brien 雪", "null": Json::Null}}),
        json!({"version": 1, "type": {"kind": "bool"}, "sql_null": false, "data": false}),
        json!({"version": 1, "type": {"kind": "vector"}, "sql_null": false,
               "data": ["80000000"]}),
    ];
    for document in well_formed {
        validate(&document)?;
    }
    Ok(())
}

#[test]
fn a_tag_names_its_kind() -> TestResult {
    assert_eq!(Tag::Scalar(ArrayType::Decimal).name(), "decimal");
    assert_eq!(Tag::Enum(TypeName::new("mood")).name(), "enum");
    assert_eq!(Tag::Scalar(ArrayType::Int).array().name(), "array");
    Ok(())
}

#[test]
fn observations_take_the_shapes_the_python_effects_produce() {
    assert_eq!(observe::count(3), json!({"kind": "count", "value": 3}));
    assert_eq!(observe::unit(), json!({"kind": "unit"}));
    assert_eq!(observe::absent(), json!({"kind": "absent"}));
    assert_eq!(
        observe::transaction("t0"),
        json!({"kind": "transaction", "scope": "t0"})
    );
    assert_eq!(
        observe::tuple(vec![observe::absent()]),
        json!({"kind": "tuple", "items": [{"kind": "absent"}]})
    );
    assert_eq!(
        observe::error(&ObservedError::new(
            "DatabaseError",
            "relation does not exist",
            Some("42P01".to_owned()),
        )),
        json!({
            "kind": "error",
            "class": "DatabaseError",
            "cause": "relation does not exist",
            "sqlstate": "42P01",
        })
    );
}

#[test]
fn a_report_is_executed_until_a_step_fails() {
    let mut report = Report::new("0".repeat(64).as_str());
    assert!(report.is_empty());
    report.observed(
        "s0",
        "fetch",
        &["pgorm::pipeline::Pipeline::filter"],
        observe::count(1),
    );
    assert_eq!(report.status(), "executed");
    report.failed(
        "s1",
        "execute",
        &["pgorm::Connection::execute_raw"],
        &ObservedError::new("DecodeError", "column 0", None),
    );
    assert_eq!(report.len(), 2);
    assert_eq!(report.status(), "error");

    let document = report.to_json();
    assert_eq!(document["status"], json!("error"));
    assert_eq!(document["builds"], json!(0));
    assert_eq!(document["cleanup_errors"], json!([]));
    assert_eq!(document["steps"][0]["status"], json!("observed"));
    assert_eq!(document["steps"][0]["operation"], json!("fetch"));
    assert_eq!(document["steps"][1]["operation"], json!("execute"));
    assert_eq!(
        document["steps"][0]["native_paths"],
        json!(["pgorm::pipeline::Pipeline::filter"])
    );
    assert_eq!(
        document["steps"][1]["observation"]["class"],
        json!("DecodeError")
    );
    assert_eq!(document["steps"][1]["observation"]["sqlstate"], Json::Null);
}

#[test]
fn redaction_removes_connection_secrets_from_diagnostics() -> TestResult {
    let config: pgorm::Config = "postgres://campaign:hunter2@localhost/subject".parse()?;
    let redactions = crate::Redactions::from_config(&config);
    assert_eq!(
        redactions.apply("role campaign could not authenticate with hunter2"),
        "role [redacted] could not authenticate with [redacted]"
    );
    Ok(())
}

#[test]
fn a_utc_instant_round_trips_through_the_canon() -> TestResult {
    let instant = instant()?;
    let normalized = temporal_text(&instant.to_string())?;
    assert_eq!(normalized, "2024-01-02 03:04:05.123456+00:00");
    assert_eq!(parse_datetime_utc(&normalized)?, instant);
    // The canon is what the oracle compares, so normalising it must be a fixed
    // point: a value re-read from an artifact has to land on the same text.
    assert_eq!(temporal_text(&normalized)?, normalized);
    Ok(())
}

#[test]
fn the_copied_entity_keeps_its_hostile_enum() {
    use pgorm::{Column, Entity, QueryFilter, Statement};

    let (sql, values) = entities::account::Entity::find()
        .filter(entities::account::Column::State.eq(entities::account::State::Busy))
        .build();
    assert!(sql.contains(r#""fixture"."accounts""#), "{sql}");
    // The enum name carries a quote and a non-ASCII character; both have to
    // reach the cast through the identifier quoter rather than as raw text.
    assert!(sql.contains(r#"AS fixture."State"" 雪""#), "{sql}");
    assert_eq!(values.0.len(), 1);
}

fn sample_account() -> Result<entities::account::Model, Box<dyn Error + Send + Sync>> {
    Ok(entities::account::Model {
        id: 1,
        tenant: 1,
        name: "Alice".to_owned(),
        note: None,
        score: Some(10),
        rank: 1,
        active: true,
        balance: Decimal::new(1_234_500, 4),
        payload: json!({"owner": "Alice"}),
        uuid: pgorm::entity::prelude::Uuid::from_u128(1),
        created_at: moment(),
        occurred_at: instant()?,
        event_date: moment().date(),
        event_time: moment().time(),
        state: entities::account::State::Busy,
    })
}

#[test]
fn a_model_observation_carries_no_output_identity() -> TestResult {
    let observed =
        observe::model::<entities::account::Entity>(&sample_account()?, "campaign.Account")?;
    assert_eq!(observed["kind"], json!("record"));
    assert_eq!(observed["entity"], json!("campaign.Account"));
    // `EntityModel` exposes no PostgreSQL fields, so `observations.row` emits
    // no `postgres` key for one and neither may this.
    assert_eq!(observed.get("postgres"), None);
    let names: Vec<_> = observed["fields"]
        .as_array()
        .ok_or("fields is not an array")?
        .iter()
        .map(|field| field["name"].clone())
        .collect();
    // Declaration order, which is what the binding's `keys()` answers with.
    assert_eq!(names[0], json!("id"));
    assert_eq!(names[14], json!("state"));
    assert_eq!(names.len(), 15);
    Ok(())
}

#[test]
fn a_model_enum_keeps_its_qualified_identity() -> TestResult {
    let observed =
        observe::model::<entities::account::Entity>(&sample_account()?, "campaign.Account")?;
    let state = &observed["fields"][14]["value"];
    assert_eq!(
        state["type"],
        json!({"kind": "enum", "name": "State\" 雪", "schema": "fixture"})
    );
    assert_eq!(state["data"], json!("O'Brien 雪"));
    // A text column beside it stays text; only the tag tells the two apart.
    assert_eq!(observed["fields"][2]["type"], Json::Null);
    assert_eq!(
        observed["fields"][2]["value"]["type"],
        json!({"kind": "text"})
    );
    Ok(())
}

#[test]
fn an_absent_slot_observes_as_absent() -> TestResult {
    assert_eq!(
        observe::maybe::<entities::note::Entity>(None, "campaign.Note"),
        Ok(observe::absent())
    );
    let rows = observe::collected(vec![observe::absent()]);
    assert_eq!(rows, json!({"kind": "rows", "rows": [{"kind": "absent"}]}));
    Ok(())
}

#[test]
fn the_copied_graphs_quote_their_given_aliases() {
    use pgorm::Statement;

    let aliases = vec!["n\" 雪".to_owned()];
    let (sql, _) = entities::graphs::optional(&aliases).build();
    assert!(sql.contains(r#""n"" 雪""#), "{sql}");
    assert!(sql.contains("LEFT JOIN"), "{sql}");

    let (sql, _) = entities::graphs::required(&aliases).build();
    assert!(sql.contains("INNER JOIN"), "{sql}");

    let many = (0..6).map(|n| format!("n{n}")).collect::<Vec<_>>();
    let (sql, _) = entities::graphs::arity7(&many).build();
    assert_eq!(sql.matches("LEFT JOIN").count(), 6, "{sql}");
}

fn days(from: (i8, i8), to: (i8, i8)) -> pgorm::pgorm_query::Range<jiff::civil::Date> {
    pgorm::pgorm_query::Range::from(
        jiff::civil::date(2026, from.0, from.1)..jiff::civil::date(2026, to.0, to.1),
    )
}

// [spec:pgorm:req:generative.replay/test]    a temporal key's period observes as pgorm's own
// range snapshot, which the Python wire accepts
#[test]
fn room_period_observes_as_a_range() -> TestResult {
    let version = entities::room::Model {
        id: 1,
        valid_at: days((1, 1), (2, 1)),
        rate: 100,
    };
    let observed = observe::model::<entities::room::Entity>(&version, "campaign.Room")?;
    assert_eq!(
        observed["fields"][1]["value"],
        json!({
            "version": 1,
            "type": {"kind": "daterange"},
            "sql_null": false,
            "data": {"lower": "2026-01-01", "upper": "2026-02-01", "bounds": "[)"},
        })
    );
    let empty = Value::Range(
        pgorm::pgorm_query::RangeType::Int4,
        Some(Box::new(pgorm::pgorm_query::Range::Empty)),
    );
    assert_eq!(encoded(empty)?["data"], json!({"empty": true}));
    let open = Value::Range(
        pgorm::pgorm_query::RangeType::Int4,
        Some(Box::new(pgorm::pgorm_query::Range::new(
            std::ops::Bound::Unbounded,
            std::ops::Bound::Included(Value::Int(Some(5))),
        ))),
    );
    assert_eq!(
        encoded(open)?["data"],
        json!({"lower": Json::Null, "upper": "5", "bounds": "(]"})
    );
    Ok(())
}

// [spec:pgorm:req:generative.replay/test]
#[test]
fn range_validation_rejects_what_the_python_format_rejects() {
    let range = |data: Json| json!({"version": 1, "type": {"kind": "daterange"}, "sql_null": false, "data": data});
    for data in [
        json!({"lower": Json::Null, "upper": "2026-02-01", "bounds": "[)"}),
        json!({"lower": "2026-1-1", "upper": Json::Null, "bounds": "[)"}),
        json!({"lower": "2026-01-01", "upper": "2026-02-01", "bounds": "[["}),
        json!({"empty": false}),
        json!({"lower": "2026-01-01", "upper": "2026-02-01"}),
    ] {
        assert!(validate(&range(data.clone())).is_err(), "{data}");
    }
    assert!(validate(&range(json!({"empty": true}))).is_ok());
}

// [spec:pgorm:req:generative.replay/test]    the copied temporal graphs join by key and
// overlapping period, and by a key the server does not enforce
#[test]
fn temporal_graphs_join_by_period_and_key() {
    use pgorm::Statement;

    let aliases = vec!["r\" 雪".to_owned()];
    let (sql, _) = entities::graphs::stay_rooms(&aliases).build();
    assert!(sql.contains(r#""during" && "r"" 雪"."valid_at""#), "{sql}");
    assert!(sql.contains("LEFT JOIN"), "{sql}");
    let (sql, _) = entities::graphs::stay_guests(&aliases).build();
    assert!(sql.contains(r#""guest_id" = "r"" 雪"."id""#), "{sql}");
    assert!(!sql.contains("&&"), "{sql}");
}

/// An interval travels as its three fields, each its own decimal text, so a
/// month and thirty days stay two values and an infinity is its triple.
// [spec:pgorm:def:sql.value.interval+3/test]
#[test]
fn an_interval_travels_as_its_three_fields() -> TestResult {
    let interval = |value: Interval| encoded(Value::from(value));
    assert_eq!(
        interval(Interval::new(1, -2, 3_000_004))?,
        json!({
            "version": 1,
            "type": {"kind": "interval"},
            "sql_null": false,
            "data": {"months": "1", "days": "-2", "microseconds": "3000004"},
        })
    );
    assert_ne!(
        interval(Interval::new(1, 0, 0))?,
        interval(Interval::new(0, 30, 0))?
    );
    assert_eq!(
        interval(Interval::new(i32::MAX, i32::MAX, i64::MAX))?["data"],
        json!({"months": "2147483647", "days": "2147483647", "microseconds": "9223372036854775807"})
    );
    assert_eq!(
        interval(Interval::new(i32::MIN, i32::MIN, i64::MIN))?["data"],
        json!({"months": "-2147483648", "days": "-2147483648", "microseconds": "-9223372036854775808"})
    );
    assert_eq!(
        encoded(Value::Interval(None))?,
        json!({"version": 1, "type": {"kind": "interval"}, "sql_null": true, "data": Json::Null})
    );
    let array = encoded(Value::Array(
        ArrayType::Interval,
        Some(Box::new(vec![
            Value::from(Interval::new(0, 0, -1)),
            Value::Interval(None),
        ])),
    ))?;
    assert_eq!(
        array["type"],
        json!({"kind": "array", "element": {"kind": "interval"}})
    );
    assert_eq!(
        array["data"][0]["data"],
        json!({"months": "0", "days": "0", "microseconds": "-1"})
    );
    assert_eq!(array["data"][1]["sql_null"], json!(true));
    Ok(())
}

// [spec:pgorm:def:sql.value.interval+3/test]
#[test]
fn interval_validation_rejects_what_the_python_format_rejects() {
    let interval = |data: Json| json!({"version": 1, "type": {"kind": "interval"}, "sql_null": false, "data": data});
    for data in [
        json!({"months": "1", "days": "0"}),
        json!({"months": "1", "days": "0", "microseconds": "0", "hours": "0"}),
        json!({"months": "2147483648", "days": "0", "microseconds": "0"}),
        json!({"months": "0", "days": "-2147483649", "microseconds": "0"}),
        json!({"months": "0", "days": "0", "microseconds": "9223372036854775808"}),
        json!({"months": "01", "days": "0", "microseconds": "0"}),
        json!({"months": 1, "days": 0, "microseconds": 0}),
        json!("'+1 months'::interval"),
    ] {
        assert!(validate(&interval(data.clone())).is_err(), "{data}");
    }
    let multirange =
        json!({"version": 1, "type": {"kind": "multirange"}, "sql_null": true, "data": Json::Null});
    assert!(validate(&multirange).is_err());
}

// [spec:pgorm:req:generative.replay/test]    every value pgorm holds encodes,
// none reaching the arm for a kind the harness does not represent
#[test]
fn every_value_kind_has_an_encoding() -> TestResult {
    for value in Value::variants() {
        let kind = value.array_type();
        Tagged::from_value(value).encode()?;
        Tagged::from_value(kind.null()).encode()?;
    }
    Ok(())
}
