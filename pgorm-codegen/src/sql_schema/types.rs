use super::{Declared, DeclaredType, unresolved, unsupported};
use crate::Error;
use pg_query::NodeEnum;
use pg_query::protobuf::TypeName;
use pgorm_query::{
    ColumnType, IntervalPrecision, IntervalSpec, Name, PgInterval, RangeType, StringLen,
};
use std::sync::Arc;

/// A column's type together with the auto-increment fact the `serial` family
/// carries in its spelling rather than in a constraint.
pub(super) struct ColumnKind {
    pub(super) col_type: ColumnType,
    pub(super) auto_increment: bool,
}

/// Read a parsed `TypeName` as the `ColumnType` that renders it.
///
/// `context` names the column for the error message; `at` is the 1-based
/// statement number.
// [spec:pgorm:sem:codegen.ddl.types+10]
pub(super) fn column_kind(
    type_name: &TypeName,
    declared: &Declared,
    context: &str,
    at: usize,
) -> Result<ColumnKind, Error> {
    let names = idents(&type_name.names)
        .ok_or_else(|| unsupported(format!("a computed type name on {context}"), at))?;
    let modifiers = modifiers(type_name, context, at)?;
    let kind = named_type(&names, &modifiers, declared, context, at)?;
    match type_name.array_bounds.as_slice() {
        [] => Ok(kind),
        [bound] => {
            let unsized_bound =
                matches!(bound.node, Some(NodeEnum::Integer(ref i)) if i.ival == -1);
            if !unsized_bound {
                return Err(unsupported(format!("a sized array on {context}"), at));
            }
            if kind.auto_increment {
                return Err(unsupported(format!("an array of serial on {context}"), at));
            }
            Ok(ColumnKind {
                col_type: ColumnType::Array(Arc::new(kind.col_type)),
                auto_increment: false,
            })
        }
        _ => Err(unsupported(
            format!("a multi-dimensional array on {context}"),
            at,
        )),
    }
}

/// The type name's own arguments — `varchar(255)`'s 255, `numeric(10, 2)`'s
/// 10 and 2. Anything that is not a plain non-negative integer is out of the
/// bridge's reach.
fn modifiers(type_name: &TypeName, context: &str, at: usize) -> Result<Vec<u32>, Error> {
    let mut modifiers = Vec::with_capacity(type_name.typmods.len());
    for node in &type_name.typmods {
        let value = match &node.node {
            Some(NodeEnum::AConst(constant)) => match &constant.val {
                Some(pg_query::protobuf::a_const::Val::Ival(int)) => u32::try_from(int.ival).ok(),
                _ => None,
            },
            _ => None,
        };
        match value {
            Some(value) => modifiers.push(value),
            None => {
                return Err(unsupported(
                    format!("a non-integer type modifier on {context}"),
                    at,
                ));
            }
        }
    }
    Ok(modifiers)
}

/// The reverse of the `ColumnType` → Postgres spelling contract, read over the
/// names the grammar produces: keyword spellings arrive qualified as
/// `pg_catalog.<name>`, everything else bare.
// [spec:pgorm:sem:codegen.ddl.types+10]
fn named_type(
    names: &[String],
    modifiers: &[u32],
    declared: &Declared,
    context: &str,
    at: usize,
) -> Result<ColumnKind, Error> {
    let (catalog, schema, name) = match names {
        [name] => (false, None, name),
        [schema, name] if schema == "pg_catalog" => (true, None, name),
        [schema, name] => (false, Some(schema), name),
        _ => return Err(unsupported(format!("a type name on {context}"), at)),
    };
    if !catalog
        && let Some(kind) = declared_type(schema.map(String::as_str), name, declared, context, at)?
    {
        return Ok(kind);
    }
    if let (Some(col_type), []) = (builtin_range(name), modifiers) {
        return Ok(plain(col_type));
    }
    let col_type = match (name.as_str(), modifiers) {
        ("serial" | "serial4", []) => return Ok(serial(ColumnType::Integer)),
        ("bigserial" | "serial8", []) => return Ok(serial(ColumnType::BigInteger)),
        ("smallserial" | "serial2", []) => return Ok(serial(ColumnType::SmallInteger)),
        ("bpchar", []) => ColumnType::Char(None),
        ("bpchar", [length]) => ColumnType::Char(Some(*length)),
        ("varchar", []) => ColumnType::String(StringLen::None),
        ("varchar", [length]) => ColumnType::String(StringLen::N(*length)),
        ("text", []) => ColumnType::Text,
        ("int2" | "smallint", []) => ColumnType::SmallInteger,
        ("int4" | "int" | "integer", []) => ColumnType::Integer,
        ("int8" | "bigint", []) => ColumnType::BigInteger,
        ("float4" | "real", []) => ColumnType::Float,
        ("float8", []) => ColumnType::Double,
        ("numeric" | "decimal", []) => ColumnType::Decimal(None),
        ("numeric" | "decimal", [precision]) => ColumnType::Decimal(Some((*precision, 0))),
        ("numeric" | "decimal", [precision, scale]) => {
            ColumnType::Decimal(Some((*precision, *scale)))
        }
        ("timestamp", []) => ColumnType::Timestamp,
        ("timestamptz", []) => ColumnType::TimestampWithTimeZone,
        ("time", []) => ColumnType::Time,
        ("date", []) => ColumnType::Date,
        ("interval", modifiers) => ColumnType::Interval(interval_spec(modifiers, context, at)?),
        ("bool" | "boolean", []) => ColumnType::Boolean,
        ("money", []) => ColumnType::Money,
        ("bytea", []) => ColumnType::Bytea,
        ("bit", []) => ColumnType::Bit(None),
        ("bit", [length]) => ColumnType::Bit(Some(*length)),
        ("varbit", [length]) => ColumnType::VarBit(*length),
        ("json", []) => ColumnType::Json,
        ("jsonb", []) => ColumnType::JsonBinary,
        ("uuid", []) => ColumnType::Uuid,
        ("inet", []) => ColumnType::Inet,
        ("cidr", []) => ColumnType::Cidr,
        ("macaddr", []) => ColumnType::MacAddr,
        ("ltree", []) => ColumnType::LTree,
        ("vector", []) => ColumnType::Vector(None),
        ("vector", [size]) => ColumnType::Vector(Some(*size)),
        ("varbit", []) => {
            return Err(unsupported(
                format!("`varbit` without a length on {context}"),
                at,
            ));
        }
        _ if !modifiers.is_empty() => {
            return Err(unsupported(
                format!("`{name}` with a type modifier on {context}"),
                at,
            ));
        }
        _ => return Err(unsupported(format!("type `{name}` on {context}"), at)),
    };
    Ok(plain(col_type))
}

