//! Binding a [`Value`] as a statement parameter, checked against the wire
//! format of the type PostgreSQL inferred for the placeholder.
//!
//! Split from `cursor.rs`: the bind adapter serves every executor, not just
//! cursors, and the acceptance matrix reads as one unit here.

use super::*;

/// Adapter binding a [`Value`] as a query parameter, converting it to the
/// Postgres type inferred for that placeholder — and refusing, before the
/// statement is sent, when the value has no representation that type could
/// receive.
// [spec:pgorm:def:exec.cursor.binding+6]
pub struct ValueHolder(pub Value);

impl std::fmt::Debug for ValueHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

use crate::executor::created_range::{is_created_multirange, is_created_range};
use bytes::BytesMut;
// A domain is transparent on the wire: its values are sent in the
// representation of the type it is built over, so the whole binding decision,
// the acceptance check and the encoding alike, is made against `base_type`,
// and only a refusal names the type as the schema declares it.
// [spec:pgorm:req:exec.cursor.binding-accepts+4]
use pgorm_query::base_type;
use rust_decimal::Decimal;

#[path = "cursor_bind_range.rs"]
mod range;

type BindResult = Result<IsNull, Box<dyn std::error::Error + Sync + Send>>;

fn out_of_range(
    value: impl std::fmt::Display,
    ty: &Type,
) -> Box<dyn std::error::Error + Sync + Send> {
    format!("value `{value}` is out of range for Postgres type `{ty}`").into()
}

/// The refusal raised when a variant has no encoding that is the wire format
/// of the type Postgres inferred for the placeholder.
// [spec:pgorm:req:exec.cursor.binding-accepts+4]
fn mismatch(kind: &'static str, ty: &Type) -> Box<dyn std::error::Error + Sync + Send> {
    format!("cannot bind a `{kind}` value to Postgres type `{ty}`").into()
}

/// The types whose binary representation is the value's text: the four
/// built-in string types, every enum (a label is sent as its own text),
/// `xml`, `unknown`, and the text-backed extension types `postgres-types`
/// itself names.
// [spec:pgorm:req:exec.cursor.binding-accepts+4]
fn is_textual(ty: &Type) -> bool {
    matches!(
        *ty,
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::XML | Type::UNKNOWN
    ) || matches!(ty.kind(), Kind::Enum(_))
        || matches!(ty.name(), "citext" | "ltree" | "lquery" | "ltxtquery")
}

/// `timestamp` and `timestamptz` share one representation — microseconds since
/// 2000-01-01 — and differ only in whether that instant is read as a wall
/// clock or as UTC. An acceptance check is about representation, so both are
/// accepted for both datetime variants, exactly as `postgres-types` does for
/// its own `SystemTime` impl. Which of the two a value *means* is the caller's.
// [spec:pgorm:req:exec.cursor.binding-accepts+4]
fn is_timestamp(ty: &Type) -> bool {
    matches!(*ty, Type::TIMESTAMP | Type::TIMESTAMPTZ)
}

/// The epoch a `timestamp` counts microseconds from on the wire. Not the Unix
/// epoch.
const PG_EPOCH: jiff::civil::DateTime = jiff::civil::DateTime::constant(2000, 1, 1, 0, 0, 0, 0);

/// Microseconds from [`PG_EPOCH`] to a civil datetime, truncated toward zero —
/// the wire payload of a `timestamp`.
///
/// Computed here rather than delegated to `postgres-types`' `ToSql for
/// jiff::civil::DateTime`, which reaches the same number by rounding a `Span`
/// *relative to* a civil datetime. `since` hands back a span whose largest
/// unit is days, and balancing days down to microseconds obliges jiff to say
/// how long a day is; given a relative datetime rather than the invariant
/// marker, it answers by projecting the reference point onto the absolute
/// `Timestamp` timeline. `jiff::Timestamp` covers a strictly narrower range
/// than `civil::DateTime` does — it holds back jiff's largest UTC offset at
/// each end so every instant still has a civil rendering in every zone — so a
/// civil datetime outside `-009999-01-02T01:59:59 ..= 9999-12-30T22:00:00`
/// cannot be rounded and is refused as `value too large to transmit`, though
/// the value is representable, its microsecond count is unremarkable, and
/// `postgres-types`' own decoder reads those very bytes back. A difference of
/// two civil datetimes needs no calendar context at all, so taking it as a
/// `SignedDuration` is exact across the whole civil range and agrees with the
/// upstream encoding everywhere upstream produces one.
///
/// `None` only when the count leaves `i64`, which the bounded civil range
/// makes unreachable; it is an error rather than a panic all the same.
// [spec:pgorm:req:exec.cursor.binding-gaps+3]
fn civil_micros_since_pg_epoch(value: &jiff::civil::DateTime) -> Option<i64> {
    i64::try_from(value.duration_since(PG_EPOCH).as_micros()).ok()
}

/// Bind a naive datetime as microseconds since [`PG_EPOCH`], written by pgorm
/// rather than by `postgres-types` — see [`civil_micros_since_pg_epoch`] for
/// why the upstream encoder cannot be used at the ends of the civil range.
// [spec:pgorm:req:exec.cursor.binding-gaps+3]
fn bind_civil_datetime(
    value: Option<&jiff::civil::DateTime>,
    ty: &Type,
    out: &mut BytesMut,
) -> BindResult {
    match value {
        None => Ok(IsNull::Yes),
        Some(_) if !is_timestamp(base_type(ty)) => Err(mismatch("DateTime", ty)),
        Some(value) => match civil_micros_since_pg_epoch(value) {
            Some(micros) => {
                postgres_protocol::types::timestamp_to_sql(micros, out);
                Ok(IsNull::No)
            }
            None => Err(out_of_range(value, ty)),
        },
    }
}

/// The largest display scale PostgreSQL accepts in an external `numeric`
/// (`NUMERIC_DSCALE_MASK` in `numeric.c`). `rust_decimal` caps its own scale at
/// 28, so this is a bound the type cannot currently reach — it is checked
/// rather than assumed because the refusal belongs on this side of the wire.
const MAX_NUMERIC_DSCALE: u16 = 0x3fff;

