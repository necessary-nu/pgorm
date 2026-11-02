use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use jiff::Timestamp;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::*;
use crate::oracle::assert_eq;

// [spec:pgorm:def:sql.value+3/test]    every variant wraps an Option; None keeps the type tag
#[test]
fn null_is_typed_none_not_shared_null() {
    let int_null: Value = Option::<i32>::None.into();
    let big_int_null: Value = Option::<i64>::None.into();

    assert_eq!(int_null, Value::Int(None));
    assert_eq!(big_int_null, Value::BigInt(None));
    // Both are SQL NULL, but they are not the same `Value` — the variant carries
    // the type even when the payload is absent.
    assert_ne!(int_null, big_int_null);
    assert_eq!(int_null.to_string(), "NULL");
    assert_eq!(big_int_null.to_string(), "NULL");
}

// [spec:pgorm:def:sql.value+3/test]    payloads larger than a pointer are boxed
#[test]
fn oversized_payloads_boxed_to_keep_enum_small() {
    // `String`, `Vec<u8>`, `serde_json::Value`, `DateTime` and friends are all
    // behind a `Box`, so `Value` stays a tag plus at most one pointer-ish payload.
    assert!(size_of::<Value>() <= 4 * size_of::<usize>());
    assert!(size_of::<Value>() < size_of::<Vec<u8>>() + size_of::<serde_json::Value>());
}

// [spec:pgorm:def:sql.value+3/test]    PartialEq, Eq and Hash, all three agreeing
#[test]
fn value_is_hashable_as_map_key() {
    let mut map: HashMap<Value, &str> = HashMap::new();
    map.insert(Value::Int(Some(1)), "one");
    map.insert(Value::String(Some(Box::new("k".to_owned()))), "kay");
    map.insert(Value::Double(Some(2.5)), "two-point-five");
    map.insert(
        Value::Vector(Some(Box::new(pgvector::Vector::from(vec![1.0, 2.0])))),
        "vector",
    );

    assert_eq!(map.get(&Value::Int(Some(1))), Some(&"one"));
    assert_eq!(
        map.get(&Value::String(Some(Box::new("k".to_owned())))),
        Some(&"kay")
    );
    assert_eq!(map.get(&Value::Double(Some(2.5))), Some(&"two-point-five"));
    assert_eq!(
        map.get(&Value::Vector(Some(Box::new(pgvector::Vector::from(
            vec![1.0, 2.0]
        ))))),
        Some(&"vector")
    );

    let mut null_map: HashMap<Value, &str> = HashMap::new();
    null_map.insert(Value::Float(None), "null float");
    assert_eq!(null_map.get(&Value::Float(None)), Some(&"null float"));
}

// [spec:pgorm:def:sql.value+3/test]    NaN equals itself, so a NaN key is findable
#[test]
fn nan_is_reflexive_and_looks_itself_up() {
    let nan = Value::Double(Some(f64::NAN));
    let also_nan = Value::Double(Some(f64::NAN));

    assert_eq!(nan, also_nan);
    assert_eq!(hash_of(&nan), hash_of(&also_nan));

    let mut map: HashMap<Value, &str> = HashMap::new();
    map.insert(nan, "not a number");
    assert_eq!(map.get(&also_nan), Some(&"not a number"));

    // The same for f32, and for a NaN buried in a vector or an array.
    assert_eq!(Value::Float(Some(f32::NAN)), Value::Float(Some(f32::NAN)));
    assert_eq!(vector(&[f32::NAN, 1.0]), vector(&[f32::NAN, 1.0]));
    assert_eq!(doubles(&[f64::NAN]), doubles(&[f64::NAN]));
}

