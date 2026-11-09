//! Where a bound value carries its own type outside a cast: the operand of an
//! operator, or the argument of a function, whose overload PostgreSQL chooses
//! by that operand's type rather than taking it from a column beside it.
//!
//! A placeholder elsewhere is left untyped, the server reading its type from
//! the column it is compared with or assigned to, which is the reading the
//! entity layer's casts are written for (`typed-binds`).

use super::*;

/// The side of a binary operator an operand stands on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Left,
    Right,
}

impl QueryBuilder {
    /// Write `operand`, a bound value as `$n::type`, its own type annotated,
    /// a 64-bit integer that fits 32 bits as the `int4` its SQL literal is.
    // [spec:pgorm:sem:sql.render.placeholder-typing+2]
    pub(super) fn prepare_typed_operand(&self, operand: &SimpleExpr, sql: &mut dyn SqlWriter) {
        match operand {
            SimpleExpr::Value(value) if fits_int4(value) => {
                sql.push_param_typed(value.clone(), "int4");
            }
            SimpleExpr::Value(value) => sql.push_param_source_typed(value.clone()),
            other => self.prepare_simple_expr(other, sql),
        }
    }
}

/// Whether `op` chooses its overload by the type of the bound value on
/// `side`. The arithmetic operators, `||` and the JSON field operators are
/// overloaded across operand types a column beside them does not settle: a
/// date plus an integer is a date, a timestamp minus an interval a
/// timestamp, `->` an integer an array element where text is a key. A
/// container range or multirange of `@>` and `<@` may stand beside an
/// element, which types it as no range.
// [spec:pgorm:sem:sql.render.placeholder-typing+2]
pub(super) fn operator_reads_type(op: &BinOper, side: Side, operand: &SimpleExpr) -> bool {
    let SimpleExpr::Value(value) = operand else {
        return false;
    };
    match op {
        BinOper::Add
        | BinOper::Sub
        | BinOper::Mul
        | BinOper::Div
        | BinOper::Mod
        | BinOper::Concatenate
        | BinOper::GetJsonField
        | BinOper::CastJsonField => names_its_own_type(value),
        BinOper::Contains => side == Side::Left && is_range(value),
        BinOper::Contained => side == Side::Right && is_range(value),
        _ => false,
    }
}

/// Whether a function reads the type of its argument at `position` to choose
/// its overload: the aggregates and `abs`, defined for every numeric type,
/// `round`'s operand, `lower` and `upper`, of text or of a range,
/// `percentile_cont` and `percentile_disc`, of one fraction or an array of
/// them, and the polymorphic `generate_series` and `unnest`. Every other
/// argument has the type the function's signature gives it.
// [spec:pgorm:sem:sql.render.placeholder-typing+2]
pub(super) fn function_reads_type(func: &Function, position: usize) -> bool {
    match func {
        Function::Max
        | Function::Min
        | Function::Sum
        | Function::Avg
        | Function::Abs
        | Function::BitAnd
        | Function::BitOr
        | Function::PercentileCont
        | Function::PercentileDisc
        | Function::Lower
        | Function::Upper
        | Function::GenerateSeries
        | Function::Unnest => true,
        Function::Round => position == 0,
        _ => false,
    }
}

/// A scalar whose type is its own beside any column: not text, which
/// PostgreSQL types from the operand beside it as it types a string literal,
/// nor JSON, which names no one type, nor a float, which beside a `numeric`
/// is the `numeric`'s, nor an array or a range, which beside a column of one
/// takes that column's type, PostgreSQL converting between none of them.
fn names_its_own_type(value: &Value) -> bool {
    matches!(
        value,
        Value::Bool(_)
            | Value::TinyInt(_)
            | Value::SmallInt(_)
            | Value::Int(_)
            | Value::BigInt(_)
            | Value::Unsigned(_)
            | Value::BigUnsigned(_)
            | Value::Bytes(_)
            | Value::Date(_)
            | Value::Time(_)
            | Value::DateTime(_)
            | Value::DateTimeWithTimeZone(_)
            | Value::Uuid(_)
            | Value::Decimal(_)
            | Value::IpNetwork(_)
            | Value::MacAddress(_)
            | Value::Interval(_)
    )
}

/// A 64-bit integer whose value fits 32 bits, which PostgreSQL would read
/// as an `integer` were it written as a literal. The bindings hold every
/// integer in 64 bits, so annotating its own `int8` would widen an
/// `integer` column's arithmetic to `bigint`, and a recursive term's with
/// it, which its anchor's `integer` then refuses (`42804`).
fn fits_int4(value: &Value) -> bool {
    match value {
        Value::BigInt(Some(v)) => i32::try_from(*v).is_ok(),
        Value::Unsigned(Some(v)) => i32::try_from(*v).is_ok(),
        Value::BigUnsigned(Some(v)) => i32::try_from(*v).is_ok(),
        _ => false,
    }
}

fn is_range(value: &Value) -> bool {
    matches!(value, Value::Range(..) | Value::Multirange(..))
}
