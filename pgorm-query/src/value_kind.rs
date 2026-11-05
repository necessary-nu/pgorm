//! The kinds of value pgorm holds, each by the one name pgorm's language
//! bindings and its generative campaign spell it by.

use crate::{ArrayType, ColumnType, IntervalSpec, Name, RangeType, TypeName, Value};

/// One table of every kind: each scalar tag beside its name, and each
/// built-in range type beside the kind its bounds are. Every function below
/// is generated from it and matches its enum without a wildcard arm, so a
/// variant `ArrayType`, `Value` or `RangeType` gains does not build until it
/// is named here. A variant carrying a payload, as `Range` and `Multirange`
/// do, is written out in each function and in `kinds` instead.
macro_rules! value_kinds {
    (
        scalars { $( $scalar:ident => $name:literal ),+ $(,)? }
        ranges { $( $range:ident => $element:ident ),+ $(,)? }
    ) => {
        const SCALARS: &[ArrayType] = &[$(ArrayType::$scalar),+];

        const RANGE_TYPES: &[RangeType] = &[$(RangeType::$range),+];

        // [spec:pgorm:def:sql.value.kinds]
        impl ArrayType {
            /// The kind's name: `bool`, `i8`, `i16`, `i32`, `i64`, `u32`,
            /// `u64`, `f32`, `f64`, `text`, `char`, `bytes`, `json`,
            /// `decimal`, `uuid`, `date`, `time`, `datetime`,
            /// `datetime_utc`, `ipnetwork`, `mac_address`, `vector` or
            /// `interval`, and a range or a multirange by its catalogue
            /// name, `int4range` or `tstzmultirange`.
            pub fn kind_name(&self) -> &'static str {
                match self {
                    $(Self::$scalar => $name,)+
                    Self::Range(range) => range.range_type_name(),
                    Self::Multirange(range) => range.multirange_type_name(),
                }
            }

            /// The kind named `name` exactly as
            /// [`kind_name`](Self::kind_name) spells it. A catalogue name
            /// of a scalar type, `int4` or `timestamptz`, names no kind.
            pub fn from_kind_name(name: &str) -> Option<Self> {
                Self::kinds().find(|kind| kind.kind_name() == name)
            }

            /// Every kind once: the scalar tags, then each built-in range
            /// type's range followed by its multirange.
            pub fn kinds() -> impl Iterator<Item = Self> {
                SCALARS.iter().cloned().chain(
                    RANGE_TYPES
                        .iter()
                        .flat_map(|range| [Self::Range(*range), Self::Multirange(*range)]),
                )
            }

            /// SQL NULL of this kind.
            pub fn null(&self) -> Value {
                match self {
                    $(Self::$scalar => Value::$scalar(None),)+
                    Self::Range(range) => Value::Range(*range, None),
                    Self::Multirange(range) => Value::Multirange(*range, None),
                }
            }
        }

        // [spec:pgorm:def:sql.value.kinds]
        impl RangeType {
            /// The kind a range of this type is bounded by: `Int` for
            /// `int4range`, `DateTimeWithTimeZone` for `tstzrange`.
            pub fn element_type(self) -> ArrayType {
                match self {
                    $(Self::$range => ArrayType::$element,)+
                }
            }
        }

        // [spec:pgorm:def:sql.value.kinds]
        impl Value {
            /// The kind this value is, as the tag an array of it carries; an
            /// array's is the tag its elements carry, and
            /// [`is_array`](Self::is_array) tells the two apart.
            pub fn array_type(&self) -> ArrayType {
                match self {
                    $(Self::$scalar(_) => ArrayType::$scalar,)+
                    Self::Array(kind, _) => kind.clone(),
                    Self::Range(range, _) => ArrayType::Range(*range),
                    Self::Multirange(range, _) => ArrayType::Multirange(*range),
                }
            }

            /// Whether this is SQL NULL. JSON's `null` is a present
            /// `Value::Json`, and an empty array a present `Value::Array`.
            pub fn is_null(&self) -> bool {
                match self {
                    $(Self::$scalar(value) => value.is_none(),)+
                    Self::Array(_, value) => value.is_none(),
                    Self::Range(_, value) => value.is_none(),
                    Self::Multirange(_, value) => value.is_none(),
                }
            }
        }
    };
}