// [spec:pgorm:def:sql.value+3/test]    `0.0` and `-0.0` are distinct keys, by
// design: PostgreSQL matches them with `=`, so a relation keyed on a float can
// have a row the server matched miss its bucket in a `HashMap<ValueTuple, _>`
#[test]
fn positive_and_negative_zero_are_distinct_keys() {
    let zero = Value::Double(Some(0.0));
    let negative_zero = Value::Double(Some(-0.0));

    assert_ne!(zero, negative_zero);
    assert_ne!(hash_of(&zero), hash_of(&negative_zero));

    let mut map: HashMap<Value, &str> = HashMap::new();
    map.insert(zero.clone(), "positive");
    assert_eq!(map.get(&negative_zero), None);

    map.insert(negative_zero.clone(), "negative");
    assert_eq!(map.get(&zero), Some(&"positive"));
    assert_eq!(map.get(&negative_zero), Some(&"negative"));
    assert_eq!(map.len(), 2);

    assert_ne!(Value::Float(Some(0.0)), Value::Float(Some(-0.0)));
    assert_ne!(vector(&[0.0]), vector(&[-0.0]));
    assert_ne!(doubles(&[0.0]), doubles(&[-0.0]));
}

// [spec:pgorm:def:sql.value+3/test]    equality implies equal hashes across the
// float variants, which is the contract a map key is required to keep
#[test]
fn float_equality_and_hashing_agree() {
    let cases = [
        Value::Double(Some(f64::NAN)),
        Value::Double(Some(0.0)),
        Value::Double(Some(-0.0)),
        Value::Double(Some(2.5)),
        Value::Double(Some(f64::INFINITY)),
        Value::Double(None),
        Value::Float(Some(f32::NAN)),
        Value::Float(Some(0.0)),
        Value::Float(Some(-0.0)),
        Value::Float(None),
        vector(&[f32::NAN, 0.0]),
        vector(&[f32::NAN, -0.0]),
        vector(&[f32::NAN]),
        Value::Vector(None),
        doubles(&[f64::NAN, 0.0]),
        doubles(&[f64::NAN, -0.0]),
    ];

    for a in &cases {
        assert_eq!(a, a);
        for b in &cases {
            if a == b {
                assert_eq!(hash_of(a), hash_of(b), "{a:?} == {b:?} but hashes differ");
            }
        }
    }

    // A shorter vector is not a prefix-collision of a longer one.
    assert_ne!(vector(&[1.0, 2.0]), vector(&[1.0]));
    assert_ne!(hash_of(&vector(&[1.0, 2.0])), hash_of(&vector(&[1.0])));
}

