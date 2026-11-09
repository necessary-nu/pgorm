//! What the schema builders read from their arguments: the relation or type a
//! statement names, one name or a non-empty list of them, the closed choices
//! an option carries, a collation, a sequence's options and the text a label
//! or comment is.
//!
//! An argument of the wrong Python type is a `TypeError`, a string outside
//! a closed choice among them, since Python's typing spells the choice as a
//! `Literal` of its strings; a value of the right type that cannot become what
//! pgorm writes — an empty list, an aliased `Table`, a label PostgreSQL cannot
//! store, an integer outside `bigint` — is a `ConstructionError`.

use crate::{
    errors::ConstructionError, identifiers::Identifier, statements::PyTable, values::PyTypeName,
};
use pgorm::pgorm_query::{
    Collation, Deferrability, DropBehavior, Enforcement, ForeignKeyAction, IntoCollation, Name,
    SequenceOption, SequenceOptions, TableName, extension::TypeRef,
};
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyBool, PyDict, PyInt, PyString},
};

/// One identifier: a `str`, anything else the wrong type.
pub(super) fn name(value: &Bound<'_, PyAny>) -> PyResult<Name> {
    if !value.is_instance_of::<PyString>() {
        return Err(PyTypeError::new_err("a name is a str"));
    }
    Ok(Identifier::new(value)?.name())
}

/// The items of a list, refusing a value that is no sequence or is text.
fn items<'py>(value: &Bound<'py, PyAny>, what: &str) -> PyResult<Vec<Bound<'py, PyAny>>> {
    value
        .extract::<Vec<Bound<'py, PyAny>>>()
        .map_err(|_| PyTypeError::new_err(format!("{what} is a list")))
}

/// The identifiers of a list, which may be empty.
pub(super) fn names(value: &Bound<'_, PyAny>, what: &str) -> PyResult<Vec<Name>> {
    items(value, what)?.iter().map(name).collect()
}

/// One item, where `single` says the value is one, or a non-empty list of
/// them; the first is apart, as pgorm-query's keys, drops and lists take it.
fn one_or_more<T>(
    value: &Bound<'_, PyAny>,
    what: &str,
    single: fn(&Bound<'_, PyAny>) -> bool,
    read: fn(&Bound<'_, PyAny>) -> PyResult<T>,
) -> PyResult<(T, Vec<T>)> {
    if single(value) {
        return Ok((read(value)?, Vec::new()));
    }
    let mut all = items(value, what)?
        .iter()
        .map(read)
        .collect::<PyResult<Vec<_>>>()?
        .into_iter();
    let Some(first) = all.next() else {
        return Err(ConstructionError::new_err(format!(
            "{what} names at least one"
        )));
    };
    Ok((first, all.collect()))
}

/// A key's or a list's columns: one name, or a non-empty list of them.
// [spec:pgorm:req:bindings.schema-tables]
pub(super) fn columns(value: &Bound<'_, PyAny>, what: &str) -> PyResult<(Name, Vec<Name>)> {
    one_or_more(
        value,
        what,
        |value: &Bound<'_, PyAny>| value.is_instance_of::<PyString>(),
        name,
    )
}

/// The relation a statement names: a name, or a `Table` with no alias, its
/// schema kept. A sequence and a view are relations too.
// [spec:pgorm:req:bindings.schema]
pub(crate) fn relation(value: &Bound<'_, PyAny>) -> PyResult<TableName> {
    if let Ok(table) = value.extract::<PyRef<'_, PyTable>>() {
        if table.inner.alias.is_some() {
            return Err(ConstructionError::new_err(
                "a DDL statement names a Table without an alias: an alias belongs to a query",
            ));
        }
        return Ok(table.inner.name.clone());
    }
    if value.is_instance_of::<PyString>() {
        return Ok(TableName::Table(name(value)?));
    }
    Err(PyTypeError::new_err(
        "a table or sequence is named by a str, an Identifier or a Table",
    ))
}

fn is_relation(value: &Bound<'_, PyAny>) -> bool {
    value.is_instance_of::<PyString>() || value.is_instance_of::<PyTable>()
}

/// One relation or a non-empty list of them, for a drop.
pub(super) fn relations(value: &Bound<'_, PyAny>) -> PyResult<(TableName, Vec<TableName>)> {
    one_or_more(value, "a drop", is_relation, relation)
}

/// The type a statement names: a name, or a `TypeName`, its schema kept.
// [spec:pgorm:req:bindings.schema-types]
pub(super) fn type_ref(value: &Bound<'_, PyAny>) -> PyResult<TypeRef> {
    if let Ok(named) = value.extract::<PyRef<'_, PyTypeName>>() {
        let local = Name::runtime(&named.name);
        return Ok(match &named.schema {
            Some(schema) => TypeRef::SchemaType(Name::runtime(schema), local),
            None => TypeRef::Type(local),
        });
    }
    if value.is_instance_of::<PyString>() {
        return Ok(TypeRef::Type(name(value)?));
    }
    Err(PyTypeError::new_err(
        "a type is named by a str, an Identifier or a TypeName",
    ))
}

