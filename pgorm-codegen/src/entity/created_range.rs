use pgorm_query::{ColumnType, Name};

/// A range type a schema created, or the multirange created beside it, as
/// the newtype the generated entities name it by: one per type, as an enum
/// type is one `ActiveEnum`.
#[derive(Clone, Debug)]
pub struct CreatedRangeType {
    pub(crate) name: Name,
    pub(crate) schema: Option<Name>,
    pub(crate) subtype: ColumnType,
    /// Whether this is the multirange, held as a `Multirange<T>`, rather than
    /// the range, held as a `Range<T>`.
    pub(crate) multirange: bool,
}

/// The Rust type a range type created over `subtype` ranges over, as the
/// prelude names it — `None` for a subtype pgorm's `RangeSubtype` does not
/// cover, which no newtype can hold.
// [spec:pgorm:sem:codegen.entity.types+13]
pub(crate) fn created_range_subtype(subtype: &ColumnType) -> Option<&'static str> {
    Some(match subtype {
        ColumnType::SmallInteger => "i16",
        ColumnType::Integer => "i32",
        ColumnType::BigInteger => "i64",
        ColumnType::Float => "f32",
        ColumnType::Double => "f64",
        ColumnType::Decimal(_) => "Decimal",
        ColumnType::Char(_) | ColumnType::String(_) | ColumnType::Text => "String",
        ColumnType::Date => "Date",
        ColumnType::Time => "Time",
        ColumnType::Timestamp => "DateTime",
        ColumnType::TimestampWithTimeZone => "DateTimeWithTimeZone",
        ColumnType::Uuid => "Uuid",
        _ => return None,
    })
}
