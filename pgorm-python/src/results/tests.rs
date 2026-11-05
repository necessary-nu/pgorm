use bytes::BytesMut;
use jiff::civil::{Time, time};
use tokio_postgres::types::{FromSql, ToSql, Type};

use super::codecs::CheckedArray;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

// [spec:pgorm:req:bindings.values/test]
#[test]
fn array_codec_preserves_nullable_rust_elements() -> TestResult {
    let expected = vec![Some(1i32), None, Some(3)];
    let mut raw = BytesMut::new();
    expected.to_sql(&Type::INT4_ARRAY, &mut raw)?;
    assert_eq!(
        CheckedArray::<i32>::from_sql(&Type::INT4_ARRAY, &raw)?.0,
        expected
    );
    // The driver's Vec decoder ignores the first dimension's lower bound.
    raw[16..20].copy_from_slice(&0i32.to_be_bytes());
    assert_eq!(
        Vec::<Option<i32>>::from_sql(&Type::INT4_ARRAY, &raw)?,
        expected
    );
    assert!(CheckedArray::<i32>::from_sql(&Type::INT4_ARRAY, &raw).is_err());
    assert!(CheckedArray::<i32>::from_sql(&Type::INT4, &raw).is_err());
    Ok(())
}

// [spec:pgorm:req:bindings.values/test]
#[test]
fn time_decode_rejects_values_outside_a_civil_day() -> TestResult {
    let valid = 45_296_123_456i64.to_be_bytes();
    assert_eq!(
        Time::from_sql(&Type::TIME, &valid)?,
        time(12, 34, 56, 123_456_000)
    );
    // PostgreSQL admits 24:00:00; a civil time has no such hour.
    for outside in [86_400_000_000i64, -1, i64::MAX] {
        assert!(Time::from_sql(&Type::TIME, &outside.to_be_bytes()).is_err());
    }
    // Every accepted value re-encodes byte for byte, so decoding cannot round.
    let mut encoded = BytesMut::new();
    Time::from_sql(&Type::TIME, &valid)?.to_sql(&Type::TIME, &mut encoded)?;
    assert_eq!(valid.as_ref(), encoded.as_ref());
    Ok(())
}
