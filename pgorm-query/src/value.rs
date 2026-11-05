//! Container for all SQL value types.

use std::{borrow::Cow, hash::Hash};

use serde_json::Value as Json;

use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
    tz::Offset,
};

use rust_decimal::Decimal;

use uuid::Uuid;

pub use ipnetwork::IpNetwork;

use std::net::IpAddr;

pub use mac_address::MacAddress;

pub use pgvector::Vector;

use crate::{
    ArrayElement, ColumnType, Interval, Multirange, QueryBuilder, Range, RangeType, StringLen,
};

/// [`Value`] types variant for Postgres array
// [spec:pgorm:def:sql.value.array+7]
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum ArrayType {
    Bool,
    TinyInt,
    SmallInt,
    Int,
    BigInt,
    Unsigned,
    BigUnsigned,
    Float,
    Double,
    String,
    Char,
    Bytes,

    Json,

    Date,

    Time,

    DateTime,

    DateTimeWithTimeZone,

    Uuid,

    Decimal,

    IpNetwork,

    MacAddress,

    Vector,

    /// `interval[]`.
    // [spec:pgorm:def:sql.value.interval+2]
    Interval,

    /// A range over one of the built-in range types, `int4range[]`.
    // [spec:pgorm:def:sql.value.range+5]
    Range(RangeType),

    /// A multirange over one of the built-in range types, `int4multirange[]`.
    // [spec:pgorm:def:sql.value.range+5]
    Multirange(RangeType),
}

/// Value variants
///
/// We want the inner Value to be exactly 1 pointer sized, so anything larger should be boxed.
///
/// The float-carrying variants (`Float`, `Double` and `Vector`) compare by bit pattern
/// rather than by IEEE equality, which is what lets `PartialEq`, `Eq` and `Hash` agree:
/// `NaN` equals itself, and `0.0` and `-0.0` are distinct values.
// [spec:pgorm:def:sql.value+3]
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Value {
    Bool(Option<bool>),
    TinyInt(Option<i8>),
    SmallInt(Option<i16>),
    Int(Option<i32>),
    BigInt(Option<i64>),
    Unsigned(Option<u32>),
    BigUnsigned(Option<u64>),
    Float(Option<f32>),
    Double(Option<f64>),
    String(Option<Box<String>>),
    Char(Option<char>),

    #[allow(clippy::box_collection)]
    Bytes(Option<Box<Vec<u8>>>),

    Json(Option<Box<Json>>),

    Date(Option<Box<Date>>),

    Time(Option<Box<Time>>),

    DateTime(Option<Box<DateTime>>),

    /// An absolute instant, for `timestamptz`. PostgreSQL normalizes to UTC on
    /// write and never stores the offset it was given, so the value carries
    /// none either.
    DateTimeWithTimeZone(Option<Box<Timestamp>>),

    Uuid(Option<Box<Uuid>>),

    Decimal(Option<Box<Decimal>>),

    Array(ArrayType, Option<Box<Vec<Value>>>),

    Vector(Option<Box<pgvector::Vector>>),

    IpNetwork(Option<Box<IpNetwork>>),

    MacAddress(Option<Box<MacAddress>>),

    /// An `interval`, its three fields kept apart. Boxed though it holds no
    /// pointer: inline, its sixteen bytes and the `Option` around them would
    /// widen every `Value` from 24 bytes to 32.
    // [spec:pgorm:def:sql.value.interval+2]
    Interval(Option<Box<Interval>>),

    /// A range, tagged with the built-in range type it is so that a NULL and
    /// an empty range still name one. Its bounds are values of the variant
    /// the range type ranges over — `Int` for `int4range` — and a NULL bound
    /// is no bound, as it is to PostgreSQL's range constructors.
    // [spec:pgorm:def:sql.value.range+5]
    Range(RangeType, Option<Box<Range<Value>>>),

    /// A multirange, tagged as [`Value::Range`] is.
    // [spec:pgorm:def:sql.value.range+5]
    Multirange(RangeType, Option<Box<Multirange<Value>>>),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", QueryBuilder.value_to_string(self))
    }
}

// [spec:pgorm:def:sql.value.value-type+5]
pub trait ValueType: Sized {
    fn from_value(v: Value) -> Result<Self, ValueTypeError>;

    fn type_name() -> String;

    fn array_type() -> ArrayType;

    fn column_type() -> ColumnType;
}

#[derive(Debug)]
pub struct ValueTypeError;

impl std::error::Error for ValueTypeError {}

impl std::fmt::Display for ValueTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Value type mismatch")
    }
}

// [spec:pgorm:def:sql.value.tuple+3]
#[derive(Clone, Debug, PartialEq)]
pub struct Values(pub Vec<Value>);

/// An ordered tuple of values, for composite keys and VALUES lists.
///
/// One arity-agnostic representation, so a tuple of two values has exactly one
/// spelling: two constructions of the same values — one from a Rust pair, one
/// gathered from an iterator — are equal and hash alike, which is what makes it
/// sound as a `HashMap` key.
// [spec:pgorm:def:sql.value.tuple+3]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ValueTuple(Vec<Value>);

/// Why a [`ValueTuple`] could not be rebuilt into a typed tuple.
// [spec:pgorm:def:sql.value.tuple+3]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValueTupleError {
    /// The tuple's arity is not the one the target type requires.
    Arity {
        /// Number of values the target type requires.
        expected: usize,
        /// Number of values actually received.
        actual: usize,
    },
    /// A value could not be converted to the type the target holds there.
    Element {
        /// Zero-based position of the value within the tuple.
        position: usize,
        /// Name of the type expected at that position.
        expected: String,
    },
}

impl std::error::Error for ValueTupleError {}

impl std::fmt::Display for ValueTupleError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Arity { expected, actual } => {
                write!(f, "expected a tuple of arity {expected}, received {actual}")
            }
            Self::Element { position, expected } => {
                write!(
                    f,
                    "value at position {position} is not a valid `{expected}`"
                )
            }
        }
    }
}

impl ValueTuple {
    /// The number of values in this tuple.
    // [spec:pgorm:def:sql.value.tuple+3]
    pub fn arity(&self) -> usize {
        self.0.len()
    }

    /// Iterate the values in positional order.
    // [spec:pgorm:def:sql.value.tuple+3]
    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.0.iter()
    }
}

// [spec:pgorm:def:sql.value.tuple+3]
impl From<Vec<Value>> for ValueTuple {
    fn from(values: Vec<Value>) -> Self {
        Self(values)
    }
}

