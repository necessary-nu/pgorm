//! The SQL types SQL/JSON's RETURNING and JSON_TABLE's columns name, as
//! pgorm-query's `ColumnType`: a built-in type by name, with the size it
//! takes, an enum by `TypeName`, or a range type a schema created.

use std::sync::Arc;

use neon::prelude::*;
use pgorm::pgorm_query::{
    ArrayType, ColumnType, IntervalPrecision, IntervalSpec, Name, PgInterval, StringLen,
};

use super::{
    Node,
    args::{absent, arg, node, refuse, this},
};
use crate::{
    codec::Codec,
    values::{Tag, read},
};

pub(super) const EXPORTS: &[(&str, super::Build)] = &[
    ("dataTypeNew", data_type_new),
    ("dataTypeArray", data_type_array),
    ("dataTypeDomain", data_type_domain),
];

/// A size modifier: a whole number from 1 (or from 0 for a scale).
fn size(cx: &mut FunctionContext, index: usize, what: &str, floor: f64) -> NeonResult<Option<u32>> {
    let value = arg(cx, index);
    if absent(cx, value) {
        return Ok(None);
    }
    let Ok(number) = value.downcast::<JsNumber, _>(cx) else {
        return cx.throw_type_error(format!("a type's {what} is a number"));
    };
    let number = number.value(cx);
    if number.fract() == 0.0 && (floor..=10_000_000.0).contains(&number) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(Some(number as u32))
    } else {
        refuse(
            cx,
            format!("a type's {what} is a whole number of at least {floor}"),
        )
    }
}

/// The built-in type `name`, sized as `length`, `precision` and `scale` say,
/// each refused where the type takes none.
// [spec:pgorm:req:bindings.sql-json]
fn builtin(
    name: &str,
    length: Option<u32>,
    precision: Option<u32>,
    scale: Option<u32>,
) -> Result<ColumnType, String> {
    if length.is_some() && !matches!(name, "char" | "varchar" | "bit" | "varbit" | "vector") {
        return Err(format!("{name} takes no length"));
    }
    if (precision.is_some() || scale.is_some()) && name != "numeric" {
        return Err("precision and scale belong to numeric".to_owned());
    }
    Ok(match name {
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
            _ => return Err("numeric takes a precision of 1–1000 and a scale of 0–1000".to_owned()),
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
        "varbit" => ColumnType::VarBit(length.ok_or("varbit takes a length")?),
        "money" => ColumnType::Money,
        "json" => ColumnType::Json,
        "jsonb" => ColumnType::JsonBinary,
        "uuid" => ColumnType::Uuid,
        "vector" => ColumnType::Vector(length),
        "cidr" => ColumnType::Cidr,
        "inet" => ColumnType::Inet,
        "macaddr" => ColumnType::MacAddr,
        "ltree" => ColumnType::LTree,
        _ => {
            return match ArrayType::from_kind_name(name) {
                Some(ArrayType::Range(range)) => Ok(ColumnType::Range(range)),
                Some(ArrayType::Multirange(range)) => Ok(ColumnType::Multirange(range)),
                _ => Err(format!("{name:?} is no built-in type a DataType names")),
            };
        }
    })
}

/// A type named by `TypeName` (an enum) or a `CreatedRange` or
/// `CreatedMultirange`, which take no size.
fn named<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<ColumnType> {
    let codec = Codec::get(cx)?;
    Ok(match read::kind(cx, codec, value)? {
        Tag::Enum(name) => ColumnType::Enum {
            name: Name::runtime(name.name),
            schema: name.schema.map(Name::runtime),
            variants: Vec::new(),
        },
        Tag::Created(kind) => {
            let Some(codec) = kind.codec() else {
                return cx.throw_type_error("a created range's subtype has no codec");
            };
            let subtype = Arc::new(codec.subtype_column_type());
            let name = Name::runtime(kind.name.name);
            let schema = kind.name.schema.map(Name::runtime);
            if kind.multirange {
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
            }
        }
        _ => {
            return cx.throw_type_error(
                "a DataType names a built-in type, a TypeName or a CreatedRange",
            );
        }
    })
}