fn is_type(value: &Bound<'_, PyAny>) -> bool {
    value.is_instance_of::<PyString>() || value.is_instance_of::<PyTypeName>()
}

/// One type or a non-empty list of them, for a drop.
pub(super) fn type_refs(value: &Bound<'_, PyAny>) -> PyResult<(TypeRef, Vec<TypeRef>)> {
    one_or_more(value, "a drop", is_type, type_ref)
}

/// `value`, a string that is one of `choices`, given as `(spelling, meaning)`.
// [spec:pgorm:req:bindings.argument-errors+1]
pub(super) fn pick<T: Clone>(value: &str, what: &str, choices: &[(&str, T)]) -> PyResult<T> {
    pgorm_bindings::pick(Some(value), what, choices)
        .map_err(|refusal| PyTypeError::new_err(refusal.into_message()))
}

pub(super) fn choice<T: Clone>(
    value: Option<&str>,
    what: &str,
    choices: &[(&str, T)],
) -> PyResult<Option<T>> {
    value.map(|value| pick(value, what, choices)).transpose()
}

pub(super) const DEFERRABILITY: &[(&str, Deferrability)] = &[
    ("not_deferrable", Deferrability::NotDeferrable),
    (
        "deferrable_initially_immediate",
        Deferrability::DeferrableInitiallyImmediate,
    ),
    (
        "deferrable_initially_deferred",
        Deferrability::DeferrableInitiallyDeferred,
    ),
];

pub(super) const ENFORCEMENT: &[(&str, Enforcement)] = &[
    ("enforced", Enforcement::Enforced),
    ("not_enforced", Enforcement::NotEnforced),
];

pub(super) const ACTIONS: &[(&str, ForeignKeyAction)] = &[
    ("no_action", ForeignKeyAction::NoAction),
    ("restrict", ForeignKeyAction::Restrict),
    ("cascade", ForeignKeyAction::Cascade),
    ("set_null", ForeignKeyAction::SetNull),
    ("set_default", ForeignKeyAction::SetDefault),
];

/// What a drop does with what depends on what it drops, when it is said.
pub(super) fn behavior(value: Option<&str>) -> PyResult<Option<DropBehavior>> {
    choice(
        value,
        "a behavior",
        &[
            ("restrict", DropBehavior::Restrict),
            ("cascade", DropBehavior::Cascade),
        ],
    )
}

pub(super) fn collation(value: Option<&Bound<'_, PyAny>>) -> PyResult<Option<Collation>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_instance_of::<PyString>() {
        return Ok(Some(name(value)?.into_collation()));
    }
    let Ok(parts) = value.cast::<PyDict>() else {
        return Err(PyTypeError::new_err(
            "a collation is a name or {\"name\": .., \"schema\": ..}",
        ));
    };
    for key in parts.keys() {
        if !matches!(key.extract::<&str>(), Ok("name" | "schema")) {
            return Err(PyTypeError::new_err(format!(
                "{key} is no part of a collation; its parts are name, schema"
            )));
        }
    }
    let Some(named) = parts.get_item("name")? else {
        return Err(PyTypeError::new_err("a collation's dict names it by name"));
    };
    let named = name(&named)?;
    Ok(Some(match parts.get_item("schema")? {
        Some(schema) => (name(&schema)?, named).into_collation(),
        None => named.into_collation(),
    }))
}