// [spec:pgorm:def:sql.value.tuple+3]
impl FromIterator<Value> for ValueTuple {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

pub trait IntoValueTuple {
    fn into_value_tuple(self) -> ValueTuple;
}

/// The fallible inverse of [`IntoValueTuple`].
pub trait TryFromValueTuple: Sized {
    /// Rebuild the typed tuple, reporting the shape or the position that did not match.
    fn try_from_value_tuple<I>(i: I) -> Result<Self, ValueTupleError>
    where
        I: IntoValueTuple;
}

pub trait Nullable {
    fn null() -> Value;
}

impl Value {
    /// Name of the Postgres type this value is bound as, for pinning a
    /// placeholder whose type would otherwise be inferred from context.
    ///
    /// `None` means the variant has no single type to pin it to: `Json` binds
    /// as either `json` or `jsonb`, and `Vector` binds as an extension type
    /// whose name is not guaranteed to resolve in the current search path.
    // [spec:pgorm:req:sql.render.cast-param-type+4]
    pub fn source_type_name(&self) -> Option<Cow<'static, str>> {
        match self {
            Self::Json(_) | Self::Vector(_) => None,
            Self::Array(ty, _) => ty
                .source_type_name()
                .map(|name| Cow::Owned(format!("{name}[]"))),
            Self::Bool(_) => Some(Cow::Borrowed("bool")),
            Self::TinyInt(_) => Some(Cow::Borrowed("int2")),
            Self::SmallInt(_) => Some(Cow::Borrowed("int2")),
            Self::Int(_) => Some(Cow::Borrowed("int4")),
            Self::BigInt(_) => Some(Cow::Borrowed("int8")),
            Self::Unsigned(_) => Some(Cow::Borrowed("int8")),
            Self::BigUnsigned(_) => Some(Cow::Borrowed("int8")),
            Self::Float(_) => Some(Cow::Borrowed("float4")),
            Self::Double(_) => Some(Cow::Borrowed("float8")),
            Self::String(_) => Some(Cow::Borrowed("text")),
            Self::Char(_) => Some(Cow::Borrowed("text")),
            Self::Bytes(_) => Some(Cow::Borrowed("bytea")),
            Self::Date(_) => Some(Cow::Borrowed("date")),
            Self::Time(_) => Some(Cow::Borrowed("time")),
            Self::DateTime(_) => Some(Cow::Borrowed("timestamp")),
            Self::DateTimeWithTimeZone(_) => Some(Cow::Borrowed("timestamptz")),
            Self::Uuid(_) => Some(Cow::Borrowed("uuid")),
            Self::Decimal(_) => Some(Cow::Borrowed("numeric")),
            Self::IpNetwork(_) => Some(Cow::Borrowed("inet")),
            Self::MacAddress(_) => Some(Cow::Borrowed("macaddr")),
            Self::Interval(_) => Some(Cow::Borrowed("interval")),
            Self::Range(ty, _) => Some(Cow::Borrowed(ty.range_type_name())),
            Self::Multirange(ty, _) => Some(Cow::Borrowed(ty.multirange_type_name())),
        }
    }
}

impl ArrayType {
    /// Name of the Postgres type an element of this array is bound as. See
    /// [`Value::source_type_name`].
    // [spec:pgorm:req:sql.render.cast-param-type+4]
    pub fn source_type_name(&self) -> Option<&'static str> {
        match self {
            Self::Json | Self::Vector => None,
            Self::Bool => Some("bool"),
            Self::TinyInt => Some("int2"),
            Self::SmallInt => Some("int2"),
            Self::Int => Some("int4"),
            Self::BigInt => Some("int8"),
            Self::Unsigned => Some("int8"),
            Self::BigUnsigned => Some("int8"),
            Self::Float => Some("float4"),
            Self::Double => Some("float8"),
            Self::String => Some("text"),
            Self::Char => Some("text"),
            Self::Bytes => Some("bytea"),
            Self::Date => Some("date"),
            Self::Time => Some("time"),
            Self::DateTime => Some("timestamp"),
            Self::DateTimeWithTimeZone => Some("timestamptz"),
            Self::Uuid => Some("uuid"),
            Self::Decimal => Some("numeric"),
            Self::IpNetwork => Some("inet"),
            Self::MacAddress => Some("macaddr"),
            Self::Interval => Some("interval"),
            Self::Range(ty) => Some(ty.range_type_name()),
            Self::Multirange(ty) => Some(ty.multirange_type_name()),
        }
    }
}

// [spec:pgorm:def:sql.value.conversions+1]
macro_rules! type_to_value {
    ( $type: ty, $name: ident, $col_type: expr ) => {
        impl From<$type> for Value {
            fn from(x: $type) -> Value {
                Value::$name(Some(x))
            }
        }

        impl Nullable for $type {
            fn null() -> Value {
                Value::$name(None)
            }
        }

        impl ValueType for $type {
            fn from_value(v: Value) -> Result<Self, ValueTypeError> {
                match v {
                    Value::$name(Some(x)) => Ok(x),
                    _ => Err(ValueTypeError),
                }
            }

            fn type_name() -> String {
                stringify!($type).to_owned()
            }

            fn array_type() -> ArrayType {
                ArrayType::$name
            }

            fn column_type() -> ColumnType {
                use ColumnType::*;
                $col_type
            }
        }
    };
}

macro_rules! type_to_box_value {
    ( $type: ty, $name: ident, $col_type: expr ) => {
        impl From<$type> for Value {
            fn from(x: $type) -> Value {
                Value::$name(Some(Box::new(x)))
            }
        }

        impl Nullable for $type {
            fn null() -> Value {
                Value::$name(None)
            }
        }

        impl ValueType for $type {
            fn from_value(v: Value) -> Result<Self, ValueTypeError> {
                match v {
                    Value::$name(Some(x)) => Ok(*x),
                    _ => Err(ValueTypeError),
                }
            }

            fn type_name() -> String {
                stringify!($type).to_owned()
            }

            fn array_type() -> ArrayType {
                ArrayType::$name
            }

            fn column_type() -> ColumnType {
                use ColumnType::*;
                $col_type
            }
        }
    };
}

type_to_value!(bool, Bool, Boolean);
type_to_value!(i8, TinyInt, SmallInteger);
type_to_value!(i16, SmallInt, SmallInteger);
type_to_value!(i32, Int, Integer);
type_to_value!(i64, BigInt, BigInteger);
type_to_value!(u32, Unsigned, BigInteger);
type_to_value!(u64, BigUnsigned, BigInteger);
type_to_value!(f32, Float, Float);
type_to_value!(f64, Double, Double);
type_to_value!(char, Char, Char(Some(1)));

impl From<&[u8]> for Value {
    fn from(x: &[u8]) -> Value {
        Value::Bytes(Some(Box::<Vec<u8>>::new(x.into())))
    }
}

