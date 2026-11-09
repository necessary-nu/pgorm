//! What the schema builders read from their arguments: the relation or type a
//! statement names, an options object and the closed choices it carries, a
//! key's columns, sequence options and the text a label or comment is.
//!
//! An options object is read strictly: a key the builder does not know is a
//! `TypeError`, as an option of the wrong JavaScript type is, so a misspelt
//! option is refused rather than left out of the statement.

use neon::{prelude::*, types::JsBigInt};
use pgorm::pgorm_query::{
    Collation, Deferrability, DropBehavior, Enforcement, ForeignKeyAction, IntoCollation, Name,
    SequenceOption, SequenceOptions, TableName, extension::TypeRef,
};

pub(in crate::statements) use super::super::args::pick;
use super::super::{
    Node,
    args::{absent, arg, name, node, refuse},
};
use crate::{codec::Codec, values::read};

/// The options object at `index`, its keys checked against `known`; `None`
/// when it is left out.
pub(in crate::statements) fn object<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
    known: &[&str],
) -> NeonResult<Option<Handle<'cx, JsObject>>> {
    let value = arg(cx, index);
    if absent(cx, value) {
        return Ok(None);
    }
    let Ok(object) = value.downcast::<JsObject, _>(cx) else {
        return cx.throw_type_error("options are an object");
    };
    let keys = object.get_own_property_names(cx)?.to_vec(cx)?;
    for key in keys {
        let key = read::string(cx, key)?;
        if !known.contains(&key.as_str()) {
            let expected = known.join(", ");
            return cx.throw_type_error(format!(
                "{key:?} is no option here; the options are {expected}"
            ));
        }
    }
    Ok(Some(object))
}

/// The option `key`, `None` when it is absent or `undefined`.
pub(in crate::statements) fn get<'cx>(
    cx: &mut Cx<'cx>,
    options: Option<Handle<'cx, JsObject>>,
    key: &str,
) -> NeonResult<Option<Handle<'cx, JsValue>>> {
    let Some(options) = options else {
        return Ok(None);
    };
    let value: Handle<JsValue> = options.get_value(cx, key)?;
    Ok((!value.is_a::<JsUndefined, _>(cx)).then_some(value))
}

/// A boolean option, `false` when absent.
pub(super) fn flag<'cx>(
    cx: &mut Cx<'cx>,
    options: Option<Handle<'cx, JsObject>>,
    key: &str,
) -> NeonResult<bool> {
    match get(cx, options, key)? {
        None => Ok(false),
        Some(value) => match value.downcast::<JsBoolean, _>(cx) {
            Ok(flag) => Ok(flag.value(cx)),
            Err(_) => cx.throw_type_error(format!("the {key} option is a boolean")),
        },
    }
}

pub(super) fn name_option<'cx>(
    cx: &mut Cx<'cx>,
    options: Option<Handle<'cx, JsObject>>,
    key: &str,
) -> NeonResult<Option<Name>> {
    match get(cx, options, key)? {
        None => Ok(None),
        Some(value) => name(cx, value).map(Some),
    }
}

/// A string option that is one of `choices`, given as `(spelling, meaning)`.
pub(super) fn choice<'cx, T: Clone>(
    cx: &mut Cx<'cx>,
    options: Option<Handle<'cx, JsObject>>,
    key: &str,
    choices: &[(&str, T)],
) -> NeonResult<Option<T>> {
    let Some(value) = get(cx, options, key)? else {
        return Ok(None);
    };
    pick(cx, value, key, choices).map(Some)
}

pub(super) const DEFERRABILITY: &[(&str, Deferrability)] = &[
    ("notDeferrable", Deferrability::NotDeferrable),
    (
        "deferrableInitiallyImmediate",
        Deferrability::DeferrableInitiallyImmediate,
    ),
    (
        "deferrableInitiallyDeferred",
        Deferrability::DeferrableInitiallyDeferred,
    ),
];

pub(super) const ENFORCEMENT: &[(&str, Enforcement)] = &[
    ("enforced", Enforcement::Enforced),
    ("notEnforced", Enforcement::NotEnforced),
];

