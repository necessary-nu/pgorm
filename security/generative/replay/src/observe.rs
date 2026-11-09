//! Lossless native observations, in the shapes `pgorm_campaign.observations`
//! and `pgorm_campaign.effects` produce.
//!
//! Nothing here decides whether a result is *correct*. An observation records
//! what the subject saw, in a form the independent Python oracle can compare
//! against what it saw; a comparison performed on this side would be a second
//! oracle agreeing with the first.

use std::collections::HashSet;

use pgorm::pgorm_query::{ArrayType, ColumnType, Name, SqlName, Value};
use pgorm::{ColumnTrait, EntityTrait, Iterable, ModelTrait, StaticName};
use serde_json::{Value as Json, json};
use tokio_postgres::Row;

use crate::{
    FormatError, ObservedError, decode,
    wire::{Tagged, TypeName},
};

/// A set of rows.
///
/// # Errors
///
/// Returns [`FormatError`] when any column cannot be decoded losslessly.
pub fn rows(rows: &[Row]) -> Result<Json, FormatError> {
    Ok(json!({"kind": "rows", "rows": records(rows)?}))
}

/// A set of rows already observed one at a time, as decoded models are.
pub fn collected(rows: Vec<Json>) -> Json {
    json!({"kind": "rows", "rows": rows})
}

/// A set of rows drained from a stream, with the stream's terminal state.
///
/// # Errors
///
/// Returns [`FormatError`] when any column cannot be decoded losslessly.
pub fn stream(
    rows: &[Row],
    complete: bool,
    cancelled: bool,
    closed: bool,
) -> Result<Json, FormatError> {
    Ok(json!({
        "kind": "rows",
        "rows": records(rows)?,
        "stream": {"complete": complete, "cancelled": cancelled, "closed": closed},
    }))
}

/// One row, with no owning entity.
///
/// # Errors
///
/// Returns [`FormatError`] when any column cannot be decoded losslessly, or
/// when two output columns share a name — an ambiguity the campaign resolves by
/// requiring every projection to carry a unique alias.
pub fn row(row: &Row) -> Result<Json, FormatError> {
    record(row, None)
}

/// One row, optionally naming the registered entity it was decoded into.
///
/// # Errors
///
/// Returns [`FormatError`] when any column cannot be decoded losslessly, or
/// when two output columns share a name.
pub fn record(row: &Row, entity: Option<&str>) -> Result<Json, FormatError> {
    let mut seen = HashSet::new();
    let mut fields = Vec::with_capacity(row.len());
    let mut postgres = Vec::with_capacity(row.len());
    for (index, column) in row.columns().iter().enumerate() {
        if !seen.insert(column.name()) {
            return Err(FormatError::new(
                "duplicate result field name; give each projection a unique alias",
            ));
        }
        let value = decode::value(row, index)?.encode_checked()?;
        fields.push(json!({"name": column.name(), "value": value}));
        postgres.push(json!({
            "name": column.name(),
            "type": column.type_().name(),
            "schema": column.type_().schema(),
        }));
    }
    let mut observation = json!({"kind": "record", "fields": fields, "postgres": postgres});
    if let (Some(entity), Some(object)) = (entity, observation.as_object_mut()) {
        object.insert("entity".to_owned(), json!(entity));
    }
    Ok(observation)
}

/// One decoded model of a compiled entity, naming the registration it belongs to.
///
/// Deliberately *unlike* [`record`]: a model is not a PostgreSQL result row and
/// carries no output identity, so this observation has no `postgres` key. The
/// binding's `EntityModel` exposes neither `fields` nor `native`, so
/// `observations.row` omits that key there too, and a subject that invented one
/// would fail parity against an oracle that cannot produce it.
///
/// The field names and their order are `Column::iter()`'s — the same iteration
/// the binding's `keys()` answers from — and each value is tagged from the
/// column's *declared* type, so an enum label keeps its qualified identity
/// rather than arriving as indistinguishable text.
///
/// # Errors
///
/// Returns [`FormatError`] when a column's declared type and its decoded Rust
/// value disagree, or when a payload is not a valid portable value.
pub fn model<E>(value: &E::Model, entity: &str) -> Result<Json, FormatError>
where
    E: EntityTrait,
{
    let mut fields = Vec::new();
    for column in E::Column::iter() {
        let definition = ColumnTrait::def(&column);
        let tagged = declared(definition.get_column_type(), ModelTrait::get(value, column))?;
        fields.push(json!({
            "name": StaticName::as_str(&column),
            "value": tagged.encode_checked()?,
        }));
    }
    if fields.is_empty() {
        return Err(FormatError::new(
            "a compiled entity projects no columns; its model cannot be observed",
        ));
    }
    Ok(json!({"kind": "record", "entity": entity, "fields": fields}))
}