impl From<&str> for Value {
    fn from(x: &str) -> Value {
        let string: String = x.into();
        Value::String(Some(Box::new(string)))
    }
}

impl From<&String> for Value {
    fn from(x: &String) -> Value {
        let string: String = x.into();
        Value::String(Some(Box::new(string)))
    }
}

impl Nullable for &str {
    fn null() -> Value {
        Value::String(None)
    }
}

// [spec:pgorm:def:sql.value.conversions+1]
impl<T> From<Option<T>> for Value
where
    T: Into<Value> + Nullable,
{
    fn from(x: Option<T>) -> Value {
        match x {
            Some(v) => v.into(),
            None => T::null(),
        }
    }
}

// [spec:pgorm:def:sql.value.value-type+5]
impl<T> ValueType for Option<T>
where
    T: ValueType + Nullable,
{
    fn from_value(v: Value) -> Result<Self, ValueTypeError> {
        if v == T::null() {
            Ok(None)
        } else {
            Ok(Some(T::from_value(v)?))
        }
    }

    fn type_name() -> String {
        format!("Option<{}>", T::type_name())
    }

    fn array_type() -> ArrayType {
        T::array_type()
    }

    fn column_type() -> ColumnType {
        T::column_type()
    }
}

impl From<Cow<'_, str>> for Value {
    fn from(x: Cow<'_, str>) -> Value {
        x.into_owned().into()
    }
}

impl ValueType for Cow<'_, str> {
    fn from_value(v: Value) -> Result<Self, ValueTypeError> {
        match v {
            Value::String(Some(x)) => Ok((*x).into()),
            _ => Err(ValueTypeError),
        }
    }

    fn type_name() -> String {
        "Cow<str>".into()
    }

    fn array_type() -> ArrayType {
        ArrayType::String
    }

    fn column_type() -> ColumnType {
        ColumnType::String(StringLen::None)
    }
}

type_to_box_value!(Vec<u8>, Bytes, Bytea);
type_to_box_value!(String, String, String(StringLen::None));

mod with_json {
    use super::*;

    type_to_box_value!(Json, Json, Json);
}

mod with_jiff {
    use super::*;

    type_to_box_value!(Date, Date, Date);
    type_to_box_value!(Time, Time, Time);
    type_to_box_value!(DateTime, DateTime, Timestamp);

    impl From<Timestamp> for Value {
        fn from(x: Timestamp) -> Value {
            Value::DateTimeWithTimeZone(Some(Box::new(x)))
        }
    }

    impl Nullable for Timestamp {
        fn null() -> Value {
            Value::DateTimeWithTimeZone(None)
        }
    }

    impl ValueType for Timestamp {
        fn from_value(v: Value) -> Result<Self, ValueTypeError> {
            match v {
                Value::DateTimeWithTimeZone(Some(x)) => Ok(*x),
                _ => Err(ValueTypeError),
            }
        }

        /// Spelled qualified: a bare `Timestamp` in an error message reads as
        /// PostgreSQL's `timestamp`, which is the naive type `DateTime` maps to.
        fn type_name() -> String {
            "jiff::Timestamp".to_owned()
        }

        fn array_type() -> ArrayType {
            ArrayType::DateTimeWithTimeZone
        }

        fn column_type() -> ColumnType {
            ColumnType::TimestampWithTimeZone
        }
    }
}

mod with_rust_decimal {
    use super::*;

    type_to_box_value!(Decimal, Decimal, Decimal(None));
}

mod with_uuid {
    use super::*;

    type_to_box_value!(Uuid, Uuid, Uuid);

    macro_rules! fmt_uuid_to_box_value {
        ( $type: ty, $conversion_fn: ident ) => {
            impl From<$type> for Value {
                fn from(x: $type) -> Value {
                    Value::Uuid(Some(Box::new(x.into_uuid())))
                }
            }

            impl Nullable for $type {
                fn null() -> Value {
                    Value::Uuid(None)
                }
            }

            impl ValueType for $type {
                fn from_value(v: Value) -> Result<Self, ValueTypeError> {
                    match v {
                        Value::Uuid(Some(x)) => Ok(x.$conversion_fn()),
                        _ => Err(ValueTypeError),
                    }
                }

                fn type_name() -> String {
                    stringify!($type).to_owned()
                }

                fn array_type() -> ArrayType {
                    ArrayType::Uuid
                }

                fn column_type() -> ColumnType {
                    ColumnType::Uuid
                }
            }
        };
    }

    fmt_uuid_to_box_value!(uuid::fmt::Braced, braced);
    fmt_uuid_to_box_value!(uuid::fmt::Hyphenated, hyphenated);
    fmt_uuid_to_box_value!(uuid::fmt::Simple, simple);
    fmt_uuid_to_box_value!(uuid::fmt::Urn, urn);
}

mod with_ipnetwork {
    use super::*;

    type_to_box_value!(IpNetwork, IpNetwork, Inet);
}

mod with_mac_address {
    use super::*;

    type_to_box_value!(MacAddress, MacAddress, MacAddr);
}

// [spec:pgorm:def:sql.value.array+7]
pub mod with_array {
    use super::*;
    use std::sync::Arc;

    // We only imlement conversion from Vec<T> to Array when T is not u8.
    // This is because for u8's case, there is already conversion to Byte defined above.
    // TODO When negative trait becomes a stable feature, following code can be much shorter.
    pub trait NotU8 {}

    impl NotU8 for bool {}
    impl NotU8 for i8 {}
    impl NotU8 for i16 {}
    impl NotU8 for i32 {}
    impl NotU8 for i64 {}
    impl NotU8 for u32 {}
    impl NotU8 for u64 {}
    impl NotU8 for f32 {}
    impl NotU8 for f64 {}
    impl NotU8 for char {}
    impl NotU8 for String {}
    impl NotU8 for Vec<u8> {}

    // TODO impl<T: NotU8> NotU8 for Option<T> {}

    impl NotU8 for Json {}

    impl NotU8 for Date {}

    impl NotU8 for Time {}

    impl NotU8 for DateTime {}

    impl NotU8 for Timestamp {}

    impl NotU8 for Decimal {}

    impl NotU8 for Uuid {}

    impl NotU8 for uuid::fmt::Braced {}

    impl NotU8 for uuid::fmt::Hyphenated {}

    impl NotU8 for uuid::fmt::Simple {}

    impl NotU8 for uuid::fmt::Urn {}

    impl NotU8 for IpNetwork {}

    impl NotU8 for MacAddress {}

