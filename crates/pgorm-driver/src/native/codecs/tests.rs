use super::*;
use crate::Column;
use crate::types::{FromSql, ToSql, to_sql_checked};

fn encode(ty: Type, value: &(dyn ToSql + Sync)) -> Result<Value, Error> {
    let codecs = parameter_codecs(std::slice::from_ref(&ty))?;
    let mut values = encode_params(&[ty], &codecs, [value], &mut BytesMut::new())?;
    Ok(values.pop().unwrap())
}

fn row(types: &[Type], values: &[Value]) -> Result<Row, Error> {
    let columns = types
        .iter()
        .enumerate()
        .map(|(index, ty)| Column {
            name: format!("c{index}"),
            table_oid: None,
            column_id: None,
            type_modifier: -1,
            r#type: ty.clone(),
        })
        .collect();
    let statement = Statement::unnamed(Vec::new(), columns);
    let codecs = parameter_codecs(types)?;
    let size = row_size(&codecs, values.iter())?;
    encode_values(statement, &codecs, values.iter(), size)
}

// [spec:pgorm:req:turso.values/test]
#[test]
fn scalar_binary_round_trips() {
    let types = [
        Type::BOOL,
        Type::INT2,
        Type::INT4,
        Type::INT8,
        Type::FLOAT4,
        Type::FLOAT8,
        Type::TEXT,
        Type::VARCHAR,
        Type::BPCHAR,
        Type::BYTEA,
    ];
    let values = [
        encode(Type::BOOL, &true).unwrap(),
        encode(Type::INT2, &i16::MIN).unwrap(),
        encode(Type::INT4, &i32::MAX).unwrap(),
        encode(Type::INT8, &i64::MIN).unwrap(),
        encode(Type::FLOAT4, &f32::MAX).unwrap(),
        encode(Type::FLOAT8, &f64::MIN_POSITIVE).unwrap(),
        encode(Type::TEXT, &"héllo").unwrap(),
        encode(Type::VARCHAR, &"v").unwrap(),
        encode(Type::BPCHAR, &"padded ").unwrap(),
        encode(Type::BYTEA, &&[0u8, 255, 17][..]).unwrap(),
    ];
    let row = row(&types, &values).unwrap();
    assert!(row.get::<_, bool>(0));
    assert_eq!(row.get::<_, i16>(1), i16::MIN);
    assert_eq!(row.get::<_, i32>(2), i32::MAX);
    assert_eq!(row.get::<_, i64>(3), i64::MIN);
    assert_eq!(row.get::<_, f32>(4), f32::MAX);
    assert_eq!(row.get::<_, f64>(5), f64::MIN_POSITIVE);
    assert_eq!(row.get::<_, &str>(6), "héllo");
    assert_eq!(row.get::<_, &str>(7), "v");
    assert_eq!(row.get::<_, &str>(8), "padded ");
    assert_eq!(row.get::<_, &[u8]>(9), &[0, 255, 17]);
}

// [spec:pgorm:req:turso.values/test]
// [spec:pgorm:req:turso.rows/test]
#[test]
fn null_empty_and_owned_rows_are_distinct() {
    let values = [
        encode(Type::TEXT, &None::<String>).unwrap(),
        encode(Type::TEXT, &"").unwrap(),
        encode(Type::BYTEA, &(&[] as &[u8])).unwrap(),
    ];
    let row = row(&[Type::TEXT, Type::TEXT, Type::BYTEA], &values).unwrap();
    let cloned = row.clone();
    drop(values);
    drop(row);
    assert_eq!(cloned.get::<_, Option<&str>>(0), None);
    assert_eq!(cloned.get::<_, Option<&str>>(1), Some(""));
    assert_eq!(cloned.get::<_, Option<&[u8]>>(2), Some(&[][..]));
    assert_eq!(cloned.raw_size_bytes(), 0);
    assert!(cloned.try_get::<_, i32>(1).is_err());
}

