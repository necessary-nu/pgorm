//! The SQL types a binding's `DataType` names, as pgorm-query's
//! `ColumnType`: a built-in type by name with the size it takes, an enum by
//! its type's name, or a range type a schema created.

use std::sync::Arc;

use pgorm::pgorm_query::{ArrayType, ColumnType, IntervalSpec, Name, StringLen};

use crate::{CreatedKind, Refusal, TypeName};

/// The built-in type `name`, sized as `length`, `precision` and `scale` say,
/// each refused where the type takes none; `Ok(None)` for a name no built-in
/// type has.
// [spec:pgorm:req:bindings.schema]
// [spec:pgorm:req:bindings.sql-json]
pub fn builtin_column(
    name: &str,
    length: Option<u32>,
    precision: Option<u32>,
    scale: Option<u32>,
) -> Result<Option<ColumnType>, Refusal> {
    let refused = |reason: String| Err(Refusal(reason));
    if length == Some(0) {
        return refused("a type's length is a whole number of at least 1".to_owned());
    }
    if length.is_some() && !matches!(name, "char" | "varchar" | "bit" | "varbit" | "vector") {
        return refused(format!("{name} takes no length"));
    }
    if (precision.is_some() || scale.is_some()) && name != "numeric" {
        return refused("precision and scale belong to numeric".to_owned());
    }
    Ok(Some(match name {
        // SQL's `char` without a length is `char(1)`; `Char(None)` is the
        // unbounded `bpchar`.
        "char" => ColumnType::Char(Some(length.unwrap_or(1))),
        "varchar" => ColumnType::String(length.map_or(StringLen::None, StringLen::N)),
        "text" => ColumnType::Text,
        "smallint" => ColumnType::SmallInteger,
        "integer" => ColumnType::Integer,
        "bigint" => ColumnType::BigInteger,
        "real" => ColumnType::Float,
        "double" => ColumnType::Double,
        "numeric" => ColumnType::Decimal(match (precision, scale) {
            (None, None) => None,
            (Some(precision @ 1..=1000), scale) if scale.unwrap_or(0) <= 1000 => {
                Some((precision, scale.unwrap_or(0)))
            }
            _ => {
                return refused(
                    "numeric takes a precision of 1–1000 and a scale of 0–1000".to_owned(),
                );
            }
        }),
        "boolean" => ColumnType::Boolean,
        "date" => ColumnType::Date,
        "time" => ColumnType::Time,
        "timestamp" => ColumnType::Timestamp,
        "timestamptz" => ColumnType::TimestampWithTimeZone,
        "interval" => ColumnType::Interval(IntervalSpec::Any(None)),
        "bytea" => ColumnType::Bytea,
        // SQL's `bit` without a length is `bit(1)`; `Bit(None)` is the bit
        // type with no length limit.
        "bit" => ColumnType::Bit(Some(length.unwrap_or(1))),
        "varbit" => match length {
            Some(length) => ColumnType::VarBit(length),
            None => return refused("varbit takes a length".to_owned()),
        },
        "money" => ColumnType::Money,
        "json" => ColumnType::Json,
        "jsonb" => ColumnType::JsonBinary,
        "uuid" => ColumnType::Uuid,
        "vector" => ColumnType::Vector(length),
        "cidr" => ColumnType::Cidr,
        "inet" => ColumnType::Inet,
        "macaddr" => ColumnType::MacAddr,
        "ltree" => ColumnType::LTree,
        _ => match ArrayType::from_kind_name(name) {
            Some(ArrayType::Range(range)) => ColumnType::Range(range),
            Some(ArrayType::Multirange(range)) => ColumnType::Multirange(range),
            _ => return Ok(None),
        },
    }))
}

/// Why `name` is refused as a `DataType`'s built-in name; the binding
/// raises it as its construction error.
pub fn unknown_type(name: &str) -> Refusal {
    Refusal(format!("{name:?} is no built-in type a DataType names"))
}

/// The refusal of a size given a type named by `TypeName` or a created
/// range, which takes none.
pub fn sized_named_type() -> Refusal {
    Refusal("a named type takes no size".to_owned())
}

/// The enum type `name` names, its labels left empty because a column or a
/// cast written with it needs only the type's name.
pub fn enum_column(name: &TypeName) -> ColumnType {
    ColumnType::Enum {
        name: Name::runtime(&name.name),
        schema: name.schema.as_ref().map(Name::runtime),
        variants: Vec::new(),
    }
}

/// The range or multirange type `kind` names, over the column type its
/// subtype's codec reads; refused for a subtype no codec reads, which no kind
/// a binding makes has.
pub fn created_column(kind: &CreatedKind) -> Result<ColumnType, Refusal> {
    let subtype = Arc::new(
        kind.codec()
            .ok_or_else(|| Refusal("a created range's subtype has no codec".to_owned()))?
            .subtype_column_type(),
    );
    let name = Name::runtime(&kind.name.name);
    let schema = kind.name.schema.as_ref().map(Name::runtime);
    Ok(if kind.multirange {
        ColumnType::CreatedMultirange {
            name,
            schema,
            subtype,
        }
    } else {
        ColumnType::CreatedRange {
            name,
            schema,
            subtype,
        }
    })
}

#[cfg(test)]
mod tests {
    use pgorm::pgorm_query::RangeType;

    use super::*;

    fn refused(built: Result<Option<ColumnType>, Refusal>) -> String {
        match built {
            Err(refusal) => refusal.into_message(),
            Ok(column) => panic!("{column:?} was not refused"),
        }
    }

    // [spec:pgorm:req:bindings.schema/test]    a built-in type takes the sizes SQL gives it and
    // no other, an unknown name is no built-in type, and a range kind's name is its range type
    #[test]
    fn built_in_types_take_their_own_sizes() {
        assert_eq!(
            builtin_column("char", None, None, None),
            Ok(Some(ColumnType::Char(Some(1))))
        );
        assert_eq!(
            builtin_column("numeric", None, Some(10), Some(2)),
            Ok(Some(ColumnType::Decimal(Some((10, 2)))))
        );
        assert_eq!(
            builtin_column("int4range", None, None, None),
            Ok(Some(ColumnType::Range(RangeType::Int4)))
        );
        assert_eq!(builtin_column("mood", None, None, None), Ok(None));
        assert_eq!(
            refused(builtin_column("integer", Some(4), None, None)),
            "integer takes no length"
        );
        assert_eq!(
            refused(builtin_column("text", None, Some(3), None)),
            "precision and scale belong to numeric"
        );
        assert_eq!(
            refused(builtin_column("varbit", None, None, None)),
            "varbit takes a length"
        );
    }
}