    impl<T> From<Vec<T>> for Value
    where
        T: Into<Value> + NotU8 + ValueType,
    {
        fn from(x: Vec<T>) -> Value {
            Value::Array(
                T::array_type(),
                Some(Box::new(x.into_iter().map(|e| e.into()).collect())),
            )
        }
    }

    impl<T> Nullable for Vec<T>
    where
        T: Into<Value> + NotU8 + ValueType,
    {
        fn null() -> Value {
            Value::Array(T::array_type(), None)
        }
    }

    impl<T> ValueType for Vec<T>
    where
        T: NotU8 + ValueType,
    {
        fn from_value(v: Value) -> Result<Self, ValueTypeError> {
            match v {
                Value::Array(ty, Some(v)) if T::array_type() == ty => {
                    v.into_iter().map(T::from_value).collect()
                }
                _ => Err(ValueTypeError),
            }
        }

        fn type_name() -> String {
            stringify!(Vec<T>).to_owned()
        }

        fn array_type() -> ArrayType {
            T::array_type()
        }

        fn column_type() -> ColumnType {
            use ColumnType::*;
            Array(Arc::new(T::column_type()))
        }
    }
}

mod with_vector {
    use super::*;

    impl From<pgvector::Vector> for Value {
        fn from(x: pgvector::Vector) -> Value {
            Value::Vector(Some(Box::new(x)))
        }
    }

    impl Nullable for pgvector::Vector {
        fn null() -> Value {
            Value::Vector(None)
        }
    }

    impl ValueType for pgvector::Vector {
        fn from_value(v: Value) -> Result<Self, ValueTypeError> {
            match v {
                Value::Vector(Some(x)) => Ok(*x),
                _ => Err(ValueTypeError),
            }
        }

        fn type_name() -> String {
            stringify!(Vector).to_owned()
        }

        fn array_type() -> ArrayType {
            ArrayType::Vector
        }

        fn column_type() -> ColumnType {
            ColumnType::Vector(None)
        }
    }
}

// [spec:pgorm:sem:sql.value.accessor-panics+2]
impl Value {
    pub fn is_json(&self) -> bool {
        matches!(self, Self::Json(_))
    }

    /// The payload of a non-NULL [`Value::Json`]; `None` for a NULL of that
    /// variant and for every other variant alike. Discriminate with
    /// [`Value::is_json`].
    pub fn as_ref_json(&self) -> Option<&Json> {
        match self {
            Self::Json(v) => v.as_deref(),
            _ => None,
        }
    }
}

impl Value {
    pub fn is_date(&self) -> bool {
        matches!(self, Self::Date(_))
    }

    /// The payload of a non-NULL [`Value::Date`]; `None` otherwise.
    pub fn as_ref_date(&self) -> Option<&Date> {
        match self {
            Self::Date(v) => v.as_deref(),
            _ => None,
        }
    }
}

impl Value {
    pub fn is_time(&self) -> bool {
        matches!(self, Self::Time(_))
    }

    /// The payload of a non-NULL [`Value::Time`]; `None` otherwise.
    pub fn as_ref_time(&self) -> Option<&Time> {
        match self {
            Self::Time(v) => v.as_deref(),
            _ => None,
        }
    }
}

impl Value {
    pub fn is_date_time(&self) -> bool {
        matches!(self, Self::DateTime(_))
    }

    /// The payload of a non-NULL [`Value::DateTime`]; `None` otherwise.
    pub fn as_ref_date_time(&self) -> Option<&DateTime> {
        match self {
            Self::DateTime(v) => v.as_deref(),
            _ => None,
        }
    }
}

impl Value {
    pub fn is_date_time_with_time_zone(&self) -> bool {
        matches!(self, Self::DateTimeWithTimeZone(_))
    }

    /// The payload of a non-NULL [`Value::DateTimeWithTimeZone`]; `None`
    /// otherwise.
    pub fn as_ref_date_time_with_time_zone(&self) -> Option<&Timestamp> {
        match self {
            Self::DateTimeWithTimeZone(v) => v.as_deref(),
            _ => None,
        }
    }
}

// [spec:pgorm:sem:sql.value.accessor-panics+2]
impl Value {
    /// The UTC-naive form of any non-NULL temporal variant, stringified; `None`
    /// for a NULL temporal variant and for every non-temporal variant alike.
    ///
    /// The strings are the payloads' own ISO 8601 forms, so a date-time reads
    /// `2020-01-01T02:02:02` — `T`-separated, and never carrying an offset.
    pub fn as_naive_utc_in_string(&self) -> Option<String> {
        match self {
            Self::Date(v) => v.as_ref().map(|v| v.to_string()),
            Self::Time(v) => v.as_ref().map(|v| v.to_string()),
            Self::DateTime(v) => v.as_ref().map(|v| v.to_string()),
            // An instant becomes civil only at some offset, and UTC is the one
            // that leaves the clock reading where PostgreSQL stored it.
            Self::DateTimeWithTimeZone(v) => v
                .as_deref()
                .map(|v| Offset::UTC.to_datetime(*v).to_string()),
            _ => None,
        }
    }
}

impl Value {
    pub fn is_decimal(&self) -> bool {
        matches!(self, Self::Decimal(_))
    }

    /// The payload of a non-NULL [`Value::Decimal`]; `None` otherwise.
    pub fn as_ref_decimal(&self) -> Option<&Decimal> {
        match self {
            Self::Decimal(v) => v.as_deref(),
            _ => None,
        }
    }

    /// The payload of a non-NULL [`Value::Decimal`] as `f64`; `None` otherwise,
    /// and `None` again for a payload that has no `f64` representation.
    pub fn decimal_to_f64(&self) -> Option<f64> {
        use rust_decimal::prelude::ToPrimitive;

        self.as_ref_decimal().and_then(|d| d.to_f64())
    }
}

impl Value {
    pub fn is_uuid(&self) -> bool {
        matches!(self, Self::Uuid(_))
    }

    /// The payload of a non-NULL [`Value::Uuid`]; `None` otherwise.
    pub fn as_ref_uuid(&self) -> Option<&Uuid> {
        match self {
            Self::Uuid(v) => v.as_deref(),
            _ => None,
        }
    }
}

impl Value {
    /// One array value gathered from homogeneous elements.
    ///
    /// The element type tag is read from `V` rather than from the elements, so
    /// an empty iterator still names its element type: an untagged empty array
    /// has no inline spelling PostgreSQL can type, and no element to infer one
    /// from.
    // [spec:pgorm:def:sql.value.array+7]
    pub fn array<V, I>(values: I) -> Self
    where
        V: ArrayElement,
        I: IntoIterator<Item = V>,
    {
        Self::Array(
            V::element_type(),
            Some(Box::new(values.into_iter().map(Into::into).collect())),
        )
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Self::Array(_, _))
    }

    /// The elements of a non-NULL [`Value::Array`], whatever its element tag;
    /// `None` otherwise.
    pub fn as_ref_array(&self) -> Option<&Vec<Value>> {
        match self {
            Self::Array(_, v) => v.as_deref(),
            _ => None,
        }
    }
}