value_kinds! {
    scalars {
        Bool => "bool", TinyInt => "i8", SmallInt => "i16", Int => "i32",
        BigInt => "i64", Unsigned => "u32", BigUnsigned => "u64",
        Float => "f32", Double => "f64", String => "text", Char => "char",
        Bytes => "bytes", Json => "json", Decimal => "decimal", Uuid => "uuid",
        Date => "date", Time => "time", DateTime => "datetime",
        DateTimeWithTimeZone => "datetime_utc", IpNetwork => "ipnetwork",
        MacAddress => "mac_address", Vector => "vector", Interval => "interval",
    }
    ranges {
        Int4 => Int, Int8 => BigInt, Numeric => Decimal, Date => Date,
        Timestamp => DateTime, TimestampTz => DateTimeWithTimeZone,
    }
}

// [spec:pgorm:def:sql.value.kinds]
impl ArrayType {
    /// The type of a column whose values a result decodes as this kind,
    /// which a table described by kinds alone is written with: the one
    /// PostgreSQL type that decodes as the kind, or where several do, the one
    /// a schema writes for it — `"char"` for `i8`, `oid` for `u32`, `text` for
    /// `text`, `jsonb` for `json` and `inet` for `ipnetwork`. `u64` and
    /// `char` are `None`, since no column decodes as either.
    // [spec:pgorm:req:bindings.disjoint+2]
    pub fn column_type(&self) -> Option<ColumnType> {
        Some(match self {
            Self::Bool => ColumnType::Boolean,
            // Bare, `char` is `character(1)`; quoted, it is the one-byte type.
            Self::TinyInt => ColumnType::Named(TypeName::raw("\"char\"")),
            Self::SmallInt => ColumnType::SmallInteger,
            Self::Int => ColumnType::Integer,
            Self::BigInt => ColumnType::BigInteger,
            Self::Unsigned => ColumnType::Named(TypeName::new(Name::runtime("oid"))),
            Self::BigUnsigned | Self::Char => return None,
            Self::Float => ColumnType::Float,
            Self::Double => ColumnType::Double,
            Self::String => ColumnType::Text,
            Self::Bytes => ColumnType::Bytea,
            Self::Json => ColumnType::JsonBinary,
            Self::Decimal => ColumnType::Decimal(None),
            Self::Uuid => ColumnType::Uuid,
            Self::Date => ColumnType::Date,
            Self::Time => ColumnType::Time,
            Self::DateTime => ColumnType::Timestamp,
            Self::DateTimeWithTimeZone => ColumnType::TimestampWithTimeZone,
            Self::IpNetwork => ColumnType::Inet,
            Self::MacAddress => ColumnType::MacAddr,
            Self::Vector => ColumnType::Vector(None),
            Self::Interval => ColumnType::Interval(IntervalSpec::Any(None)),
            Self::Range(range) => ColumnType::Range(*range),
            Self::Multirange(range) => ColumnType::Multirange(*range),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::Range;

    // [spec:pgorm:def:sql.value.kinds/test]
    #[test]
    fn each_kind_reads_back_from_its_one_name() {
        let kinds: Vec<ArrayType> = ArrayType::kinds().collect();
        assert_eq!(kinds.len(), 23 + 6 + 6);
        let names: BTreeSet<&str> = kinds.iter().map(ArrayType::kind_name).collect();
        assert_eq!(names.len(), kinds.len(), "no two kinds share a name");
        for kind in &kinds {
            assert_eq!(
                ArrayType::from_kind_name(kind.kind_name()).as_ref(),
                Some(kind)
            );
        }
        assert_eq!(ArrayType::DateTimeWithTimeZone.kind_name(), "datetime_utc");
        assert_eq!(
            ArrayType::Multirange(RangeType::TimestampTz).kind_name(),
            "tstzmultirange"
        );
        for unknown in ["", "int4", "timestamptz", "enum", "array", "I32", "i32 "] {
            assert_eq!(ArrayType::from_kind_name(unknown), None, "{unknown:?}");
        }
    }

    // [spec:pgorm:def:sql.value.kinds/test]
    #[test]
    fn each_decoded_kind_names_a_column_type() {
        let written = |kind: ArrayType| {
            let column_type = kind.column_type()?;
            let sql = crate::Table::create(Name::runtime("t"))
                .col(crate::ColumnDef::new_with_type(
                    Name::runtime("c"),
                    column_type,
                ))
                .to_string();
            Some(
                sql.trim_start_matches(r#"CREATE TABLE "t" ( "c" "#)
                    .trim_end_matches(" )")
                    .to_owned(),
            )
        };
        let types: Vec<(&str, Option<String>)> = ArrayType::kinds()
            .map(|kind| (kind.kind_name(), written(kind)))
            .collect();
        let expected = [
            ("bool", "bool"),
            ("i8", "\"char\""),
            ("i16", "smallint"),
            ("i32", "integer"),
            ("i64", "bigint"),
            ("u32", "oid"),
            ("f32", "real"),
            ("f64", "double precision"),
            ("text", "text"),
            ("bytes", "bytea"),
            ("json", "jsonb"),
            ("decimal", "decimal"),
            ("uuid", "uuid"),
            ("date", "date"),
            ("time", "time"),
            ("datetime", "timestamp"),
            ("datetime_utc", "timestamp with time zone"),
            ("ipnetwork", "inet"),
            ("mac_address", "macaddr"),
            ("vector", "vector"),
            ("interval", "interval"),
            ("int4range", "int4range"),
            ("int4multirange", "int4multirange"),
        ];
        for (kind, sql) in expected {
            let found = types.iter().find(|(name, _)| *name == kind);
            assert_eq!(
                found.and_then(|(_, sql)| sql.as_deref()),
                Some(sql),
                "{kind}"
            );
        }
        for kind in ["u64", "char"] {
            assert_eq!(
                types
                    .iter()
                    .find(|(name, _)| *name == kind)
                    .map(|(_, sql)| sql.clone()),
                Some(None),
                "{kind}"
            );
        }
    }

    // [spec:pgorm:def:sql.value.kinds/test]
    #[test]
    fn a_kinds_null_is_that_kind_and_null() {
        for kind in ArrayType::kinds() {
            let null = kind.null();
            assert!(null.is_null(), "{kind:?}");
            assert!(!null.is_array(), "{kind:?}");
            assert_eq!(null.array_type(), kind);
        }
    }

    // [spec:pgorm:def:sql.value.kinds/test]
    #[test]
    fn a_value_names_its_kind_and_its_nullness() {
        let array = Value::Array(ArrayType::Int, Some(Box::new(vec![Value::Int(None)])));
        assert_eq!(array.array_type(), ArrayType::Int);
        assert!(array.is_array() && !array.is_null());
        assert!(Value::Array(ArrayType::Uuid, None).is_null());
        let empty = Value::Array(ArrayType::Bool, Some(Box::new(Vec::new())));
        assert!(!empty.is_null());
        assert!(!Value::Json(Some(Box::new(serde_json::Value::Null))).is_null());
        assert!(!Value::Int(Some(0)).is_null());
        let range = Value::Range(RangeType::Date, Some(Box::new(Range::Empty)));
        assert_eq!(range.array_type(), ArrayType::Range(RangeType::Date));
        assert!(!range.is_null());
    }

    // [spec:pgorm:def:sql.value.kinds/test]
    #[test]
    fn a_range_types_bounds_are_its_element_kind() {
        let elements: Vec<ArrayType> = RANGE_TYPES
            .iter()
            .map(|range| range.element_type())
            .collect();
        assert_eq!(
            elements,
            [
                ArrayType::Int,
                ArrayType::BigInt,
                ArrayType::Decimal,
                ArrayType::Date,
                ArrayType::DateTime,
                ArrayType::DateTimeWithTimeZone,
            ]
        );
    }
}
