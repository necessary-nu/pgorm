//! The type a cast to a column's type names. `ColumnType`'s file is over its
//! function cap, so the method sits beside it here.

use crate::types::{Name, TypeName};

use super::{ColumnType, IntervalSpec, StringLen};

impl ColumnType {
    /// The type as a cast names it: a built-in type by its one-word catalogue
    /// spelling with its length or precision as modifiers (`varchar(20)`,
    /// `numeric(10, 2)`, `bit(7)`, `float8`, `timestamptz`), a named type by
    /// its qualified name, and an array of either as the array of it. `None`
    /// for a type no name and modifiers spell: an interval restricted to
    /// fields, and the unlimited `bit`, whose only spelling is the quoted
    /// catalogue name.
    ///
    /// ```
    /// use pgorm_query::{ColumnType, StringLen};
    ///
    /// let cast = |column_type: ColumnType| column_type.cast_type().map(|t| t.to_sql_string());
    /// assert_eq!(cast(ColumnType::Bit(Some(7))).as_deref(), Some("bit(7)"));
    /// assert_eq!(cast(ColumnType::Decimal(Some((10, 2)))).as_deref(), Some("numeric(10, 2)"));
    /// assert_eq!(cast(ColumnType::String(StringLen::N(20))).as_deref(), Some("varchar(20)"));
    /// assert_eq!(cast(ColumnType::Bit(None)), None);
    /// ```
    // [spec:pgorm:def:sql.types.type-name+9]
    pub fn cast_type(&self) -> Option<TypeName> {
        let named = |name: &str| TypeName::new(Name::runtime(name));
        let sized = |name: &str, size: Option<u32>| match size {
            Some(size) => named(name).modifiers([size]),
            None => named(name),
        };
        Some(match self {
            ColumnType::Char(length) => match length {
                Some(length) => named("char").modifiers([*length]),
                None => named("bpchar"),
            },
            ColumnType::String(StringLen::N(length)) => named("varchar").modifiers([*length]),
            ColumnType::String(_) => named("varchar"),
            ColumnType::Text => named("text"),
            ColumnType::Bytea => named("bytea"),
            ColumnType::SmallInteger => named("smallint"),
            ColumnType::Integer => named("integer"),
            ColumnType::BigInteger => named("bigint"),
            ColumnType::Float => named("real"),
            ColumnType::Double => named("float8"),
            ColumnType::Decimal(Some((precision, scale))) => {
                named("numeric").modifiers([*precision, *scale])
            }
            ColumnType::Decimal(None) => named("numeric"),
            ColumnType::Timestamp => named("timestamp"),
            ColumnType::TimestampWithTimeZone => named("timestamptz"),
            ColumnType::Time => named("time"),
            ColumnType::Date => named("date"),
            ColumnType::Interval(IntervalSpec::Any(precision)) => {
                sized("interval", precision.map(|p| u32::from(p.digits())))
            }
            ColumnType::Interval(IntervalSpec::Fields(_)) | ColumnType::Bit(None) => return None,
            ColumnType::Bit(length) => sized("bit", *length),
            ColumnType::VarBit(length) => named("varbit").modifiers([*length]),
            ColumnType::Boolean => named("bool"),
            ColumnType::Money => named("money"),
            ColumnType::Json => named("json"),
            ColumnType::JsonBinary => named("jsonb"),
            ColumnType::Uuid => named("uuid"),
            ColumnType::Vector(size) => sized("vector", *size),
            ColumnType::Cidr => named("cidr"),
            ColumnType::Inet => named("inet"),
            ColumnType::MacAddr => named("macaddr"),
            ColumnType::LTree => named("ltree"),
            ColumnType::Range(range) => named(range.range_type_name()),
            ColumnType::Multirange(range) => named(range.multirange_type_name()),
            ColumnType::Named(type_name) => type_name.clone(),
            ColumnType::Enum { name, schema, .. }
            | ColumnType::CreatedRange { name, schema, .. }
            | ColumnType::CreatedMultirange { name, schema, .. }
            | ColumnType::Domain { name, schema, .. } => {
                let mut type_name = TypeName::new(Name::clone(name));
                type_name.schema = schema.clone();
                type_name
            }
            ColumnType::Array(element) => element.cast_type()?.array(),
        })
    }
}