// [spec:pgorm:req:turso.values/test]
// [spec:pgorm:req:turso.errors/test]
#[test]
fn invalid_native_values_are_refused() {
    for (ty, value) in [
        (Type::BOOL, Value::from_i64(2)),
        (Type::INT2, Value::from_i64(i64::from(i16::MAX) + 1)),
        (Type::INT4, Value::from_i64(i64::from(i32::MIN) - 1)),
        (Type::INT8, Value::from_f64(1.5)),
        (Type::FLOAT8, Value::from_i64(9_007_199_254_740_993)),
        (Type::FLOAT8, Value::from_i64(i64::MAX)),
        (Type::FLOAT4, Value::from_f64(0.1)),
        (Type::FLOAT8, Value::from_f64(f64::INFINITY)),
        (Type::TEXT, Value::from_i64(1)),
    ] {
        let error = row(&[ty], &[value]).unwrap_err();
        assert!(error.code().is_none());
        assert_eq!(error.to_string(), "error deserializing column 0");
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(encode(Type::FLOAT8, &value).is_err());
        assert!(encode(Type::FLOAT4, &(value as f32)).is_err());
    }
    let exact = row(&[Type::FLOAT8], &[Value::from_i64(i64::MIN)]).unwrap();
    assert_eq!(exact.get::<_, f64>(0), i64::MIN as f64);
    let signed_zero = encode(Type::FLOAT8, &-0.0f64).unwrap();
    assert_eq!(
        row(&[Type::FLOAT8], &[signed_zero])
            .unwrap()
            .get::<_, f64>(0)
            .to_bits(),
        (-0.0f64).to_bits()
    );
}

#[derive(Debug)]
struct Custom {
    bytes: &'static [u8],
    text: bool,
    null: bool,
}

impl ToSql for Custom {
    fn to_sql(&self, _: &Type, out: &mut BytesMut) -> Result<IsNull, Cause> {
        out.extend_from_slice(self.bytes);
        Ok(if self.null { IsNull::Yes } else { IsNull::No })
    }
    fn accepts(_: &Type) -> bool {
        true
    }
    fn encode_format(&self, _: &Type) -> Format {
        if self.text {
            Format::Text
        } else {
            Format::Binary
        }
    }
    to_sql_checked!();
}

// [spec:pgorm:req:turso.values/test]
// [spec:pgorm:req:turso.errors/test]
#[test]
fn custom_codecs_are_checked_without_trial_encoding() {
    let value = Custom {
        bytes: &[0, 0, 0, 42],
        text: false,
        null: false,
    };
    let encoded = encode(Type::INT4, &value).unwrap();
    assert_eq!(row(&[Type::INT4], &[encoded]).unwrap().get::<_, i32>(0), 42);
    for (ty, value) in [
        (
            Type::INT4,
            Custom {
                bytes: &[1],
                text: false,
                null: false,
            },
        ),
        (
            Type::BOOL,
            Custom {
                bytes: &[2],
                text: false,
                null: false,
            },
        ),
        (
            Type::TEXT,
            Custom {
                bytes: &[255],
                text: false,
                null: false,
            },
        ),
        (
            Type::INT4,
            Custom {
                bytes: b"42",
                text: true,
                null: false,
            },
        ),
        (
            Type::INT4,
            Custom {
                bytes: &[1],
                text: false,
                null: true,
            },
        ),
        (
            Type::NUMERIC,
            Custom {
                bytes: &[],
                text: false,
                null: true,
            },
        ),
    ] {
        let error = encode(ty, &value).unwrap_err();
        assert_eq!(error.to_string(), "error serializing parameter 0");
        assert!(error.code().is_none());
    }
    assert!(encode(Type::INT8, &42i32).is_err());
    let unsupported = [
        Type::UUID,
        Type::JSONB,
        Type::NUMERIC,
        Type::TIMESTAMP,
        Type::INT4_ARRAY,
    ];
    for ty in unsupported {
        assert!(parameter_codecs(&[ty]).is_err());
    }
}

// [spec:pgorm:req:turso.rows/test]
#[test]
fn custom_from_sql_reads_common_row_storage() {
    #[derive(Debug, PartialEq)]
    struct Borrowed<'a>(&'a str);
    impl<'a> FromSql<'a> for Borrowed<'a> {
        fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Cause> {
            Ok(Self(std::str::from_utf8(raw)?))
        }
        fn accepts(ty: &Type) -> bool {
            *ty == Type::TEXT
        }
    }
    let row = row(&[Type::TEXT], &[Value::build_text("owned by the row")]).unwrap();
    let first = row.get::<_, Borrowed<'_>>("C0");
    let second = row.get::<_, &str>(0);
    assert_eq!(first.0, second);
    assert_eq!(first.0.as_ptr(), second.as_ptr());
}

#[test]
fn parameter_count_precedes_user_encoding() {
    let types = [Type::INT4, Type::INT4];
    let codecs = parameter_codecs(&types).unwrap();
    let mut scratch = BytesMut::new();
    let error = encode_params(
        &types,
        &codecs,
        [&1i32 as &(dyn ToSql + Sync)],
        &mut scratch,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "expected 2 parameters but got 1");
    assert!(scratch.is_empty());
}