pub(super) const BEHAVIOR: &[(&str, DropBehavior)] = &[
    ("restrict", DropBehavior::Restrict),
    ("cascade", DropBehavior::Cascade),
];

pub(crate) const ACTIONS: &[(&str, ForeignKeyAction)] = &[
    ("noAction", ForeignKeyAction::NoAction),
    ("restrict", ForeignKeyAction::Restrict),
    ("cascade", ForeignKeyAction::Cascade),
    ("setNull", ForeignKeyAction::SetNull),
    ("setDefault", ForeignKeyAction::SetDefault),
];

/// The identifiers of an array, which may be empty.
pub(in crate::statements) fn names<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
    what: &str,
) -> NeonResult<Vec<Name>> {
    let Ok(array) = value.downcast::<JsArray, _>(cx) else {
        return cx.throw_type_error(format!("{what} is an array of names"));
    };
    let items = array.to_vec(cx)?;
    items.into_iter().map(|item| name(cx, item)).collect()
}

/// A key's columns: one name, or a non-empty array of them. The first is
/// apart, as pgorm-query's key, index and foreign key take it.
// [spec:pgorm:req:bindings.schema-tables]
pub(super) fn columns<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
    what: &str,
) -> NeonResult<(Name, Vec<Name>)> {
    let mut all = if value.is_a::<JsString, _>(cx) {
        vec![name(cx, value)?]
    } else {
        names(cx, value, what)?
    };
    if all.is_empty() {
        return refuse(cx, format!("{what} names at least one column"));
    }
    let first = all.remove(0);
    Ok((first, all))
}

/// The relation a statement names: a name, or a `Table` with no alias, its
/// schema kept. A sequence is a relation too.
// [spec:pgorm:req:bindings.schema]
pub(crate) fn relation<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<TableName> {
    if value.is_a::<JsString, _>(cx) {
        return Ok(TableName::Table(name(cx, value)?));
    }
    match node(cx, value) {
        Some(Node::Table(table)) if table.alias.is_none() => Ok(table.name),
        Some(Node::Table(_)) => refuse(
            cx,
            "a DDL statement names a Table without an alias: an alias belongs to a query",
        ),
        _ => cx.throw_type_error("a table or sequence is named by a string or a Table"),
    }
}

pub(super) fn relation_at<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<TableName> {
    let value = arg(cx, index);
    relation(cx, value)
}

/// One relation or a non-empty array of them, for a drop.
pub(super) fn relations_at<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<(TableName, Vec<TableName>)> {
    let value = arg(cx, index);
    let Ok(array) = value.downcast::<JsArray, _>(cx) else {
        return Ok((relation(cx, value)?, Vec::new()));
    };
    let mut all = Vec::new();
    for item in array.to_vec(cx)? {
        all.push(relation(cx, item)?);
    }
    if all.is_empty() {
        return refuse(cx, "a drop names at least one relation");
    }
    let first = all.remove(0);
    Ok((first, all))
}

/// The type a statement names: a name, or a `TypeName`, its schema kept.
// [spec:pgorm:req:bindings.schema-types]
pub(super) fn type_ref<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<TypeRef> {
    if value.is_a::<JsString, _>(cx) {
        return Ok(TypeRef::Type(name(cx, value)?));
    }
    let codec = Codec::get(cx)?;
    match read::named_type(cx, codec, value)? {
        Some(named) => {
            let type_name = Name::runtime(named.name);
            Ok(match named.schema {
                Some(schema) => TypeRef::SchemaType(Name::runtime(schema), type_name),
                None => TypeRef::Type(type_name),
            })
        }
        None => cx.throw_type_error("a type is named by a string or a TypeName"),
    }
}

/// A collation: a name, or `{ name, schema }`.
pub(super) fn collation<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<Collation> {
    if value.is_a::<JsString, _>(cx) {
        return Ok(name(cx, value)?.into_collation());
    }
    let Ok(object) = value.downcast::<JsObject, _>(cx) else {
        return cx.throw_type_error("a collation is a name or { name, schema }");
    };
    let collation: Handle<JsValue> = object.get_value(cx, "name")?;
    let collation = name(cx, collation)?;
    let schema: Handle<JsValue> = object.get_value(cx, "schema")?;
    Ok(if absent(cx, schema) {
        collation.into_collation()
    } else {
        (name(cx, schema)?, collation).into_collation()
    })
}