/// `interval`, `interval(p)` or `interval <fields>`: the fields spelled in
/// camelCase (`hourToMinute`), a precision of 0 to 6 only where the last field
/// is seconds.
// [spec:pgorm:req:bindings.model-casts]
fn interval(cx: &mut FunctionContext) -> NeonResult<Node> {
    let length = size(cx, 1, "length", 1.0)?;
    let scale = size(cx, 3, "scale", 0.0)?;
    if length.is_some() || scale.is_some() {
        return refuse(
            cx,
            "an interval takes fields and a precision, no length or scale",
        );
    }
    let precision = match size(cx, 2, "precision", 0.0)? {
        Some(digits) => match u8::try_from(digits).ok().and_then(IntervalPrecision::new) {
            Some(precision) => Some(precision),
            None => return refuse(cx, "an interval's precision is 0 to 6"),
        },
        None => None,
    };
    let fields = arg(cx, 4);
    if absent(cx, fields) {
        return Ok(Node::DataType(ColumnType::Interval(IntervalSpec::Any(
            precision,
        ))));
    }
    let fields = read::string(cx, fields)?;
    let words: String = fields
        .chars()
        .flat_map(|c| match c.is_ascii_uppercase() {
            true => vec![' ', c],
            false => vec![c],
        })
        .collect();
    let Ok(named) = PgInterval::try_from(words.as_str()) else {
        return cx.throw_type_error(
            "an interval's fields are year, month, day, hour, minute, second, yearToMonth, \
             dayToHour, dayToMinute, dayToSecond, hourToMinute, hourToSecond or minuteToSecond",
        );
    };
    match named.with_precision(precision) {
        Some(fields) => Ok(Node::DataType(ColumnType::Interval(IntervalSpec::Fields(
            fields,
        )))),
        None => refuse(cx, "only an interval ending in seconds takes a precision"),
    }
}

/// `dataTypeDomain(name, base)`: a domain over `base`, named by a string or
/// a `TypeName`.
// [spec:pgorm:req:bindings.model-casts]
fn data_type_domain(cx: &mut FunctionContext) -> NeonResult<Node> {
    let name = arg(cx, 0);
    let (name, schema) = match name.is_a::<JsString, _>(cx) {
        true => (read::string(cx, name)?, None),
        false => {
            let codec = Codec::get(cx)?;
            match read::kind(cx, codec, name)? {
                Tag::Enum(named) => (named.name, named.schema),
                _ => return cx.throw_type_error("a domain is named by a string or a TypeName"),
            }
        }
    };
    let base = arg(cx, 1);
    let base = column_type(cx, base)?;
    Ok(Node::DataType(ColumnType::Domain {
        name: Name::runtime(name),
        schema: schema.map(Name::runtime),
        base: Arc::new(base),
    }))
}

/// `dataTypeNew(kind, length, precision, scale, fields)`.
// [spec:pgorm:req:bindings.sql-json]
fn data_type_new(cx: &mut FunctionContext) -> NeonResult<Node> {
    let kind = arg(cx, 0);
    let fields = arg(cx, 4);
    let interval = !absent(cx, fields)
        || (kind.is_a::<JsString, _>(cx) && read::string(cx, kind)? == "interval");
    if interval {
        if !kind.is_a::<JsString, _>(cx) || read::string(cx, kind)? != "interval" {
            return refuse(cx, "fields are an interval's");
        }
        return self::interval(cx);
    }
    let length = size(cx, 1, "length", 1.0)?;
    let precision = size(cx, 2, "precision", 1.0)?;
    let scale = size(cx, 3, "scale", 0.0)?;
    if !kind.is_a::<JsString, _>(cx) {
        if length.is_some() || precision.is_some() || scale.is_some() {
            return refuse(cx, "a named type takes no size");
        }
        let named = named(cx, kind)?;
        return Ok(Node::DataType(named));
    }
    let name = read::string(cx, kind)?;
    match builtin(&name, length, precision, scale) {
        Ok(column) => Ok(Node::DataType(column)),
        Err(reason) => refuse(cx, reason),
    }
}

/// `dataTypeArray(type)`: the array of a type.
fn data_type_array(cx: &mut FunctionContext) -> NeonResult<Node> {
    match this(cx, 0)? {
        Node::DataType(column) => Ok(Node::DataType(ColumnType::Array(Arc::new(column)))),
        other => cx.throw_type_error(format!("expected a DataType, got {}", other.describe())),
    }
}

/// The type `value` names: a `DataType`, or a built-in type's name.
pub(crate) fn column_type<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<ColumnType> {
    if value.is_a::<JsString, _>(cx) {
        let name = read::string(cx, value)?;
        return match builtin(&name, None, None, None) {
            Ok(column) => Ok(column),
            Err(reason) => refuse(cx, reason),
        };
    }
    match node(cx, value) {
        Some(Node::DataType(column)) => Ok(column),
        _ => named(cx, value),
    }
}