fn hash_of(value: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn vector(elements: &[f32]) -> Value {
    Value::Vector(Some(Box::new(pgvector::Vector::from(elements.to_vec()))))
}

fn doubles(elements: &[f64]) -> Value {
    Value::Array(
        ArrayType::Double,
        Some(Box::new(
            elements.iter().map(|v| Value::Double(Some(*v))).collect(),
        )),
    )
}

// [spec:pgorm:def:sql.value+3/test]    Display renders the Postgres SQL literal
#[test]
fn display_renders_a_postgres_literal() {
    assert_eq!(Value::Bool(Some(true)).to_string(), "TRUE");
    assert_eq!(Value::Bool(Some(false)).to_string(), "FALSE");
    assert_eq!(Value::Int(Some(-7)).to_string(), "-7");
    assert_eq!(
        Value::String(Some(Box::new("it".to_owned()))).to_string(),
        "'it'"
    );
    assert_eq!(
        Value::Bytes(Some(Box::new(vec![0xAB, 0x01]))).to_string(),
        r"'\xAB01'"
    );
    assert_eq!(
        Value::Array(
            ArrayType::Int,
            Some(Box::new(vec![Value::Int(Some(1)), Value::Int(Some(2))]))
        )
        .to_string(),
        "ARRAY [1,2]"
    );
}

// [spec:pgorm:def:sql.render.value-literals+5/test]    an empty array literal
// carries a cast to its element type: PostgreSQL rejects a bare `ARRAY []` with
// "cannot determine type of empty array", there being no element to infer from
#[test]
fn empty_array_literal_names_element_type() {
    assert_eq!(
        Value::array(Vec::<i32>::new()).to_string(),
        "ARRAY []::int4[]"
    );
    assert_eq!(
        Value::array(Vec::<String>::new()).to_string(),
        "ARRAY []::text[]"
    );

    // `Json` binds as either `json` or `jsonb`, so its arrays have no single
    // element type to name and the untypeable spelling is all there is.
    assert_eq!(
        Value::Array(ArrayType::Json, Some(Box::new(vec![]))).to_string(),
        "ARRAY []"
    );

    // A NULL array is still the bare keyword, and a populated one is unchanged.
    assert_eq!(Value::Array(ArrayType::Int, None).to_string(), "NULL");
    assert_eq!(Value::array([1, 2]).to_string(), "ARRAY [1,2]");
}

// [spec:pgorm:def:sql.value.array+6/test]    `Value::array` tags the element
// type from `V`, not from the elements, so an empty list is still typed
#[test]
fn value_array_tags_element_type_from_rust() {
    assert_eq!(
        Value::array([1i32, 2]),
        Value::Array(
            ArrayType::Int,
            Some(Box::new(vec![Value::Int(Some(1)), Value::Int(Some(2))]))
        )
    );
    assert_eq!(
        Value::array(Vec::<Uuid>::new()),
        Value::Array(ArrayType::Uuid, Some(Box::new(vec![])))
    );
}

// [spec:pgorm:def:sql.value+3/test]    the two surviving unsigned variants
#[test]
fn unsigned_variants_carry_oid_and_limit_counts() {
    let oid: Value = 42u32.into();
    let count: Value = 42u64.into();

    assert_eq!(oid, Value::Unsigned(Some(42)));
    assert_eq!(count, Value::BigUnsigned(Some(42)));

    // A `LIMIT`/`OFFSET` count reaches the builder as `BigUnsigned`.
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .limit(10)
            .build(),
        (
            r#"SELECT "id" FROM "glyph" LIMIT $1"#.to_owned(),
            Values(vec![Value::BigUnsigned(Some(10))])
        )
    );
}

// [spec:pgorm:def:sql.value.value-type+4/test]    a `char` holds one character, so
// its column is `char(1)`, not the unbounded `bpchar` of `Char(None)`
#[test]
fn a_char_column_holds_one_character() {
    assert_eq!(
        <char as ValueType>::column_type(),
        ColumnType::Char(Some(1))
    );
}

// [spec:pgorm:def:sql.value.value-type+4/test]    try_from errors on a variant mismatch
#[test]
fn try_from_errors_on_variant_mismatch() {
    assert!(<i32 as ValueType>::try_from(Value::BigInt(Some(1))).is_err());
}

// [spec:pgorm:def:sql.value.value-type+4/test]    try_from errors on a nullability mismatch
#[test]
fn try_from_errors_on_a_null_payload() {
    assert!(<i32 as ValueType>::try_from(Value::Int(None)).is_err());
}

// [spec:pgorm:sem:sql.value.accessor-panics+2/test]    `as_ref_*` yields None for SQL NULL of the
// right variant
#[test]
fn as_ref_accessors_return_none_on_typed_null() {
    assert!(Value::Json(None).as_ref_json().is_none());
    assert!(Value::Date(None).as_ref_date().is_none());
    assert!(Value::Decimal(None).as_ref_decimal().is_none());
    assert!(Value::Uuid(None).as_ref_uuid().is_none());
    assert!(Value::Array(ArrayType::Int, None).as_ref_array().is_none());
    assert!(Value::IpNetwork(None).as_ref_ipnetwork().is_none());
    assert!(Value::MacAddress(None).as_ref_mac_address().is_none());

    let uuid = Uuid::nil();
    assert_eq!(Value::Uuid(Some(Box::new(uuid))).as_ref_uuid(), Some(&uuid));

    assert!(!Value::Int(Some(1)).is_json());
    assert!(Value::Json(None).is_json());
}