/// An interval's qualifier, from the modifiers the grammar writes for it: a
/// field mask — `INTERVAL_MASK` bits over the fields the qualifier spans, or
/// every bit (`0x7FFF`) for an unqualified `interval(p)` — then the
/// fractional-seconds precision when one was given.
///
/// The mask is read as `intervaltypmodin` reads it, so a generic spelling
/// such as `pg_catalog.interval(6)` means what the server takes it to mean,
/// `YEAR TO MONTH`. A precision on a field other than `SECOND` reaches here
/// only through that generic spelling and has no `IntervalSpec`; a precision
/// past 6 the server would quietly reduce to 6, which is the file saying
/// something other than what the column would be.
// [spec:pgorm:sem:codegen.ddl.types+10]
fn interval_spec(modifiers: &[u32], context: &str, at: usize) -> Result<IntervalSpec, Error> {
    const MONTH: u32 = 1 << 1;
    const YEAR: u32 = 1 << 2;
    const DAY: u32 = 1 << 3;
    const HOUR: u32 = 1 << 10;
    const MINUTE: u32 = 1 << 11;
    const SECOND: u32 = 1 << 12;
    const FULL_RANGE: u32 = 0x7FFF;

    let (mask, precision) = match modifiers {
        [] => return Ok(IntervalSpec::Any(None)),
        [mask] => (*mask, None),
        [mask, precision] => {
            let digits = u8::try_from(*precision)
                .ok()
                .and_then(IntervalPrecision::new)
                .ok_or_else(|| {
                    unsupported(
                        format!(
                            "an `interval` precision of {precision}, above the 6 PostgreSQL keeps, on {context}"
                        ),
                        at,
                    )
                })?;
            (*mask, Some(digits))
        }
        _ => {
            return Err(unsupported(
                format!("`interval` with a type modifier on {context}"),
                at,
            ));
        }
    };
    let spec = match (mask, precision) {
        (FULL_RANGE, precision) => IntervalSpec::Any(precision),
        (YEAR, None) => IntervalSpec::Fields(PgInterval::Year),
        (MONTH, None) => IntervalSpec::Fields(PgInterval::Month),
        (DAY, None) => IntervalSpec::Fields(PgInterval::Day),
        (HOUR, None) => IntervalSpec::Fields(PgInterval::Hour),
        (MINUTE, None) => IntervalSpec::Fields(PgInterval::Minute),
        (SECOND, precision) => IntervalSpec::Fields(PgInterval::Second(precision)),
        (m, None) if m == YEAR | MONTH => IntervalSpec::Fields(PgInterval::YearToMonth),
        (m, None) if m == DAY | HOUR => IntervalSpec::Fields(PgInterval::DayToHour),
        (m, None) if m == DAY | HOUR | MINUTE => IntervalSpec::Fields(PgInterval::DayToMinute),
        (m, precision) if m == DAY | HOUR | MINUTE | SECOND => {
            IntervalSpec::Fields(PgInterval::DayToSecond(precision))
        }
        (m, None) if m == HOUR | MINUTE => IntervalSpec::Fields(PgInterval::HourToMinute),
        (m, precision) if m == HOUR | MINUTE | SECOND => {
            IntervalSpec::Fields(PgInterval::HourToSecond(precision))
        }
        (m, precision) if m == MINUTE | SECOND => {
            IntervalSpec::Fields(PgInterval::MinuteToSecond(precision))
        }
        _ => {
            return Err(unsupported(
                format!("`interval` with a type modifier on {context}"),
                at,
            ));
        }
    };
    Ok(spec)
}

