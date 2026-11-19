//! The types `JSON_VALUE` can return its scalar as.

use crate::{ColumnType, IntervalSpec, Name, RangeType, StringLen, TypeName};
use std::sync::Arc;

/// The type [`JsonValue::returning`](crate::JsonValue::returning) returns the
/// scalar as: [`ColumnType`]'s vocabulary without `Json` and `JsonBinary`,
/// each variant converting into the [`ColumnType`] of the same name.
///
/// `json` and `jsonb` are left out because PostgreSQL 18.6 returns them
/// wrongly. Once one evaluation of a `JSON_VALUE .. RETURNING json` or
/// `RETURNING jsonb` is `NULL` — a JSON `null`, a path that finds nothing or
/// fails, a `NULL` context item — every later evaluation of that expression
/// in the statement is `NULL` too (PostgreSQL bug #19695): the executor
/// leaves its result flagged null and skips the conversion. Every other type
/// is converted by a step that resets the flag. PostgreSQL 18.7 and 19.0
/// carry the fix. To read JSON out of a
/// document, [`Func::json_query`](crate::Func::json_query) returns what the
/// path finds, and differs from such a `JSON_VALUE` only in returning a JSON
/// `null` as itself and an object or array rather than failing.
///
/// No variant spells either type:
///
/// ```compile_fail,E0599
/// use pgorm_query::*;
///
/// Func::json_value(Expr::col(Name::runtime("doc")), "$.a").returning(JsonValueType::JsonBinary);
/// ```
///
/// and a [`ColumnType`] chosen at run time converts with `try_from`, which
/// hands either back:
///
/// ```
/// use pgorm_query::*;
///
/// assert_eq!(
///     JsonValueType::try_from(ColumnType::JsonBinary),
///     Err(ColumnType::JsonBinary)
/// );
/// assert_eq!(
///     JsonValueType::try_from(ColumnType::Integer),
///     Ok(JsonValueType::Integer)
/// );
/// ```
///
/// [`Named`](Self::Named) is a catalogue name the crate does not interpret,
/// so `json` or `jsonb` spelled that way is not refused and meets the defect.
// [spec:pgorm:def:sql.ast.expr.sql-json+2]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum JsonValueType {
    Char(Option<u32>),
    String(StringLen),
    Text,
    Bytea,
    SmallInteger,
    Integer,
    BigInteger,
    Float,
    Double,
    Decimal(Option<(u32, u32)>),
    Timestamp,
    TimestampWithTimeZone,
    Time,
    Date,
    Interval(IntervalSpec),
    Bit(Option<u32>),
    VarBit(u32),
    Boolean,
    Money,
    Uuid,
    Named(TypeName),
    Enum {
        name: Name,
        schema: Option<Name>,
        variants: Vec<Name>,
    },
    /// An array, read from a JSON string in the array's text form. Its
    /// element may be `json` or `jsonb`: an array is converted by the step
    /// that resets the flag.
    Array(Arc<ColumnType>),
    Vector(Option<u32>),
    Cidr,
    Inet,
    MacAddr,
    LTree,
    Range(RangeType),
    Multirange(RangeType),
    CreatedRange {
        name: Name,
        schema: Option<Name>,
        subtype: Arc<ColumnType>,
    },
    CreatedMultirange {
        name: Name,
        schema: Option<Name>,
        subtype: Arc<ColumnType>,
    },
    /// A domain, whose conversion clears the flag whatever its base, a
    /// domain over `jsonb` included.
    Domain {
        name: Name,
        schema: Option<Name>,
        base: Arc<ColumnType>,
    },
}