// [spec:pgorm:sem:sql.value.accessor-panics+2/test]    None is ambiguous: the wrong variant reads
// the same as a typed NULL, and only the `is_*` discriminator separates them
#[test]
fn as_ref_json_returns_none_on_another_variant() {
    assert!(Value::Int(Some(1)).as_ref_json().is_none());
    assert!(Value::Json(None).as_ref_json().is_none());
    assert!(!Value::Int(Some(1)).is_json());
    assert!(Value::Json(None).is_json());
}

// [spec:pgorm:sem:sql.value.accessor-panics+2/test]
#[test]
fn as_ref_uuid_returns_none_on_another_variant() {
    assert!(Value::Int(Some(1)).as_ref_uuid().is_none());
    assert!(!Value::Int(Some(1)).is_uuid());
}

// [spec:pgorm:sem:sql.value.accessor-panics+2/test]    the accessor stringifies the UTC-naive form
#[test]
fn as_naive_utc_in_string_uses_naive_form() {
    let instant: Timestamp = "2020-01-01T02:02:02+08:00".parse().unwrap();
    assert_eq!(
        Value::DateTimeWithTimeZone(Some(Box::new(instant))).as_naive_utc_in_string(),
        Some("2019-12-31T18:02:02".to_owned())
    );

    let date = jiff::civil::date(2020, 1, 1);
    assert_eq!(
        Value::Date(Some(Box::new(date))).as_naive_utc_in_string(),
        Some("2020-01-01".to_owned())
    );
    assert_eq!(Value::Date(None).as_naive_utc_in_string(), None);
}

// [spec:pgorm:sem:sql.value.accessor-panics+2/test]
#[test]
fn as_naive_utc_none_on_other_variant() {
    assert!(Value::Int(Some(1)).as_naive_utc_in_string().is_none());
}

// [spec:pgorm:sem:sql.value.accessor-panics+2/test]    `as_ipaddr` returns the network address
#[test]
fn as_ipaddr_returns_the_network_address() {
    let net = IpNetwork::from_str("10.1.2.3/32").unwrap();
    assert_eq!(
        Value::IpNetwork(Some(Box::new(net))).as_ipaddr(),
        Some(net.network())
    );
}

// [spec:pgorm:sem:sql.value.accessor-panics+2/test]
#[test]
fn as_ipaddr_returns_none_on_another_variant() {
    assert!(Value::Int(Some(1)).as_ipaddr().is_none());
    assert!(Value::IpNetwork(None).as_ipaddr().is_none());
}

// [spec:pgorm:sem:sql.value.accessor-panics+2/test]    `decimal_to_f64` goes through the payload
#[test]
fn decimal_to_f64_converts_the_payload() {
    let decimal = Decimal::from_str("2.02").unwrap();
    assert_eq!(
        Value::Decimal(Some(Box::new(decimal))).decimal_to_f64(),
        Some(2.02)
    );
    assert_eq!(Value::Decimal(None).decimal_to_f64(), None);
}

// [spec:pgorm:def:sql.render.value-literals+5/test]    a char renders as its whole
// UTF-8 text, quoted and escaped exactly like a one-character string
#[test]
fn char_renders_whole_scalar_not_low_byte() {
    assert_eq!(Value::Char(Some('a')).to_string(), "'a'");
    assert_eq!(Value::Char(Some('é')).to_string(), "'é'");
    assert_eq!(Value::Char(Some('—')).to_string(), "'—'");
    assert_eq!(Value::Char(Some('\'')).to_string(), r"E'\''");
    assert_eq!(
        Value::Char(Some('é')).to_string(),
        Value::String(Some(Box::new("é".to_owned()))).to_string()
    );
}

// [spec:pgorm:def:sql.render.value-literals+5/test]    the char literals the renderer
// emits are ones the PostgreSQL grammar accepts
#[test]
fn char_literals_parse_as_postgres_literals() {
    for (character, expected) in [
        ('a', "SELECT 'a'"),
        ('é', "SELECT 'é'"),
        ('—', "SELECT '—'"),
        ('\'', r"SELECT E'\''"),
    ] {
        assert_eq!(
            Query::select()
                .expr(Value::Char(Some(character)))
                .to_string(),
            expected
        );
    }
}