impl Value {
    pub fn is_ipnetwork(&self) -> bool {
        matches!(self, Self::IpNetwork(_))
    }

    /// The payload of a non-NULL [`Value::IpNetwork`]; `None` otherwise.
    pub fn as_ref_ipnetwork(&self) -> Option<&IpNetwork> {
        match self {
            Self::IpNetwork(v) => v.as_deref(),
            _ => None,
        }
    }

    /// The network address of a non-NULL [`Value::IpNetwork`]; `None` otherwise.
    pub fn as_ipaddr(&self) -> Option<IpAddr> {
        match self {
            Self::IpNetwork(v) => v.as_ref().map(|v| v.network()),
            _ => None,
        }
    }
}

impl Value {
    pub fn is_mac_address(&self) -> bool {
        matches!(self, Self::MacAddress(_))
    }

    /// The payload of a non-NULL [`Value::MacAddress`]; `None` otherwise.
    pub fn as_ref_mac_address(&self) -> Option<&MacAddress> {
        match self {
            Self::MacAddress(v) => v.as_deref(),
            _ => None,
        }
    }
}

impl IntoIterator for ValueTuple {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl IntoValueTuple for ValueTuple {
    fn into_value_tuple(self) -> ValueTuple {
        self
    }
}

impl<V> IntoValueTuple for V
where
    V: Into<Value>,
{
    fn into_value_tuple(self) -> ValueTuple {
        ValueTuple(vec![self.into()])
    }
}

macro_rules! impl_into_value_tuple {
    ( $($idx:tt : $T:ident),+ $(,)? ) => {
        impl< $($T),+ > IntoValueTuple for ( $($T),+ )
        where
            $($T: Into<Value>),+
        {
            fn into_value_tuple(self) -> ValueTuple {
                ValueTuple(vec![
                    $(self.$idx.into()),+
                ])
            }
        }
    };
}

#[rustfmt::skip]
mod impl_into_value_tuple {
    use super::*;

    impl_into_value_tuple!(0:T0, 1:T1);
    impl_into_value_tuple!(0:T0, 1:T1, 2:T2);
    impl_into_value_tuple!(0:T0, 1:T1, 2:T2, 3:T3);
    impl_into_value_tuple!(0:T0, 1:T1, 2:T2, 3:T3, 4:T4);
    impl_into_value_tuple!(0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5);
    impl_into_value_tuple!(0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6);
    impl_into_value_tuple!(0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6, 7:T7);
    impl_into_value_tuple!(0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6, 7:T7, 8:T8);
    impl_into_value_tuple!(0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6, 7:T7, 8:T8, 9:T9);
    impl_into_value_tuple!(0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6, 7:T7, 8:T8, 9:T9, 10:T10);
    impl_into_value_tuple!(0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6, 7:T7, 8:T8, 9:T9, 10:T10, 11:T11);
}

// [spec:pgorm:def:sql.value.tuple+3]
fn tuple_element<T>(value: Value, position: usize) -> Result<T, ValueTupleError>
where
    T: ValueType,
{
    <T as ValueType>::from_value(value).map_err(|_| ValueTupleError::Element {
        position,
        expected: T::type_name(),
    })
}

// [spec:pgorm:def:sql.value.tuple+3]
fn take_tuple_element<T>(
    iter: &mut std::vec::IntoIter<Value>,
    position: usize,
    expected: usize,
) -> Result<T, ValueTupleError>
where
    T: ValueType,
{
    match iter.next() {
        Some(value) => tuple_element(value, position),
        None => Err(ValueTupleError::Arity {
            expected,
            actual: position,
        }),
    }
}

/// The tuple's values, positionally, once its arity is the one the target type
/// requires — the single length check every [`TryFromValueTuple`] impl makes.
// [spec:pgorm:def:sql.value.tuple+3]
fn tuple_values(
    tuple: ValueTuple,
    expected: usize,
) -> Result<std::vec::IntoIter<Value>, ValueTupleError> {
    let actual = tuple.arity();
    if actual == expected {
        Ok(tuple.into_iter())
    } else {
        Err(ValueTupleError::Arity { expected, actual })
    }
}

impl<V> TryFromValueTuple for V
where
    V: Into<Value> + ValueType,
{
    // [spec:pgorm:def:sql.value.tuple+3]
    fn try_from_value_tuple<I>(i: I) -> Result<Self, ValueTupleError>
    where
        I: IntoValueTuple,
    {
        let mut values = tuple_values(i.into_value_tuple(), 1)?;
        take_tuple_element(&mut values, 0, 1)
    }
}

macro_rules! impl_try_from_value_tuple {
    ( $len:expr, $($idx:tt : $T:ident),+ $(,)? ) => {
        impl< $($T),+ > TryFromValueTuple for ( $($T),+ )
        where
            $($T: Into<Value> + ValueType),+
        {
            // [spec:pgorm:def:sql.value.tuple+3]
            fn try_from_value_tuple<Z>(i: Z) -> Result<Self, ValueTupleError>
            where
                Z: IntoValueTuple,
            {
                let mut values = tuple_values(i.into_value_tuple(), $len)?;
                Ok((
                    $(take_tuple_element::<$T>(&mut values, $idx, $len)?),+
                ))
            }
        }
    };
}

#[rustfmt::skip]
mod impl_try_from_value_tuple {
    use super::*;

    impl_try_from_value_tuple!( 2, 0:T0, 1:T1);
    impl_try_from_value_tuple!( 3, 0:T0, 1:T1, 2:T2);
    impl_try_from_value_tuple!( 4, 0:T0, 1:T1, 2:T2, 3:T3);
    impl_try_from_value_tuple!( 5, 0:T0, 1:T1, 2:T2, 3:T3, 4:T4);
    impl_try_from_value_tuple!( 6, 0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5);
    impl_try_from_value_tuple!( 7, 0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6);
    impl_try_from_value_tuple!( 8, 0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6, 7:T7);
    impl_try_from_value_tuple!( 9, 0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6, 7:T7, 8:T8);
    impl_try_from_value_tuple!(10, 0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6, 7:T7, 8:T8, 9:T9);
    impl_try_from_value_tuple!(11, 0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6, 7:T7, 8:T8, 9:T9, 10:T10);
    impl_try_from_value_tuple!(12, 0:T0, 1:T1, 2:T2, 3:T3, 4:T4, 5:T5, 6:T6, 7:T7, 8:T8, 9:T9, 10:T10, 11:T11);
}

impl Values {
    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.0.iter()
    }
}