impl From<JsonValueType> for ColumnType {
    fn from(ty: JsonValueType) -> Self {
        match ty {
            JsonValueType::Char(n) => Self::Char(n),
            JsonValueType::String(len) => Self::String(len),
            JsonValueType::Text => Self::Text,
            JsonValueType::Bytea => Self::Bytea,
            JsonValueType::SmallInteger => Self::SmallInteger,
            JsonValueType::Integer => Self::Integer,
            JsonValueType::BigInteger => Self::BigInteger,
            JsonValueType::Float => Self::Float,
            JsonValueType::Double => Self::Double,
            JsonValueType::Decimal(precision) => Self::Decimal(precision),
            JsonValueType::Timestamp => Self::Timestamp,
            JsonValueType::TimestampWithTimeZone => Self::TimestampWithTimeZone,
            JsonValueType::Time => Self::Time,
            JsonValueType::Date => Self::Date,
            JsonValueType::Interval(spec) => Self::Interval(spec),
            JsonValueType::Bit(n) => Self::Bit(n),
            JsonValueType::VarBit(n) => Self::VarBit(n),
            JsonValueType::Boolean => Self::Boolean,
            JsonValueType::Money => Self::Money,
            JsonValueType::Uuid => Self::Uuid,
            JsonValueType::Named(name) => Self::Named(name),
            JsonValueType::Enum {
                name,
                schema,
                variants,
            } => Self::Enum {
                name,
                schema,
                variants,
            },
            JsonValueType::Array(element) => Self::Array(element),
            JsonValueType::Vector(n) => Self::Vector(n),
            JsonValueType::Cidr => Self::Cidr,
            JsonValueType::Inet => Self::Inet,
            JsonValueType::MacAddr => Self::MacAddr,
            JsonValueType::LTree => Self::LTree,
            JsonValueType::Range(range) => Self::Range(range),
            JsonValueType::Multirange(range) => Self::Multirange(range),
            JsonValueType::CreatedRange {
                name,
                schema,
                subtype,
            } => Self::CreatedRange {
                name,
                schema,
                subtype,
            },
            JsonValueType::CreatedMultirange {
                name,
                schema,
                subtype,
            } => Self::CreatedMultirange {
                name,
                schema,
                subtype,
            },
            JsonValueType::Domain { name, schema, base } => Self::Domain { name, schema, base },
        }
    }
}

/// Every [`ColumnType`] but `json` and `jsonb`, which come back as the error.
impl TryFrom<ColumnType> for JsonValueType {
    type Error = ColumnType;

    fn try_from(ty: ColumnType) -> Result<Self, ColumnType> {
        Ok(match ty {
            ColumnType::Json | ColumnType::JsonBinary => return Err(ty),
            ColumnType::Char(n) => Self::Char(n),
            ColumnType::String(len) => Self::String(len),
            ColumnType::Text => Self::Text,
            ColumnType::Bytea => Self::Bytea,
            ColumnType::SmallInteger => Self::SmallInteger,
            ColumnType::Integer => Self::Integer,
            ColumnType::BigInteger => Self::BigInteger,
            ColumnType::Float => Self::Float,
            ColumnType::Double => Self::Double,
            ColumnType::Decimal(precision) => Self::Decimal(precision),
            ColumnType::Timestamp => Self::Timestamp,
            ColumnType::TimestampWithTimeZone => Self::TimestampWithTimeZone,
            ColumnType::Time => Self::Time,
            ColumnType::Date => Self::Date,
            ColumnType::Interval(spec) => Self::Interval(spec),
            ColumnType::Bit(n) => Self::Bit(n),
            ColumnType::VarBit(n) => Self::VarBit(n),
            ColumnType::Boolean => Self::Boolean,
            ColumnType::Money => Self::Money,
            ColumnType::Uuid => Self::Uuid,
            ColumnType::Named(name) => Self::Named(name),
            ColumnType::Enum {
                name,
                schema,
                variants,
            } => Self::Enum {
                name,
                schema,
                variants,
            },
            ColumnType::Array(element) => Self::Array(element),
            ColumnType::Vector(n) => Self::Vector(n),
            ColumnType::Cidr => Self::Cidr,
            ColumnType::Inet => Self::Inet,
            ColumnType::MacAddr => Self::MacAddr,
            ColumnType::LTree => Self::LTree,
            ColumnType::Range(range) => Self::Range(range),
            ColumnType::Multirange(range) => Self::Multirange(range),
            ColumnType::CreatedRange {
                name,
                schema,
                subtype,
            } => Self::CreatedRange {
                name,
                schema,
                subtype,
            },
            ColumnType::CreatedMultirange {
                name,
                schema,
                subtype,
            } => Self::CreatedMultirange {
                name,
                schema,
                subtype,
            },
            ColumnType::Domain { name, schema, base } => Self::Domain { name, schema, base },
        })
    }
}
