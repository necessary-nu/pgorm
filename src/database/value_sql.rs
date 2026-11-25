use super::connection::{SqlText, private};
use crate::ValueHolder;
use pgorm_driver::{Error, types::Type};
use pgorm_query::Value;

/// Borrow the values before ToSql erases their variants. PostgreSQL never
/// resolves these hints, retaining its context inference and cache keys.
// [spec:pgorm:req:turso.parameters]
#[derive(Debug)]
pub struct ValueSql<'a, P = ValueHolder> {
    sql: &'a str,
    values: &'a [P],
    value: for<'v> fn(&'v P) -> Option<&'v Value>,
}

impl<'a> ValueSql<'a> {
    /// Retain SQL and borrowed values until the connection selects its backend.
    pub fn new(sql: &'a str, values: &'a [ValueHolder]) -> Self {
        Self {
            sql,
            values,
            value: |value| Some(&value.0),
        }
    }
}

impl<'a, P> ValueSql<'a, P> {
    /// Borrow parameters without replacing their encoders. An absent value
    /// has no native type; PostgreSQL never calls the mapper.
    pub fn mapped(
        sql: &'a str,
        values: &'a [P],
        value: for<'v> fn(&'v P) -> Option<&'v Value>,
    ) -> Self {
        Self { sql, values, value }
    }
}

// [spec:pgorm:def:conn.sql-text+6]
impl<P> SqlText for ValueSql<'_, P> {
    fn sql_text(&self) -> &str {
        self.sql
    }
}

// [spec:pgorm:req:turso.parameters] typed nulls retain the variant's scalar type
impl<P> private::Sealed for ValueSql<'_, P> {
    fn native_parameter_types(&self) -> Result<Vec<Type>, Error> {
        self.values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                (self.value)(value)
                    .ok_or("untyped NULL")
                    .and_then(native_type)
                    .map_err(|kind| {
                        Error::to_sql(
                            format!("native Turso does not support {kind} parameters").into(),
                            index,
                        )
                    })
            })
            .collect()
    }
}

fn native_type(value: &Value) -> Result<Type, &'static str> {
    Ok(match value {
        Value::Bool(_) => Type::BOOL,
        Value::TinyInt(_) | Value::SmallInt(_) => Type::INT2,
        Value::Int(_) => Type::INT4,
        Value::BigInt(_) | Value::Unsigned(_) | Value::BigUnsigned(_) => Type::INT8,
        Value::Float(_) => Type::FLOAT4,
        Value::Double(_) => Type::FLOAT8,
        Value::String(_) | Value::Char(_) => Type::TEXT,
        Value::Bytes(_) => Type::BYTEA,
        Value::Json(_) => return Err("JSON"),
        Value::Date(_) => return Err("DATE"),
        Value::Time(_) => return Err("TIME"),
        Value::DateTime(_) => return Err("TIMESTAMP"),
        Value::DateTimeWithTimeZone(_) => return Err("TIMESTAMPTZ"),
        Value::Uuid(_) => return Err("UUID"),
        Value::Decimal(_) => return Err("NUMERIC"),
        Value::Array(..) => return Err("array"),
        Value::Vector(_) => return Err("vector"),
        Value::IpNetwork(_) => return Err("INET"),
        Value::MacAddress(_) => return Err("MACADDR"),
        Value::Interval(_) => return Err("INTERVAL"),
        Value::Range(..) => return Err("range"),
        Value::Multirange(..) => return Err("multirange"),
        _ => return Err("unknown Value"),
    })
}