/// Write a zero `numeric`.
///
/// The binary form is a four-field header — group count, weight, sign, display
/// scale — followed by one 16-bit word per base-10000 digit group. Zero has no
/// digit groups, so its whole encoding is the header, and the display scale is
/// the only field left carrying anything.
// [spec:pgorm:req:exec.cursor.binding-gaps+3]
fn zero_numeric_to_sql(scale: u16, out: &mut BytesMut) {
    use bytes::BufMut;

    out.reserve(8);
    out.put_u16(0);
    out.put_i16(0);
    // Positive. A zero is never negative on the wire: PostgreSQL renders
    // `-0.00` as `0.00`, and `rust_decimal` reports `neg: false` for zero too.
    out.put_u16(0x0000);
    out.put_u16(scale);
}

/// Write a decimal in PostgreSQL's binary `numeric` format, taking zero away
/// from `rust_decimal`.
///
/// `rust_decimal`'s `ToSql` builds its wire form through `to_postgres`, which
/// short-circuits on zero and returns a fixed record carrying `scale: 0` —
/// discarding `self.scale()`, the one field a zero's encoding still has to
/// say anything with. `0.00` therefore reaches the server as `0`, while the
/// same value rendered as a literal arrives as `0.00` and keeps its scale,
/// which PostgreSQL treats as part of a numeric's identity. Every other value
/// reads `self.scale()` on the ordinary path, so only zero is written here and
/// everything else still delegates.
// [spec:pgorm:req:exec.cursor.binding-gaps+3]
fn numeric_to_sql(value: &Decimal, ty: &Type, out: &mut BytesMut) -> BindResult {
    if !value.is_zero() {
        return value.to_sql(base_type(ty), out);
    }
    match u16::try_from(value.scale()) {
        Ok(scale) if scale <= MAX_NUMERIC_DSCALE => {
            zero_numeric_to_sql(scale, out);
            Ok(IsNull::No)
        }
        _ => Err(out_of_range(value, ty)),
    }
}

/// Bind a decimal, which is written only against `numeric` — see
/// [`numeric_to_sql`] for why the zero case is not delegated.
// [spec:pgorm:req:exec.cursor.binding-gaps+3]
fn bind_decimal(value: Option<&Decimal>, ty: &Type, out: &mut BytesMut) -> BindResult {
    match value {
        None => Ok(IsNull::Yes),
        Some(_) if *base_type(ty) != Type::NUMERIC => Err(mismatch("Decimal", ty)),
        Some(value) => numeric_to_sql(value, ty, out),
    }
}

/// Bind a payload whose binary encoding is the wire format of the `accepted`
/// types and of no others.
///
/// A `None` payload is SQL `NULL`, which is sent as a length of -1 with no
/// bytes at all. Having no representation, it has none to mismatch, so it
/// binds against whatever type Postgres inferred.
// [spec:pgorm:req:exec.cursor.binding-accepts+4]
fn bind_exact<T>(
    value: Option<&T>,
    kind: &'static str,
    accepted: fn(&Type) -> bool,
    ty: &Type,
    out: &mut BytesMut,
) -> BindResult
where
    T: ToSql,
{
    let target = base_type(ty);
    match value {
        None => Ok(IsNull::Yes),
        Some(value) if accepted(target) => value.to_sql(target, out),
        Some(_) => Err(mismatch(kind, ty)),
    }
}

/// Bind an integer against the type Postgres inferred for the placeholder.
/// Within the numeric family the value is converted and written in *that*
/// type's format; `oid` and `"char"` take the widths their own impls define,
/// through the same checked conversion. Any other inferred type has no
/// integer representation to receive, and is refused.
// [spec:pgorm:req:exec.cursor.binding-coerce+2]
fn integer_to_sql(value: i64, kind: &'static str, ty: &Type, out: &mut BytesMut) -> BindResult {
    let target = base_type(ty);
    if *target == Type::INT2 {
        match i16::try_from(value) {
            Ok(value) => value.to_sql(target, out),
            Err(_) => Err(out_of_range(value, ty)),
        }
    } else if *target == Type::INT4 {
        match i32::try_from(value) {
            Ok(value) => value.to_sql(target, out),
            Err(_) => Err(out_of_range(value, ty)),
        }
    } else if *target == Type::INT8 {
        value.to_sql(target, out)
    } else if *target == Type::OID {
        match u32::try_from(value) {
            Ok(value) => value.to_sql(target, out),
            Err(_) => Err(out_of_range(value, ty)),
        }
    } else if *target == Type::CHAR {
        match i8::try_from(value) {
            Ok(value) => value.to_sql(target, out),
            Err(_) => Err(out_of_range(value, ty)),
        }
    } else if *target == Type::FLOAT4 {
        (value as f32).to_sql(target, out)
    } else if *target == Type::FLOAT8 {
        (value as f64).to_sql(target, out)
    } else if *target == Type::NUMERIC {
        numeric_to_sql(&Decimal::from(value), ty, out)
    } else {
        Err(mismatch(kind, ty))
    }
}

/// The floating-point counterpart of [`integer_to_sql`]. Narrowing to `float4`
/// rounds the way Postgres' own `float8 -> float4` cast does, but a conversion
/// that would silently drop the fractional part or overflow is an error rather
/// than a lie.
// [spec:pgorm:req:exec.cursor.binding-coerce+2]
fn float_to_sql(value: f64, kind: &'static str, ty: &Type, out: &mut BytesMut) -> BindResult {
    let target = base_type(ty);
    if *target == Type::FLOAT4 {
        let narrowed = value as f32;
        if value.is_finite() && !narrowed.is_finite() {
            Err(out_of_range(value, ty))
        } else {
            narrowed.to_sql(target, out)
        }
    } else if *target == Type::FLOAT8 {
        value.to_sql(target, out)
    } else if *target == Type::NUMERIC {
        match Decimal::try_from(value) {
            Ok(value) => numeric_to_sql(&value, ty, out),
            Err(_) => Err(out_of_range(value, ty)),
        }
    } else if *target == Type::INT2 || *target == Type::INT4 || *target == Type::INT8 {
        Err(format!(
            "cannot bind floating-point value `{value}` to Postgres type `{ty}` without loss"
        )
        .into())
    } else {
        Err(mismatch(kind, ty))
    }
}