/// A name the file declared as a type — an enum, a range type or a domain — resolved by
/// its exact identity, `None` when it names none and is to be read as a
/// built-in. A reference to a declared type's bare name under another
/// qualification MUST NOT resolve to that schema's type, and a schema-qualified
/// name the file did not declare is no built-in either.
// [spec:pgorm:sem:codegen.ddl.types+10]
fn declared_type(
    schema: Option<&str>,
    name: &str,
    declared: &Declared,
    context: &str,
    at: usize,
) -> Result<Option<ColumnKind>, Error> {
    let identity = (schema.map(str::to_owned), name.to_owned());
    match declared.get(&identity) {
        Some(DeclaredType::Enum(variants)) => {
            return Ok(Some(plain(ColumnType::Enum {
                schema: identity.0.map(|schema| Name::runtime(schema) as _),
                name: Name::runtime(name),
                variants: variants
                    .iter()
                    .map(|variant| Name::runtime(variant.as_str()))
                    .collect(),
            })));
        }
        Some(DeclaredType::Range(subtype)) => {
            return Ok(Some(plain(ColumnType::CreatedRange {
                schema: identity.0.map(|schema| Name::runtime(schema) as _),
                name: Name::runtime(name),
                subtype: Arc::new(subtype.clone()),
            })));
        }
        Some(DeclaredType::Multirange(subtype)) => {
            return Ok(Some(plain(ColumnType::CreatedMultirange {
                schema: identity.0.map(|schema| Name::runtime(schema) as _),
                name: Name::runtime(name),
                subtype: Arc::new(subtype.clone()),
            })));
        }
        Some(DeclaredType::Domain(base)) => {
            return Ok(Some(plain(ColumnType::Domain {
                schema: identity.0.map(|schema| Name::runtime(schema) as _),
                name: Name::runtime(name),
                base: Arc::new(base.clone()),
            })));
        }
        None => {}
    }
    let same_named = declared
        .iter()
        .find(|((_, declared), _)| declared.as_str() == name)
        .map(|(_, kind)| kind);
    if let Some(kind) = same_named {
        let spelled = spell_identity(&identity);
        let kind = match kind {
            DeclaredType::Enum(_) => "enum",
            DeclaredType::Range(_) => "range type",
            DeclaredType::Multirange(_) => "multirange type",
            DeclaredType::Domain(_) => "domain",
        };
        return Err(unresolved(
            format!(
                "type `{spelled}` on {context} is not declared; a same-named {kind} under a different qualification does not resolve it"
            ),
            at,
        ));
    }
    match schema {
        Some(schema) => Err(unsupported(
            format!("type `{schema}.{name}` on {context}"),
            at,
        )),
        None => Ok(None),
    }
}

/// A built-in range or multirange type by its catalogue name. None takes a
/// type modifier, so a modified one falls through to the refusal below.
// [spec:pgorm:sem:codegen.ddl.types+10]
fn builtin_range(name: &str) -> Option<ColumnType> {
    [
        RangeType::Int4,
        RangeType::Int8,
        RangeType::Numeric,
        RangeType::Date,
        RangeType::Timestamp,
        RangeType::TimestampTz,
    ]
    .into_iter()
    .find_map(|range| {
        if name == range.range_type_name() {
            Some(ColumnType::Range(range))
        } else if name == range.multirange_type_name() {
            Some(ColumnType::Multirange(range))
        } else {
            None
        }
    })
}

fn plain(col_type: ColumnType) -> ColumnKind {
    ColumnKind {
        col_type,
        auto_increment: false,
    }
}

fn serial(col_type: ColumnType) -> ColumnKind {
    ColumnKind {
        col_type,
        auto_increment: true,
    }
}

/// The `String` nodes of a name list, or `None` when the list holds anything
/// else.
pub(super) fn idents(nodes: &[pg_query::protobuf::Node]) -> Option<Vec<String>> {
    nodes
        .iter()
        .map(|node| match &node.node {
            Some(NodeEnum::String(text)) if !text.sval.is_empty() => Some(text.sval.clone()),
            _ => None,
        })
        .collect()
}

/// The human spelling of a type's identity: `schema.name` or the bare name.
pub(super) fn spell_identity(identity: &super::TypeIdentity) -> String {
    match identity {
        (Some(schema), name) => format!("{schema}.{name}"),
        (None, name) => name.clone(),
    }
}