/// An optional slot: the model when a join matched it, [`absent`] when not.
///
/// # Errors
///
/// Returns [`FormatError`] for the same reasons [`model`] does.
pub fn maybe<E>(value: Option<&E::Model>, entity: &str) -> Result<Json, FormatError>
where
    E: EntityTrait,
{
    match value {
        Some(value) => model::<E>(value, entity),
        None => Ok(absent()),
    }
}

/// Tag a model's value by what its column was *declared* as.
///
/// Only an enum needs this: `Value::String` is the Rust representation of both
/// a text column and an enum label, and the campaign's hostile enum names make
/// the distinction load-bearing. Everything else is tagged by its own variant,
/// exactly as pgorm-bindings' `ColumnInfo::tag_of` decides.
fn declared(kind: &ColumnType, value: Value) -> Result<Tagged, FormatError> {
    match kind {
        ColumnType::Enum { name, schema, .. } => enumerated(name, schema.as_ref(), value, false),
        ColumnType::Array(member) => match member.as_ref() {
            ColumnType::Enum { name, schema, .. } => enumerated(name, schema.as_ref(), value, true),
            _ => Ok(Tagged::from_value(value)),
        },
        _ => Ok(Tagged::from_value(value)),
    }
}

fn enumerated(
    name: &Name,
    schema: Option<&Name>,
    value: Value,
    array: bool,
) -> Result<Tagged, FormatError> {
    let compatible = if array {
        matches!(value, Value::Array(ArrayType::String, _))
    } else {
        matches!(value, Value::String(_))
    };
    if !compatible {
        return Err(FormatError::new(
            "compiled enum column returned an incompatible Rust value",
        ));
    }
    let identity = TypeName::new(SqlName::to_string(&**name));
    let identity = match schema {
        Some(schema) => identity.in_schema(SqlName::to_string(&**schema)),
        None => identity,
    };
    Ok(Tagged::from_enum(value, identity, array))
}

/// A terminal that yielded no row at all, as an optional graph slot does.
pub fn absent() -> Json {
    json!({"kind": "absent"})
}

/// A tuple of already-observed values, as a multi-source graph row is.
pub fn tuple(items: Vec<Json>) -> Json {
    json!({"kind": "tuple", "items": items})
}

/// An affected-row count.
///
/// The payload key is `value`, not `count`: `effects.py` spells it that way and
/// the Python report is what this one is compared against field by field.
pub fn count(count: u64) -> Json {
    json!({"kind": "count", "value": count})
}

/// An effect that produced no value, as a commit or rollback does.
pub fn unit() -> Json {
    json!({"kind": "unit"})
}

/// A transaction opened into a named scope.
pub fn transaction(scope: &str) -> Json {
    json!({"kind": "transaction", "scope": scope})
}

/// A built statement and the parameters bound to it, without executing either.
///
/// # Errors
///
/// Returns [`FormatError`] when a bound parameter is not a valid portable value.
pub fn compiled(sql: &str, parameters: &[Tagged]) -> Result<Json, FormatError> {
    let parameters = parameters
        .iter()
        .map(Tagged::encode_checked)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"kind": "compiled", "sql": sql, "parameters": parameters}))
}

/// A failure, classified the way the Python binding classifies the same cause.
pub fn error(error: &ObservedError) -> Json {
    json!({
        "kind": "error",
        "class": error.class(),
        "cause": error.cause(),
        "sqlstate": error.sqlstate(),
    })
}

fn records(rows: &[Row]) -> Result<Vec<Json>, FormatError> {
    rows.iter().map(row).collect()
}