impl IntoIterator for Values {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // [spec:pgorm:def:sql.value.conversions+1/test]
    #[test]
    fn test_value() {
        macro_rules! test_value {
            ( $type: ty, $val: literal ) => {
                let val: $type = $val;
                let v: Value = val.into();
                let out: $type = <$type as ValueType>::from_value(v).unwrap();
                assert_eq!(out, val);
            };
        }

        test_value!(u32, 4294967295);
        test_value!(i8, 127);
        test_value!(i16, 32767);
        test_value!(i32, 1073741824);
        test_value!(i64, 8589934592);
    }

    // [spec:pgorm:def:sql.value.value-type+5/test]
    #[test]
    fn test_option_value() {
        macro_rules! test_some_value {
            ( $type: ty, $val: literal ) => {
                let val: Option<$type> = Some($val);
                let v: Value = val.into();
                let out: $type = <$type as ValueType>::from_value(v).unwrap();
                assert_eq!(out, val.unwrap());
            };
        }

        macro_rules! test_none {
            ( $type: ty, $name: ident ) => {
                let val: Option<$type> = None;
                let v: Value = val.into();
                assert_eq!(v, Value::$name(None));
            };
        }

        test_some_value!(u32, 4294967295);
        test_some_value!(i8, 127);
        test_some_value!(i16, 32767);
        test_some_value!(i32, 1073741824);
        test_some_value!(i64, 8589934592);

        test_none!(u32, Unsigned);
        test_none!(i8, TinyInt);
        test_none!(i16, SmallInt);
        test_none!(i32, Int);
        test_none!(i64, BigInt);
    }

    #[test]
    fn test_cow_value() {
        let val: Cow<str> = "hello".into();
        let val2 = val.clone();
        let v: Value = val.into();
        let out: Cow<str> = <Cow<str> as ValueType>::from_value(v).unwrap();
        assert_eq!(out, val2);
    }

    #[test]
    fn test_box_value() {
        let val: String = "hello".to_owned();
        let v: Value = val.clone().into();
        let out: String = <String as ValueType>::from_value(v).unwrap();
        assert_eq!(out, val);
    }

    // [spec:pgorm:def:sql.value.tuple+3/test]
    #[test]
    fn test_value_tuple() {
        assert_eq!(
            1i32.into_value_tuple(),
            ValueTuple::from(vec![Value::Int(Some(1))])
        );
        assert_eq!(
            "b".into_value_tuple(),
            ValueTuple::from(vec![Value::String(Some(Box::new("b".to_owned())))])
        );
        assert_eq!(
            (1i32, "b").into_value_tuple(),
            ValueTuple::from(vec![
                Value::Int(Some(1)),
                Value::String(Some(Box::new("b".to_owned())))
            ])
        );
        assert_eq!(
            (1i32, 2.4f64, "b").into_value_tuple(),
            ValueTuple::from(vec![
                Value::Int(Some(1)),
                Value::Double(Some(2.4)),
                Value::String(Some(Box::new("b".to_owned())))
            ])
        );
        assert_eq!(
            (1i32, 2.4f64, "b", 123i8).into_value_tuple(),
            ValueTuple::from(vec![
                Value::Int(Some(1)),
                Value::Double(Some(2.4)),
                Value::String(Some(Box::new("b".to_owned()))),
                Value::TinyInt(Some(123))
            ])
        );
        assert_eq!(
            (1i32, 2.4f64, "b", 123i8, 456i16).into_value_tuple(),
            ValueTuple::from(vec![
                Value::Int(Some(1)),
                Value::Double(Some(2.4)),
                Value::String(Some(Box::new("b".to_owned()))),
                Value::TinyInt(Some(123)),
                Value::SmallInt(Some(456))
            ])
        );
        assert_eq!(
            (1i32, 2.4f64, "b", 123i8, 456i16, 789u32).into_value_tuple(),
            ValueTuple::from(vec![
                Value::Int(Some(1)),
                Value::Double(Some(2.4)),
                Value::String(Some(Box::new("b".to_owned()))),
                Value::TinyInt(Some(123)),
                Value::SmallInt(Some(456)),
                Value::Unsigned(Some(789))
            ])
        );
    }

    // [spec:pgorm:def:sql.value.tuple+3/test]
    #[test]
    #[allow(clippy::clone_on_copy)]
    fn test_try_from_value_tuple() {
        let mut val = 1i32;
        let original = val.clone();
        val = TryFromValueTuple::try_from_value_tuple(val).unwrap();
        assert_eq!(val, original);

        let mut val = "b".to_owned();
        let original = val.clone();
        val = TryFromValueTuple::try_from_value_tuple(val).unwrap();
        assert_eq!(val, original);

        let mut val = (1i32, "b".to_owned());
        let original = val.clone();
        val = TryFromValueTuple::try_from_value_tuple(val).unwrap();
        assert_eq!(val, original);

        let mut val = (1i32, 2.4f64, "b".to_owned());
        let original = val.clone();
        val = TryFromValueTuple::try_from_value_tuple(val).unwrap();
        assert_eq!(val, original);

        let mut val = (1i32, 2.4f64, "b".to_owned(), 123i8);
        let original = val.clone();
        val = TryFromValueTuple::try_from_value_tuple(val).unwrap();
        assert_eq!(val, original);

        let mut val = (1i32, 2.4f64, "b".to_owned(), 123i8, 456i16);
        let original = val.clone();
        val = TryFromValueTuple::try_from_value_tuple(val).unwrap();
        assert_eq!(val, original);

        let mut val = (1i32, 2.4f64, "b".to_owned(), 123i8, 456i16, 789u32);
        let original = val.clone();
        val = TryFromValueTuple::try_from_value_tuple(val).unwrap();
        assert_eq!(val, original);
    }

    // [spec:pgorm:def:sql.value.tuple+3/test]
    #[test]
    fn value_tuple_arity_counts_values() {
        assert_eq!(1i32.into_value_tuple().arity(), 1);
        assert_eq!((1i32, 2i32).into_value_tuple().arity(), 2);
        assert_eq!((1i32, 2i32, 3i32).into_value_tuple().arity(), 3);
        assert_eq!((1i32, 2i32, 3i32, 4i32).into_value_tuple().arity(), 4);
    }

    /// The dual spelling of one logical key is gone: a tuple built from a Rust
    /// pair and one gathered from an iterator of the same values are the same
    /// value, so a `HashMap` keyed on them cannot split a key in two.
    // [spec:pgorm:def:sql.value.tuple+3/test]
    #[test]
    fn value_tuples_of_equal_values_are_equal() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn hash_of(tuple: &ValueTuple) -> u64 {
            let mut hasher = DefaultHasher::new();
            tuple.hash(&mut hasher);
            hasher.finish()
        }