// [spec:pgorm:req:exec.cursor.binding-coerce+2]
fn bind_integer<T>(
    value: Option<T>,
    kind: &'static str,
    ty: &Type,
    out: &mut BytesMut,
) -> BindResult
where
    T: Copy + Into<i64>,
{
    match value {
        None => Ok(IsNull::Yes),
        Some(value) => integer_to_sql(value.into(), kind, ty, out),
    }
}

// [spec:pgorm:req:exec.cursor.binding-coerce+2]
fn bind_float<T>(value: Option<T>, kind: &'static str, ty: &Type, out: &mut BytesMut) -> BindResult
where
    T: Copy + Into<f64>,
{
    match value {
        None => Ok(IsNull::Yes),
        Some(value) => float_to_sql(value.into(), kind, ty, out),
    }
}

/// `u64` is the one integer variant with no `Into<i64>`: Postgres has no
/// unsigned 64-bit type, so the value has to fit `i64` to be written at all.
// [spec:pgorm:req:exec.cursor.binding-coerce+2]
fn bind_big_unsigned(value: Option<u64>, ty: &Type, out: &mut BytesMut) -> BindResult {
    match value {
        None => Ok(IsNull::Yes),
        Some(value) => match i64::try_from(value) {
            Ok(value) => bind_integer(Some(value), "BigUnsigned", ty, out),
            Err(_) => Err(out_of_range(value, ty)),
        },
    }
}

// [spec:pgorm:def:exec.cursor.binding+6]
impl ToSql for ValueHolder {
    // [spec:pgorm:req:exec.cursor.binding-gaps+3]
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        match &self.0 {
            Value::Bool(x) => bind_exact(x.as_ref(), "Bool", |ty| *ty == Type::BOOL, ty, out),
            Value::TinyInt(x) => bind_integer(*x, "TinyInt", ty, out),
            Value::SmallInt(x) => bind_integer(*x, "SmallInt", ty, out),
            Value::Int(x) => bind_integer(*x, "Int", ty, out),
            Value::BigInt(x) => bind_integer(*x, "BigInt", ty, out),
            Value::Unsigned(x) => bind_integer(*x, "Unsigned", ty, out),
            Value::BigUnsigned(x) => bind_big_unsigned(*x, ty, out),
            Value::Float(x) => bind_float(*x, "Float", ty, out),
            Value::Double(x) => bind_float(*x, "Double", ty, out),
            Value::String(Some(text))
                if is_created_range(base_type(ty)) || is_created_multirange(base_type(ty)) =>
            {
                range::bind_created_text(text, ty, out)
            }
            Value::String(x) => bind_exact(x.as_deref(), "String", is_textual, ty, out),
            Value::Char(x) => {
                let x = x.map(|x| x.to_string());
                bind_exact(x.as_ref(), "Char", is_textual, ty, out)
            }
            Value::Bytes(x) => bind_exact(x.as_deref(), "Bytes", |ty| *ty == Type::BYTEA, ty, out),
            Value::Json(x) => bind_exact(
                x.as_deref(),
                "Json",
                |ty| matches!(*ty, Type::JSON | Type::JSONB),
                ty,
                out,
            ),
            Value::Date(x) => bind_exact(x.as_deref(), "Date", |ty| *ty == Type::DATE, ty, out),
            Value::Time(x) => bind_exact(x.as_deref(), "Time", |ty| *ty == Type::TIME, ty, out),
            Value::DateTime(x) => bind_civil_datetime(x.as_deref(), ty, out),
            Value::DateTimeWithTimeZone(x) => {
                bind_exact(x.as_deref(), "DateTimeWithTimeZone", is_timestamp, ty, out)
            }
            Value::Uuid(x) => bind_exact(x.as_deref(), "Uuid", |ty| *ty == Type::UUID, ty, out),
            Value::Decimal(x) => bind_decimal(x.as_deref(), ty, out),
            Value::Array(_, Some(x)) => {
                // `Vec<T>`'s own impl panics on a non-array type rather than
                // erroring, so the shape has to be established here first.
                // Each element then re-enters this impl with the member type.
                let target = base_type(ty);
                match target.kind() {
                    Kind::Array(_) => x
                        .iter()
                        .map(|x| ValueHolder(x.clone()))
                        .collect::<Vec<_>>()
                        .to_sql(target, out),
                    _ => Err(mismatch("Array", ty)),
                }
            }
            Value::Array(_, None) => Ok(IsNull::Yes),
            Value::Vector(x) => {
                bind_exact(x.as_deref(), "Vector", |ty| ty.name() == "vector", ty, out)
            }
            Value::IpNetwork(x) => match x.as_deref() {
                None => Ok(IsNull::Yes),
                Some(_) if !matches!(*base_type(ty), Type::INET | Type::CIDR) => {
                    Err(mismatch("IpNetwork", ty))
                }
                Some(x) => {
                    postgres_protocol::types::inet_to_sql(x.ip(), x.prefix(), out);
                    Ok(IsNull::No)
                }
            },
            Value::MacAddress(x) => match x.as_deref() {
                None => Ok(IsNull::Yes),
                Some(_) if *base_type(ty) != Type::MACADDR => Err(mismatch("MacAddress", ty)),
                Some(x) => {
                    postgres_protocol::types::macaddr_to_sql(x.bytes(), out);
                    Ok(IsNull::No)
                }
            },
            // [spec:pgorm:req:exec.cursor.binding-interval]
            Value::Interval(x) => match x.as_deref() {
                None => Ok(IsNull::Yes),
                Some(_) if *base_type(ty) != Type::INTERVAL => Err(mismatch("Interval", ty)),
                Some(x) => {
                    out.extend_from_slice(&x.to_binary());
                    Ok(IsNull::No)
                }
            },
            Value::Range(_, Some(x)) => range::bind_range(x, ty, out),
            Value::Multirange(_, Some(x)) => range::bind_multirange(x, ty, out),
            Value::Range(_, None) | Value::Multirange(_, None) => Ok(IsNull::Yes),
            _ => Err(format!("pgorm has no binding for this value to Postgres type `{ty}`").into()),
        }
    }

    /// Every Postgres type is accepted here because this is the wrong place to
    /// refuse one: `accepts` is a static method, with no access to the `Value`
    /// whose representation is the question. The acceptance decision of
    /// `[spec:pgorm:req:exec.cursor.binding-accepts+4]` therefore lives in
    /// `to_sql`, which is reached through `to_sql_checked!` and runs
    /// client-side before the bind message is sent.
    // [spec:pgorm:req:exec.cursor.binding-accepts+4]
    fn accepts(_ty: &Type) -> bool
    where
        Self: Sized,
    {
        true
    }

    to_sql_checked!();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for a type `postgres-types` has no constant for: an
    /// extension type (`vector`), an enum, or a domain over another type.
    fn named(name: &str, kind: Kind) -> Type {
        Type::new(name.to_owned(), 16_384, kind, "public".to_owned())
    }

    fn encode_as(value: Value, ty: &Type) -> Result<Option<Vec<u8>>, String> {
        let mut out = BytesMut::new();
        match ValueHolder(value).to_sql(ty, &mut out) {
            Ok(IsNull::Yes) => Ok(None),
            Ok(IsNull::No) => Ok(Some(out.to_vec())),
            Err(err) => Err(err.to_string()),
        }
    }

    fn bytes(value: Value, ty: &Type) -> Vec<u8> {
        encode_as(value, ty).unwrap().unwrap()
    }

    fn error(value: Value, ty: &Type) -> String {
        encode_as(value, ty).unwrap_err()
    }

    // [spec:pgorm:req:exec.cursor.binding-coerce+2/test]
    #[test]
    fn binds_integer_as_float() {
        assert_eq!(
            bytes(Value::Int(Some(2)), &Type::FLOAT8),
            2.0f64.to_be_bytes()
        );
        assert_eq!(
            bytes(Value::Int(Some(2)), &Type::FLOAT4),
            2.0f32.to_be_bytes()
        );
        assert_eq!(
            bytes(Value::BigInt(Some(-3)), &Type::FLOAT8),
            (-3.0f64).to_be_bytes()
        );
        assert_eq!(encode_as(Value::Int(None), &Type::FLOAT8), Ok(None));
    }

    // [spec:pgorm:req:exec.cursor.binding-coerce+2/test]
    #[test]
    fn binds_integer_across_widths() {
        assert_eq!(
            bytes(Value::BigInt(Some(300)), &Type::INT2),
            300i16.to_be_bytes()
        );
        assert_eq!(
            bytes(Value::SmallInt(Some(7)), &Type::INT8),
            7i64.to_be_bytes()
        );
        assert_eq!(
            bytes(Value::TinyInt(Some(-1)), &Type::INT4),
            (-1i32).to_be_bytes()
        );
        assert_eq!(
            bytes(Value::BigUnsigned(Some(9)), &Type::INT2),
            9i16.to_be_bytes()
        );
        assert_eq!(
            bytes(Value::Unsigned(Some(9)), &Type::OID),
            9u32.to_be_bytes()
        );
        assert_eq!(
            bytes(Value::TinyInt(Some(65)), &Type::CHAR),
            65i8.to_be_bytes()
        );
    }

    // [spec:pgorm:req:exec.cursor.binding-coerce+2/test]
    #[test]
    fn binds_integer_as_numeric() {
        assert_eq!(
            bytes(Value::Int(Some(2)), &Type::NUMERIC),
            vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 2]
        );
    }

    /// The reported defect. A zero `numeric` is written as a bare header whose
    /// only informative field is the display scale, and `rust_decimal`'s
    /// `to_postgres` short-circuits on zero with that field hardcoded to 0 —
    /// so `0.00` was sent as `0` while the literal path sent `0.00`. Every
    /// non-zero value reads its own scale and was never affected.
    // [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
    #[test]
    fn binds_zero_decimals_with_the_scale_they_carry() {
        // ndigits, weight, sign, dscale — no digit groups follow.
        let header = |scale: u16| {
            [
                &0u16.to_be_bytes()[..],
                &0i16.to_be_bytes()[..],
                &0u16.to_be_bytes()[..],
                &scale.to_be_bytes()[..],
            ]
            .concat()
        };

        for scale in [0u32, 1, 2, 6, 28] {
            let value = Decimal::new(0, scale);
            assert_eq!(value.scale(), scale, "rust_decimal kept the scale");
            let expected = u16::try_from(scale).expect("a small scale");
            assert_eq!(
                bytes(Value::Decimal(Some(Box::new(value))), &Type::NUMERIC),
                header(expected),
                "binding a zero of scale {scale}"
            );
        }

        // A negative zero is still written positive, which is both what
        // PostgreSQL renders and what `rust_decimal` itself reports for zero.
        let mut negative = Decimal::new(0, 2);
        negative.set_sign_negative(true);
        assert_eq!(
            bytes(Value::Decimal(Some(Box::new(negative))), &Type::NUMERIC),
            header(2)
        );

        // Non-zero values keep delegating, byte for byte.
        for value in [
            Decimal::new(150, 2),
            Decimal::new(15, 1),
            Decimal::new(123_400, 4),
            Decimal::new(10, 2),
            Decimal::new(10_000, 2),
            Decimal::ONE,
            Decimal::new(-1_005, 3),
            Decimal::MAX,
            Decimal::MIN,
        ] {
            let mut upstream = BytesMut::new();
            value
                .to_sql(&Type::NUMERIC, &mut upstream)
                .expect("rust_decimal writes every non-zero value");
            assert_eq!(
                bytes(Value::Decimal(Some(Box::new(value))), &Type::NUMERIC),
                upstream.to_vec(),
                "binding {value}"
            );
        }

        // The refusal and the NULL path are untouched by the new arm.
        assert_eq!(
            error(Value::Decimal(Some(Box::new(Decimal::ZERO))), &Type::FLOAT8),
            "cannot bind a `Decimal` value to Postgres type `float8`"
        );
        assert_eq!(encode_as(Value::Decimal(None), &Type::NUMERIC), Ok(None));

        // A domain over `numeric` resolves to it and takes the same encoding.
        let money = named("money_amount", Kind::Domain(Type::NUMERIC));
        assert_eq!(
            bytes(Value::Decimal(Some(Box::new(Decimal::new(0, 4)))), &money),
            header(4)
        );

        // Zeroes reached through the numeric coercion have no authored scale,
        // so they keep scale 0 — the coercion is not a place to invent one.
        assert_eq!(bytes(Value::Int(Some(0)), &Type::NUMERIC), header(0));
        assert_eq!(bytes(Value::Double(Some(0.0)), &Type::NUMERIC), header(0));
    }

    /// The array path inherits the fix through `ValueHolder`, which is worth
    /// pinning rather than assuming: an array of zeroes must carry each
    /// element's scale too.
    // [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
    #[test]
    fn binds_zero_decimals_inside_an_array() {
        use pgorm_query::ArrayType;

        let encoded = bytes(
            Value::Array(
                ArrayType::Decimal,
                Some(Box::new(vec![
                    Value::Decimal(Some(Box::new(Decimal::new(0, 2)))),
                    Value::Decimal(Some(Box::new(Decimal::new(0, 0)))),
                ])),
            ),
            &Type::NUMERIC_ARRAY,
        );

        // Each element is length-prefixed; the dscale is the last two bytes of
        // each eight-byte zero header.
        let scales: Vec<u16> = encoded
            .windows(12)
            .filter(|w| w[..4] == 8u32.to_be_bytes())
            .map(|w| u16::from_be_bytes([w[10], w[11]]))
            .collect();
        assert_eq!(scales, vec![2, 0]);
    }

    // [spec:pgorm:req:exec.cursor.binding-coerce+2/test]
    #[test]
    fn rejects_out_of_range_integer() {
        assert_eq!(
            error(Value::BigInt(Some(i64::from(i32::MAX) + 1)), &Type::INT4),
            "value `2147483648` is out of range for Postgres type `int4`"
        );
        assert_eq!(
            error(Value::Int(Some(40_000)), &Type::INT2),
            "value `40000` is out of range for Postgres type `int2`"
        );
    }

    // [spec:pgorm:req:exec.cursor.binding-coerce+2/test]    a `u64` above
    // `i64::MAX` has no `int8` to be written in, so it errors rather than
    // wrapping to a negative — whatever type Postgres inferred
    #[test]
    fn rejects_out_of_range_big_unsigned() {
        let too_big = i64::MAX as u64 + 1;

        for ty in [
            &Type::INT8,
            &Type::INT4,
            &Type::INT2,
            &Type::NUMERIC,
            &Type::FLOAT8,
            &Type::TEXT,
        ] {
            assert_eq!(
                error(Value::BigUnsigned(Some(too_big)), ty),
                format!("value `{too_big}` is out of range for Postgres type `{ty}`")
            );
        }

        assert_eq!(
            error(Value::BigUnsigned(Some(u64::MAX)), &Type::INT8),
            "value `18446744073709551615` is out of range for Postgres type `int8`"
        );

        // The largest value that does fit is written exactly, not approximated.
        assert_eq!(
            bytes(Value::BigUnsigned(Some(i64::MAX as u64)), &Type::INT8),
            i64::MAX.to_be_bytes()
        );
        assert_eq!(
            bytes(Value::BigUnsigned(Some(7)), &Type::FLOAT8),
            7.0f64.to_be_bytes()
        );
        assert_eq!(encode_as(Value::BigUnsigned(None), &Type::INT8), Ok(None));
    }

    // [spec:pgorm:req:exec.cursor.binding-coerce+2/test]
    #[test]
    fn binds_float_across_widths() {
        assert_eq!(
            bytes(Value::Float(Some(1.5)), &Type::FLOAT8),
            1.5f64.to_be_bytes()
        );
        assert_eq!(
            bytes(Value::Double(Some(1.5)), &Type::FLOAT4),
            1.5f32.to_be_bytes()
        );
        assert!(
            error(Value::Double(Some(1e300)), &Type::FLOAT4)
                .ends_with("is out of range for Postgres type `float4`")
        );
    }

    // [spec:pgorm:req:exec.cursor.binding-coerce+2/test]
    #[test]
    fn rejects_float_bound_to_integer() {
        assert_eq!(
            error(Value::Double(Some(1.5)), &Type::INT8),
            "cannot bind floating-point value `1.5` to Postgres type `int8` without loss"
        );
    }

    // [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
    #[test]
    fn binds_vector() {
        let vector = named("vector", Kind::Simple);
        assert_eq!(
            bytes(
                Value::Vector(Some(Box::new(pgvector::Vector::from(vec![1.0f32, 2.0])))),
                &vector
            ),
            [
                &[0, 2, 0, 0][..],
                &1.0f32.to_be_bytes()[..],
                &2.0f32.to_be_bytes()[..],
            ]
            .concat()
        );
        assert_eq!(encode_as(Value::Vector(None), &vector), Ok(None));
    }

    // [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
    #[test]
    fn binds_ip_network() {
        assert_eq!(
            bytes(
                Value::IpNetwork(Some(Box::new("10.0.0.1/24".parse().unwrap()))),
                &Type::INET
            ),
            vec![2, 24, 0, 4, 10, 0, 0, 1]
        );
        assert_eq!(encode_as(Value::IpNetwork(None), &Type::INET), Ok(None));
    }

    // [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
    #[test]
    fn binds_mac_address() {
        assert_eq!(
            bytes(
                Value::MacAddress(Some(Box::new("00:11:22:33:44:55".parse().unwrap()))),
                &Type::MACADDR
            ),
            vec![0x00, 0x11, 0x22, 0x33, 0x44, 0x55]
        );
        assert_eq!(encode_as(Value::MacAddress(None), &Type::MACADDR), Ok(None));
    }

    // [spec:pgorm:req:exec.cursor.binding-interval/test]
    #[test]
    fn binds_interval_fields_in_wire_order() {
        let interval = pgorm_query::Interval::new(1, -2, 3_000_004);
        let wire = [
            &3_000_004i64.to_be_bytes()[..],
            &(-2i32).to_be_bytes()[..],
            &1i32.to_be_bytes()[..],
        ]
        .concat();
        assert_eq!(bytes(interval.into(), &Type::INTERVAL), wire);
        let span = named("span", Kind::Domain(Type::INTERVAL));
        assert_eq!(bytes(interval.into(), &span), wire);
        assert_eq!(encode_as(Value::Interval(None), &Type::INT8), Ok(None));
        assert_eq!(
            bytes(Value::array([interval]), &Type::INTERVAL_ARRAY).len(),
            20 + 4 + 16
        );
    }

    // [spec:pgorm:req:exec.cursor.binding-interval/test]
    #[test]
    fn refuses_interval_bound_to_other_types() {
        let interval = pgorm_query::Interval::new(0, 1, 0);
        for ty in [&Type::INT8, &Type::TEXT, &Type::TIME, &Type::TIMESTAMP] {
            assert_eq!(
                error(interval.into(), ty),
                format!("cannot bind a `Interval` value to Postgres type `{ty}`")
            );
        }
        assert_eq!(
            error(Value::array([interval]), &Type::INTERVAL),
            "cannot bind a `Array` value to Postgres type `interval`"
        );
    }

    /// The defect this check exists for: ASCII digits written into an `int4`
    /// placeholder were read back as the integer those bytes spell.
    // [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
    #[test]
    fn rejects_string_bound_to_non_textual_types() {
        for ty in [
            &Type::INT4,
            &Type::INT8,
            &Type::FLOAT8,
            &Type::NUMERIC,
            &Type::BOOL,
            &Type::BYTEA,
            &Type::UUID,
            &Type::DATE,
            &Type::TIMESTAMPTZ,
            &Type::JSONB,
            &Type::INET,
        ] {
            assert_eq!(
                error(Value::String(Some(Box::new("1234".to_owned()))), ty),
                format!("cannot bind a `String` value to Postgres type `{ty}`")
            );
        }
    }

    /// Every type whose binary representation *is* the text keeps working.
    // [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
    #[test]
    fn binds_string_to_textual_types() {
        let mood = named("mood", Kind::Enum(vec!["happy".to_owned()]));
        for ty in [
            &Type::TEXT,
            &Type::VARCHAR,
            &Type::BPCHAR,
            &Type::NAME,
            &Type::XML,
            &Type::UNKNOWN,
            &mood,
            &named("citext", Kind::Simple),
        ] {
            assert_eq!(
                bytes(Value::String(Some(Box::new("happy".to_owned()))), ty),
                b"happy"
            );
        }
        assert_eq!(bytes(Value::Char(Some('h')), &mood), b"h");

        // `ltree` and its query types are accepted too, but `postgres-types`
        // writes them with a leading format-version byte rather than as bare
        // text, so the bytes are its business, not this check's.
        for name in ["ltree", "lquery", "ltxtquery"] {
            let ty = named(name, Kind::Simple);
            assert_eq!(
                bytes(Value::String(Some(Box::new("a.b".to_owned()))), &ty),
                [&[1u8][..], b"a.b"].concat()
            );
        }
    }

    /// A domain is transparent on the wire, so the decision is made against
    /// the type it is built over — in both directions.
    // [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
    #[test]
    fn resolves_domains_to_their_base_type() {
        let email = named("email", Kind::Domain(Type::TEXT));
        let positive = named("positive", Kind::Domain(Type::INT4));

        assert_eq!(
            bytes(Value::String(Some(Box::new("a@b".to_owned()))), &email),
            b"a@b"
        );
        assert_eq!(bytes(Value::BigInt(Some(7)), &positive), 7i32.to_be_bytes());

        // The refusal names the type the schema declares, not the base type
        // the check was made against.
        assert_eq!(
            error(Value::String(Some(Box::new("7".to_owned()))), &positive),
            "cannot bind a `String` value to Postgres type `positive`"
        );
        assert_eq!(
            error(Value::Int(Some(7)), &email),
            "cannot bind a `Int` value to Postgres type `email`"
        );
        assert_eq!(
            error(Value::BigInt(Some(i64::MAX)), &positive),
            format!(
                "value `{}` is out of range for Postgres type `positive`",
                i64::MAX
            )
        );
    }

    /// A domain over a domain resolves all the way down, and a `jsonb` domain
    /// still gets the version byte its base type's encoding prescribes.
    // [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
    #[test]
    fn resolves_nested_domains() {
        let inner = named("inner", Kind::Domain(Type::JSONB));
        let outer = named("outer", Kind::Domain(inner));
        assert_eq!(
            bytes(Value::Json(Some(Box::new(serde_json::json!(1)))), &outer),
            b"\x011"
        );
    }

    /// `NULL` is sent as a length of -1 with no bytes, so it has no
    /// representation to mismatch and binds against any inferred type.
    // [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
    #[test]
    fn binds_null_against_any_type() {
        for value in [
            Value::String(None),
            Value::Int(None),
            Value::Json(None),
            Value::Uuid(None),
            Value::Bytes(None),
            Value::Array(pgorm_query::ArrayType::Int, None),
        ] {
            for ty in [&Type::INT4, &Type::TEXT, &Type::BYTEA, &Type::UUID] {
                assert_eq!(encode_as(value.clone(), ty), Ok(None));
            }
        }
    }

    // [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
    #[test]
    fn rejects_mismatched_payload_variants() {
        let cases = [
            (Value::Bool(Some(true)), &Type::INT4, "Bool"),
            (Value::Bytes(Some(Box::new(vec![1]))), &Type::TEXT, "Bytes"),
            (
                Value::Json(Some(Box::new(serde_json::json!(1)))),
                &Type::TEXT,
                "Json",
            ),
            (
                Value::Uuid(Some(Box::new(uuid::Uuid::nil()))),
                &Type::TEXT,
                "Uuid",
            ),
            (
                Value::Decimal(Some(Box::new(Decimal::ONE))),
                &Type::FLOAT8,
                "Decimal",
            ),
            (Value::Int(Some(1)), &Type::BOOL, "Int"),
            (Value::Double(Some(1.5)), &Type::TEXT, "Double"),
            (
                Value::Date(Some(Box::new(jiff::civil::date(2026, 1, 1)))),
                &Type::TIMESTAMP,
                "Date",
            ),
            (
                Value::IpNetwork(Some(Box::new(
                    "10.0.0.1/24".parse().expect("a real network"),
                ))),
                &Type::BYTEA,
                "IpNetwork",
            ),
            (
                Value::MacAddress(Some(Box::new(
                    "00:11:22:33:44:55".parse().expect("a real address"),
                ))),
                &Type::BYTEA,
                "MacAddress",
            ),
        ];

        for (value, ty, kind) in cases {
            assert_eq!(
                error(value, ty),
                format!("cannot bind a `{kind}` value to Postgres type `{ty}`")
            );
        }
    }

    /// `timestamp` and `timestamptz` share a representation, so both datetime
    /// variants bind against either.
    // [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
    #[test]
    fn binds_datetimes_to_either_timestamp_type() {
        let naive = jiff::civil::datetime(2000, 1, 1, 0, 0, 1, 0);
        let instant: jiff::Timestamp = "2000-01-01T00:00:01Z".parse().expect("a real instant");
        let micros = 1_000_000i64.to_be_bytes();

        for ty in [&Type::TIMESTAMP, &Type::TIMESTAMPTZ] {
            assert_eq!(bytes(Value::DateTime(Some(Box::new(naive))), ty), micros);
            assert_eq!(
                bytes(Value::DateTimeWithTimeZone(Some(Box::new(instant))), ty),
                micros
            );
        }

        assert_eq!(
            error(Value::DateTime(Some(Box::new(naive))), &Type::DATE),
            "cannot bind a `DateTime` value to Postgres type `date`"
        );
        assert_eq!(encode_as(Value::DateTime(None), &Type::TIMESTAMP), Ok(None));
        assert_eq!(
            encode_as(Value::DateTimeWithTimeZone(None), &Type::TIMESTAMPTZ),
            Ok(None)
        );
    }

    /// The reported defect. `9999-12-31 23:59:59.999999` was refused before
    /// the bind message was sent, as `error serializing parameter 0` over
    /// `postgres-types`' bare `value too large to transmit`. Precision was
    /// incidental: the whole last day and a bit of the civil calendar range
    /// was unbindable, microseconds or not, because the upstream encoder
    /// balances its span *relative to* a civil datetime and so drags the value
    /// onto `jiff::Timestamp`'s narrower timeline.
    // [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
    #[test]
    fn binds_datetimes_at_the_civil_range_ends() {
        let cases = [
            // The campaign's value, and the whole band around it that shared
            // the fault.
            (
                jiff::civil::date(9999, 12, 31).at(23, 59, 59, 999_999_000),
                252_455_615_999_999_999i64,
            ),
            (
                jiff::civil::date(9999, 12, 31).at(23, 59, 59, 0),
                252_455_615_999_000_000,
            ),
            (
                jiff::civil::date(9999, 12, 30).at(22, 0, 1, 0),
                252_455_522_401_000_000,
            ),
            // The largest civil datetime the upstream encoder still managed,
            // which must not have moved.
            (
                jiff::civil::date(9999, 12, 30).at(22, 0, 0, 0),
                252_455_522_400_000_000,
            ),
            // Both extremes of what the type can hold. Sub-microsecond digits
            // truncate toward zero rather than rounding, which is jiff's
            // documented behaviour on write.
            (jiff::civil::DateTime::MAX, 252_455_615_999_999_999),
            (jiff::civil::DateTime::MIN, -378_651_801_600_000_000),
            (
                jiff::civil::date(-9999, 1, 1).at(0, 0, 0, 1_000),
                -378_651_801_599_999_999,
            ),
            (
                jiff::civil::date(-9999, 1, 2).at(1, 59, 59, 0),
                -378_651_708_001_000_000,
            ),
            // The epoch the wire format counts from, and either side of it.
            (jiff::civil::date(2000, 1, 1).at(0, 0, 0, 0), 0),
            (
                jiff::civil::date(1999, 12, 31).at(23, 59, 59, 999_999_000),
                -1,
            ),
        ];

        for (value, micros) in cases {
            for ty in [&Type::TIMESTAMP, &Type::TIMESTAMPTZ] {
                assert_eq!(
                    bytes(Value::DateTime(Some(Box::new(value))), ty),
                    micros.to_be_bytes(),
                    "binding {value} as {ty}"
                );
            }
        }

        // The arm still resolves a domain to the type it is built over, and
        // still refuses a target with no timestamp representation — the two
        // behaviours the shared `bind_exact` used to supply.
        let moment = named("moment", Kind::Domain(Type::TIMESTAMP));
        assert_eq!(
            bytes(Value::DateTime(Some(Box::new(campaign_value()))), &moment),
            252_455_615_999_999_999i64.to_be_bytes()
        );
        assert_eq!(
            error(
                Value::DateTime(Some(Box::new(campaign_value()))),
                &Type::DATE
            ),
            "cannot bind a `DateTime` value to Postgres type `date`"
        );
    }

    /// The value seven defects in one campaign run shared.
    fn campaign_value() -> jiff::civil::DateTime {
        jiff::civil::date(9999, 12, 31).at(23, 59, 59, 999_999_000)
    }

    /// Sub-microsecond digits truncate toward zero on both sides of the epoch,
    /// matching what `postgres-types` produces for the values it accepts.
    // [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
    #[test]
    fn truncates_sub_microsecond_digits_toward_zero() {
        let base = jiff::civil::date(2000, 1, 1).at(0, 0, 0, 0);
        for (nanos, micros) in [
            (-1_500i64, -1i64),
            (-999, 0),
            (-1, 0),
            (0, 0),
            (1, 0),
            (999, 0),
            (1_500, 1),
        ] {
            let value = base
                .checked_add(jiff::SignedDuration::from_nanos(nanos))
                .expect("inside the civil range");
            assert_eq!(
                bytes(Value::DateTime(Some(Box::new(value))), &Type::TIMESTAMP),
                micros.to_be_bytes(),
                "binding {value}"
            );
        }
    }

    /// pgorm encodes `timestamp` itself, so the replacement has to agree with
    /// `postgres-types` everywhere `postgres-types` produces an answer at all —
    /// the fix widens the accepted range and changes nothing inside it.
    // [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
    #[test]
    fn datetime_binding_agrees_with_postgres_types() {
        let mut value = jiff::civil::DateTime::MIN;
        let mut agreed = 0u32;
        loop {
            let mut upstream = BytesMut::new();
            if value.to_sql(&Type::TIMESTAMP, &mut upstream).is_ok() {
                assert_eq!(
                    bytes(Value::DateTime(Some(Box::new(value))), &Type::TIMESTAMP),
                    upstream.to_vec(),
                    "binding {value}"
                );
                agreed += 1;
            }
            // A stride with no common factor with a day, so the probes do not
            // all land on the same wall-clock time.
            let step = jiff::Span::new().days(337).hours(7).minutes(13).seconds(11);
            match value.checked_add(step) {
                Ok(next) => value = next,
                Err(_) => break,
            }
        }
        assert!(agreed > 20_000, "only {agreed} values were comparable");
    }

    /// The other three temporal variants at their own extremes. None of them
    /// shared the defect, and a regression in any of them would look exactly
    /// like the one that was fixed.
    // [spec:pgorm:req:exec.cursor.binding-gaps+3/test]
    #[test]
    fn binds_the_other_temporal_variants_at_their_extremes() {
        // `date` is a day count from 2000-01-01, in an `i32`.
        for (value, days) in [
            (jiff::civil::Date::MAX, 2_921_939i32),
            (jiff::civil::Date::MIN, -4_382_544),
            (jiff::civil::date(2000, 1, 1), 0),
        ] {
            assert_eq!(
                bytes(Value::Date(Some(Box::new(value))), &Type::DATE),
                days.to_be_bytes(),
                "binding {value}"
            );
        }

        // `time` is microseconds since midnight; the ninth fractional digit
        // truncates away.
        for (value, micros) in [
            (jiff::civil::Time::MAX, 86_399_999_999i64),
            (jiff::civil::Time::MIN, 0),
            (jiff::civil::time(23, 59, 59, 999_999_000), 86_399_999_999),
        ] {
            assert_eq!(
                bytes(Value::Time(Some(Box::new(value))), &Type::TIME),
                micros.to_be_bytes(),
                "binding {value}"
            );
        }

        // `timestamptz` counts from the same epoch as `timestamp`. Note that
        // `jiff::Timestamp` stops well short of the civil calendar's end —
        // it holds back jiff's largest UTC offset at each end — so
        // `9999-12-31 23:59:59.999999` has no `Timestamp` to be bound from in
        // the first place, and the naive variant above is the only route to it.
        for (value, micros) in [
            (jiff::Timestamp::MAX, 252_455_522_400_999_999i64),
            (jiff::Timestamp::MIN, -378_651_708_001_000_000),
        ] {
            assert_eq!(
                bytes(
                    Value::DateTimeWithTimeZone(Some(Box::new(value))),
                    &Type::TIMESTAMPTZ
                ),
                micros.to_be_bytes(),
                "binding {value}"
            );
        }
    }

    /// An array binds only against an array type, and its elements are held
    /// to the member type — a mismatched element is refused, not written.
    /// The non-array target used to reach a `panic!` inside `postgres-types`.
    // [spec:pgorm:req:exec.cursor.binding-accepts+4/test]
    #[test]
    fn checks_arrays_against_the_member_type() {
        use pgorm_query::ArrayType;

        let text_array = Value::Array(
            ArrayType::String,
            Some(Box::new(vec![Value::String(Some(Box::new(
                "a".to_owned(),
            )))])),
        );

        assert!(encode_as(text_array.clone(), &Type::TEXT_ARRAY).is_ok());
        assert_eq!(
            error(text_array.clone(), &Type::TEXT),
            "cannot bind a `Array` value to Postgres type `text`"
        );
        assert_eq!(
            error(text_array, &Type::INT4_ARRAY),
            "cannot bind a `String` value to Postgres type `int4`"
        );

        // The element-wise numeric coercion still applies through the member.
        let int_array = Value::Array(ArrayType::Int, Some(Box::new(vec![Value::Int(Some(2))])));
        assert!(encode_as(int_array, &Type::FLOAT8_ARRAY).is_ok());
    }

    // [spec:pgorm:req:exec.cursor.binding-coerce+2/test]
    #[test]
    fn rejects_integer_bound_outside_the_numeric_family() {
        for ty in [&Type::TEXT, &Type::BYTEA, &Type::BOOL, &Type::UUID] {
            assert_eq!(
                error(Value::Int(Some(1)), ty),
                format!("cannot bind a `Int` value to Postgres type `{ty}`")
            );
        }
        assert_eq!(
            error(Value::BigInt(Some(-1)), &Type::OID),
            "value `-1` is out of range for Postgres type `oid`"
        );
        assert_eq!(
            error(Value::Int(Some(300)), &Type::CHAR),
            format!(
                "value `300` is out of range for Postgres type `{}`",
                Type::CHAR
            )
        );
    }
}