pub(super) fn collation_option<'cx>(
    cx: &mut Cx<'cx>,
    options: Option<Handle<'cx, JsObject>>,
) -> NeonResult<Option<Collation>> {
    match get(cx, options, "collation")? {
        None => Ok(None),
        Some(value) => collation(cx, value).map(Some),
    }
}

/// A 64-bit integer: a safe-integer number or a `bigint` within `i64`.
pub(in crate::statements) fn integer<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
    what: &str,
) -> NeonResult<i64> {
    if !value.is_a::<JsNumber, _>(cx) && !value.is_a::<JsBigInt, _>(cx) {
        return cx.throw_type_error(format!("{what} is a number or a bigint"));
    }
    let read = if let Ok(number) = value.downcast::<JsNumber, _>(cx) {
        let number = number.value(cx);
        #[allow(clippy::cast_possible_truncation)]
        (number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0).then_some(number as i64)
    } else if let Ok(big) = value.downcast::<JsBigInt, _>(cx) {
        big.to_i64(cx).ok()
    } else {
        None
    };
    match read {
        Some(integer) => Ok(integer),
        None => refuse(
            cx,
            format!("{what} is a safe-integer number or a bigint within PostgreSQL's bigint"),
        ),
    }
}

/// The keys a sequence's options object takes.
const SEQUENCE_OPTIONS: &[&str] = &[
    "incrementBy",
    "minValue",
    "maxValue",
    "startWith",
    "cache",
    "cycle",
];

/// A sequence's options from the object at `index`, an identity column's and
/// a standalone sequence's alike: `None` when it is left out or empty. A
/// bound of `null` is its `NO` form.
// [spec:pgorm:req:bindings.schema-sequences]
pub(super) fn sequence_options<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<Option<SequenceOptions>> {
    let options = object(cx, index, SEQUENCE_OPTIONS)?;
    let mut chosen = Vec::new();
    for key in SEQUENCE_OPTIONS {
        let Some(value) = get(cx, options, key)? else {
            continue;
        };
        let option = match *key {
            "cycle" => match value.downcast::<JsBoolean, _>(cx) {
                Ok(cycle) if cycle.value(cx) => SequenceOption::Cycle,
                Ok(_) => SequenceOption::NoCycle,
                Err(_) => return cx.throw_type_error("the cycle option is a boolean"),
            },
            "minValue" if value.is_a::<JsNull, _>(cx) => SequenceOption::NoMinValue,
            "maxValue" if value.is_a::<JsNull, _>(cx) => SequenceOption::NoMaxValue,
            key => {
                let number = integer(cx, value, key)?;
                match key {
                    "incrementBy" => SequenceOption::IncrementBy(number),
                    "minValue" => SequenceOption::MinValue(number),
                    "maxValue" => SequenceOption::MaxValue(number),
                    "startWith" => SequenceOption::StartWith(number),
                    _ => SequenceOption::Cache(number),
                }
            }
        };
        chosen.push(option);
    }
    let mut chosen = chosen.into_iter();
    Ok(chosen
        .next()
        .map(|first| chosen.fold(SequenceOptions::from(first), SequenceOptions::and)))
}

/// Text a statement writes as a literal — a comment, an extension's version —
/// which cannot hold NUL.
pub(in crate::statements) fn text<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
    what: &str,
) -> NeonResult<String> {
    if !value.is_a::<JsString, _>(cx) {
        return cx.throw_type_error(format!("{what} is a string"));
    }
    let text = read::string(cx, value)?;
    if text.contains('\0') {
        return refuse(cx, format!("{what} cannot contain NUL"));
    }
    Ok(text)
}

/// An enum label: data, written as a literal, of at most 63 bytes without
/// NUL, which is what PostgreSQL stores; the empty label is one.
// [spec:pgorm:req:bindings.schema-types]
pub(super) fn label<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<String> {
    let label = text(cx, value, "an enum label")?;
    if label.len() > 63 {
        return refuse(cx, "an enum label is at most 63 UTF-8 bytes");
    }
    Ok(label)
}