        let values = vec![Value::from(1i32), Value::from(2i32)];
        let from_pair = (1i32, 2i32).into_value_tuple();
        let from_iter: ValueTuple = values.iter().cloned().collect();
        let from_vec = ValueTuple::from(values);

        assert_eq!(from_pair, from_iter);
        assert_eq!(from_pair, from_vec);
        assert_eq!(hash_of(&from_pair), hash_of(&from_iter));
        assert_eq!(hash_of(&from_pair), hash_of(&from_vec));

        let four = (1i32, 2i32, 3i32, 4i32).into_value_tuple();
        let four_by_iter: ValueTuple = (1..=4).map(|n: i32| Value::from(n)).collect();
        assert_eq!(four, four_by_iter);
        assert_eq!(hash_of(&four), hash_of(&four_by_iter));
    }

    // [spec:pgorm:def:sql.value.tuple+3/test]
    #[test]
    fn try_from_value_tuple_errs_on_wrong_arity() {
        assert_eq!(
            <i32 as TryFromValueTuple>::try_from_value_tuple((1i32, 2i32)),
            Err(ValueTupleError::Arity {
                expected: 1,
                actual: 2,
            })
        );
        assert_eq!(
            <(i32, i32) as TryFromValueTuple>::try_from_value_tuple(1i32),
            Err(ValueTupleError::Arity {
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(
            <(i32, i32, i32) as TryFromValueTuple>::try_from_value_tuple((1i32, 2i32)),
            Err(ValueTupleError::Arity {
                expected: 3,
                actual: 2,
            })
        );
        assert_eq!(
            <(i32, i32, i32, i32) as TryFromValueTuple>::try_from_value_tuple((1i32, 2i32, 3i32)),
            Err(ValueTupleError::Arity {
                expected: 4,
                actual: 3,
            })
        );
        assert_eq!(
            <(i32, i32, i32, i32) as TryFromValueTuple>::try_from_value_tuple((
                1i32, 2i32, 3i32, 4i32, 5i32
            )),
            Err(ValueTupleError::Arity {
                expected: 4,
                actual: 5,
            })
        );
        assert_eq!(
            ValueTupleError::Arity {
                expected: 4,
                actual: 5,
            }
            .to_string(),
            "expected a tuple of arity 4, received 5"
        );
    }

    // [spec:pgorm:def:sql.value.tuple+3/test]
    #[test]
    fn try_from_value_tuple_errs_on_wrong_element() {
        assert_eq!(
            <String as TryFromValueTuple>::try_from_value_tuple(1i32),
            Err(ValueTupleError::Element {
                position: 0,
                expected: "String".to_owned(),
            })
        );
        assert_eq!(
            <(i32, String) as TryFromValueTuple>::try_from_value_tuple((1i32, 2i32)),
            Err(ValueTupleError::Element {
                position: 1,
                expected: "String".to_owned(),
            })
        );
        assert_eq!(
            <(i32, i32, i32, String) as TryFromValueTuple>::try_from_value_tuple((
                1i32, 2i32, 3i32, 4i32
            )),
            Err(ValueTupleError::Element {
                position: 3,
                expected: "String".to_owned(),
            })
        );
        assert_eq!(
            ValueTupleError::Arity {
                expected: 1,
                actual: 3,
            }
            .to_string(),
            "expected a tuple of arity 1, received 3"
        );
        assert_eq!(
            ValueTupleError::Element {
                position: 1,
                expected: "String".to_owned(),
            }
            .to_string(),
            "value at position 1 is not a valid `String`"
        );
    }

    #[test]
    fn test_value_tuple_iter() {
        let mut iter = (1i32).into_value_tuple().into_iter();
        assert_eq!(iter.next().unwrap(), Value::Int(Some(1)));
        assert_eq!(iter.next(), None);

        let mut iter = (1i32, 2.4f64).into_value_tuple().into_iter();
        assert_eq!(iter.next().unwrap(), Value::Int(Some(1)));
        assert_eq!(iter.next().unwrap(), Value::Double(Some(2.4)));
        assert_eq!(iter.next(), None);

        let mut iter = (1i32, 2.4f64, "b").into_value_tuple().into_iter();
        assert_eq!(iter.next().unwrap(), Value::Int(Some(1)));
        assert_eq!(iter.next().unwrap(), Value::Double(Some(2.4)));
        assert_eq!(
            iter.next().unwrap(),
            Value::String(Some(Box::new("b".to_owned())))
        );
        assert_eq!(iter.next(), None);

        let mut iter = (1i32, 2.4f64, "b", 123i8).into_value_tuple().into_iter();
        assert_eq!(iter.next().unwrap(), Value::Int(Some(1)));
        assert_eq!(iter.next().unwrap(), Value::Double(Some(2.4)));
        assert_eq!(
            iter.next().unwrap(),
            Value::String(Some(Box::new("b".to_owned())))
        );
        assert_eq!(iter.next().unwrap(), Value::TinyInt(Some(123)));
        assert_eq!(iter.next(), None);

        let mut iter = (1i32, 2.4f64, "b", 123i8, 456i16)
            .into_value_tuple()
            .into_iter();
        assert_eq!(iter.next().unwrap(), Value::Int(Some(1)));
        assert_eq!(iter.next().unwrap(), Value::Double(Some(2.4)));
        assert_eq!(
            iter.next().unwrap(),
            Value::String(Some(Box::new("b".to_owned())))
        );
        assert_eq!(iter.next().unwrap(), Value::TinyInt(Some(123)));
        assert_eq!(iter.next().unwrap(), Value::SmallInt(Some(456)));
        assert_eq!(iter.next(), None);

        let mut iter = (1i32, 2.4f64, "b", 123i8, 456i16, 789u32)
            .into_value_tuple()
            .into_iter();
        assert_eq!(iter.next().unwrap(), Value::Int(Some(1)));
        assert_eq!(iter.next().unwrap(), Value::Double(Some(2.4)));
        assert_eq!(
            iter.next().unwrap(),
            Value::String(Some(Box::new("b".to_owned())))
        );
        assert_eq!(iter.next().unwrap(), Value::TinyInt(Some(123)));
        assert_eq!(iter.next().unwrap(), Value::SmallInt(Some(456)));
        assert_eq!(iter.next().unwrap(), Value::Unsigned(Some(789)));
        assert_eq!(iter.next(), None);
    }

    #[test]

    fn test_json_value() {
        let json = serde_json::json! {{
            "a": 25.0,
            "b": "hello",
        }};
        let value: Value = json.clone().into();
        let out: Json = <Json as ValueType>::from_value(value).unwrap();
        assert_eq!(out, json);
    }

    // [spec:pgorm:def:sql.value.conversions+1/test]
    #[test]
    fn civil_date_time_round_trips_through_value() {
        let timestamp = jiff::civil::datetime(2020, 1, 1, 2, 2, 2, 0);
        let value: Value = timestamp.into();
        let out: DateTime = <DateTime as ValueType>::from_value(value).unwrap();
        assert_eq!(out, timestamp);
    }

    // [spec:pgorm:def:sql.value.conversions+1/test]
    #[test]
    fn instant_round_trips_through_value() {
        let instant: Timestamp = "2022-01-02T03:04:05Z".parse().unwrap();
        let value: Value = instant.into();
        let out: Timestamp = <Timestamp as ValueType>::from_value(value).unwrap();
        assert_eq!(out, instant);
    }

    /// A `timestamptz` is an absolute instant: PostgreSQL never stored the
    /// offset the value was written with, so the literal renders in UTC
    /// whatever offset it was parsed from.
    // [spec:pgorm:sem:sql.value.render+2/test]
    #[test]
    fn timestamptz_literal_renders_at_utc() {
        use crate::*;

        let instant: Timestamp = "2020-01-01T02:02:02+08:00".parse().unwrap();

        let query = Query::select().expr(instant).to_owned();

        assert_eq!(query.to_string(), "SELECT '2019-12-31 18:02:02 +00:00'");
    }

    /// `strftime` is lenient: a directive the payload cannot fill is copied
    /// into the output instead of failing, so a stray `%` in a rendered literal
    /// is the only evidence that a format string outran its type.
    // [spec:pgorm:sem:sql.value.render+2/test]
    #[test]
    fn temporal_literals_render_without_stray_directives() {
        let rendered = [
            Value::from(jiff::civil::date(2020, 1, 1)),
            Value::from(jiff::civil::time(2, 2, 2, 0)),
            Value::from(jiff::civil::datetime(2020, 1, 1, 2, 2, 2, 0)),
            Value::from("2020-01-01T02:02:02Z".parse::<Timestamp>().unwrap()),
        ];

        let literals = rendered.map(|v| QueryBuilder.value_to_string(&v));

        for literal in &literals {
            assert!(!literal.contains('%'), "unfilled directive in {literal}");
        }

        assert_eq!(
            literals,
            [
                "'2020-01-01'",
                "'02:02:02'",
                "'2020-01-01 02:02:02'",
                "'2020-01-01 02:02:02 +00:00'",
            ]
            .map(str::to_owned)
        );
    }

    /// A literal must carry the sub-second digits the value holds. The formats
    /// asked only for whole seconds, so `23:59:59.999999` was written into SQL
    /// as `23:59:59` — silently, and at every magnitude rather than only at a
    /// boundary. `%.f` omits the fraction when it is zero, which is why the
    /// whole-second literals above are unchanged.
    // [spec:pgorm:sem:sql.value.render+2/test]
    #[test]
    fn temporal_literals_keep_sub_second_digits() {
        let literals = [
            Value::from(jiff::civil::time(23, 59, 59, 999_999_000)),
            Value::from(jiff::civil::datetime(9999, 12, 31, 23, 59, 59, 999_999_000)),
            Value::from("2020-01-01T02:02:02.123456Z".parse::<Timestamp>().unwrap()),
        ]
        .map(|v| QueryBuilder.value_to_string(&v));

        assert_eq!(
            literals,
            [
                "'23:59:59.999999'",
                "'9999-12-31 23:59:59.999999'",
                "'2020-01-01 02:02:02.123456 +00:00'",
            ]
            .map(str::to_owned)
        );

        // PostgreSQL stores microseconds and the parameter path discards the
        // rest, so a literal truncates too rather than leaving the server to
        // round a digit the bound form would have dropped.
        let nanos = Value::from(jiff::civil::time(1, 2, 3, 123_456_789));
        assert_eq!(QueryBuilder.value_to_string(&nanos), "'01:02:03.123456'");
    }

    // [spec:pgorm:def:sql.value.conversions+1/test]
    #[test]
    fn test_uuid_value() {
        let uuid = Uuid::parse_str("936DA01F9ABD4d9d80C702AF85C822A8").unwrap();
        let value: Value = uuid.into();
        let out: Uuid = <Uuid as ValueType>::from_value(value).unwrap();
        assert_eq!(out, uuid);

        let uuid_braced = uuid.braced();
        let value: Value = uuid_braced.into();
        let out: Uuid = <Uuid as ValueType>::from_value(value).unwrap();
        assert_eq!(out, uuid);

        let uuid_hyphenated = uuid.hyphenated();
        let value: Value = uuid_hyphenated.into();
        let out: Uuid = <Uuid as ValueType>::from_value(value).unwrap();
        assert_eq!(out, uuid);

        let uuid_simple = uuid.simple();
        let value: Value = uuid_simple.into();
        let out: Uuid = <Uuid as ValueType>::from_value(value).unwrap();
        assert_eq!(out, uuid);

        let uuid_urn = uuid.urn();
        let value: Value = uuid_urn.into();
        let out: Uuid = <Uuid as ValueType>::from_value(value).unwrap();
        assert_eq!(out, uuid);
    }

    #[test]

    fn test_decimal_value() {
        use std::str::FromStr;

        let num = "2.02";
        let val = Decimal::from_str(num).unwrap();
        let v: Value = val.into();
        let out: Decimal = <Decimal as ValueType>::from_value(v).unwrap();
        assert_eq!(out.to_string(), num);
    }

    // [spec:pgorm:def:sql.value.array+7/test]
    #[test]
    fn test_array_value() {
        let array = vec![1, 2, 3, 4, 5];
        let v: Value = array.into();
        let out: Vec<i32> = <Vec<i32> as ValueType>::from_value(v).unwrap();
        assert_eq!(out, vec![1, 2, 3, 4, 5]);
    }

    // [spec:pgorm:def:sql.value.array+7/test]
    #[test]
    fn test_option_array_value() {
        let v: Value = Value::Array(ArrayType::Int, None);
        let out: Option<Vec<i32>> = <Option<Vec<i32>> as ValueType>::from_value(v).unwrap();
        assert_eq!(out, None);
    }

    // [spec:pgorm:def:sql.value.array+7/test]
    #[test]
    fn vector_has_an_array_type_tag() {
        assert_eq!(<Vector as ValueType>::array_type(), ArrayType::Vector);
        assert_eq!(ArrayType::Vector.source_type_name(), None);
    }
}