/// A 64-bit integer: an `int`, never a `bool`, within PostgreSQL's `bigint`.
pub(super) fn integer(value: &Bound<'_, PyAny>, what: &str) -> PyResult<i64> {
    if !value.is_instance_of::<PyInt>() || value.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!("{what} is an int")));
    }
    value
        .extract::<i64>()
        .map_err(|_| ConstructionError::new_err(format!("{what} is outside PostgreSQL's bigint")))
}

const SEQUENCE_OPTIONS: &[&str] = &[
    "increment_by",
    "min_value",
    "max_value",
    "start_with",
    "cache",
    "cycle",
];

fn sequence_option(key: &str, value: &Bound<'_, PyAny>) -> PyResult<SequenceOption> {
    Ok(match key {
        "cycle" => match value.cast::<PyBool>() {
            Ok(cycle) if cycle.is_true() => SequenceOption::Cycle,
            Ok(_) => SequenceOption::NoCycle,
            Err(_) => return Err(PyTypeError::new_err("the cycle option is a bool")),
        },
        "min_value" if value.is_none() => SequenceOption::NoMinValue,
        "max_value" if value.is_none() => SequenceOption::NoMaxValue,
        "increment_by" => SequenceOption::IncrementBy(integer(value, key)?),
        "min_value" => SequenceOption::MinValue(integer(value, key)?),
        "max_value" => SequenceOption::MaxValue(integer(value, key)?),
        "start_with" => SequenceOption::StartWith(integer(value, key)?),
        _ => SequenceOption::Cache(integer(value, key)?),
    })
}

/// A sequence's options from keyword arguments, an identity column's and a
/// standalone sequence's alike: `None` when none is given. A keyword that is
/// no option is a `TypeError`, as Python's own would be.
// [spec:pgorm:req:bindings.schema-sequences]
pub(super) fn sequence_options(
    options: Option<&Bound<'_, PyDict>>,
) -> PyResult<Option<SequenceOptions>> {
    let Some(options) = options else {
        return Ok(None);
    };
    for key in options.keys() {
        let key = key.extract::<String>()?;
        if !SEQUENCE_OPTIONS.contains(&key.as_str()) {
            let expected = SEQUENCE_OPTIONS.join(", ");
            return Err(PyTypeError::new_err(format!(
                "{key:?} is no sequence option; the options are {expected}"
            )));
        }
    }
    let mut chosen = Vec::new();
    for key in SEQUENCE_OPTIONS {
        if let Some(value) = options.get_item(key)? {
            chosen.push(sequence_option(key, &value)?);
        }
    }
    let mut chosen = chosen.into_iter();
    Ok(chosen
        .next()
        .map(|first| chosen.fold(SequenceOptions::from(first), SequenceOptions::and)))
}

/// Text a statement writes as a literal — a comment, an extension's version —
/// which cannot hold NUL.
pub(super) fn text(value: &Bound<'_, PyAny>, what: &str) -> PyResult<String> {
    let Ok(text) = value.extract::<String>() else {
        return Err(PyTypeError::new_err(format!("{what} is a str")));
    };
    if text.contains('\0') {
        return Err(ConstructionError::new_err(format!(
            "{what} cannot contain NUL"
        )));
    }
    Ok(text)
}

/// An enum label: data, written as a literal, of at most 63 bytes without
/// NUL, which is what PostgreSQL stores; the empty label is one.
// [spec:pgorm:req:bindings.schema-types]
pub(super) fn label(value: &Bound<'_, PyAny>) -> PyResult<String> {
    let label = text(value, "an enum label")?;
    if label.len() > 63 {
        return Err(ConstructionError::new_err(
            "an enum label is at most 63 UTF-8 bytes",
        ));
    }
    Ok(label)
}
