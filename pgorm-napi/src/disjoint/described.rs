//! A disjoint model's shape, described from the models JavaScript declares.

use std::sync::Arc;

use neon::prelude::*;
use pgorm::{DisjointColumn, DisjointTable};
use pgorm_bindings::entity::{DisjointCore, Kinded};

use super::DisjointHandle;
use crate::{
    codec::Codec,
    connect::DIALECT,
    errors::{Failure, throw},
    models::{columns, flag, names, property, table_name, text, unique_keys},
    values::{Tag, read},
};

/// One model's table, as JavaScript describes it: its schema, its name, its
/// columns in field order, its key's columns and its unique keys, each column
/// lowered as a model's schema lowers it.
// [spec:pgorm:req:bindings.model-unique-keys+2]
pub(crate) fn table_of<'cx>(
    cx: &mut FunctionContext<'cx>,
    table: Handle<'cx, JsValue>,
) -> NeonResult<DisjointTable> {
    let Ok(table_object) = table.downcast::<JsObject, _>(cx) else {
        return cx.throw_type_error("a described table is an object");
    };
    let table = table_object;
    let columns = columns(cx, table)?
        .into_iter()
        .map(|(name, def, unstated)| {
            let column = DisjointColumn::new(name, def);
            match unstated {
                true => column.with_unstated_default(),
                false => column,
            }
        })
        .collect();
    let key = property(cx, table, "key")?;
    let key = names(cx, key, "key")?;
    let unique_keys = unique_keys(cx, table)?;
    let name = table_name(cx, table)?;
    let key: Vec<&str> = key.iter().map(String::as_str).collect();
    let mut table = DisjointTable::new(name, columns, &key)
        .or_else(|error| throw(cx, Failure::Construction(error.to_string())))?;
    let auto_increment = property(cx, table_object, "autoIncrement")?;
    if flag(cx, auto_increment)? {
        table = table.auto_increment();
    }
    for (name, columns, nulls_not_distinct) in unique_keys {
        let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
        table = table
            .with_unique_key(name.as_deref(), &columns, nulls_not_distinct)
            .or_else(|error| throw(cx, Failure::Construction(error.to_string())))?;
    }
    Ok(table)
}

/// A model's name beside its described table.
fn described<'cx>(
    cx: &mut FunctionContext<'cx>,
    pair: Handle<'cx, JsValue>,
) -> NeonResult<(String, DisjointTable)> {
    let Ok(pair) = pair.downcast::<JsArray, _>(cx) else {
        return cx.throw_type_error("a described model is [name, table]");
    };
    let name: Handle<JsValue> = pair.get(cx, 0)?;
    let name = text(cx, name, "model name")?;
    let table: Handle<JsValue> = pair.get(cx, 1)?;
    Ok((name, table_of(cx, table)?))
}

/// `disjointModel(name, base, kinded, kinds)`: the disjoint model `name`
/// over the described `base` and each kind in order, `[value, [model, table]
/// or null]`. A kinded model's `kinded` is `[kindColumn, kindField,
/// kindType]`, its kind column, the field that column is of and its kind
/// type; an unkinded model's is `null`, its base holding no kind. What the
/// shape refuses is a `ConstructionError`, thrown as the model is declared.
// [spec:pgorm:req:bindings.disjoint+2]
// [spec:pgorm:req:bindings.disjoint-unkinded]
pub(crate) fn disjoint_model(mut cx: FunctionContext) -> JsResult<JsValue> {
    let name = cx.argument::<JsString>(0)?.value(&mut cx);
    let base = cx.argument::<JsValue>(1)?;
    let (base_name, base) = described(&mut cx, base)?;
    let kinded = cx.argument::<JsValue>(2)?;
    let kinded = match kinded.downcast::<JsArray, _>(&mut cx) {
        Ok(kinded) => {
            let column: Handle<JsValue> = kinded.get(&mut cx, 0)?;
            let column = text(&mut cx, column, "kind column")?;
            let field: Handle<JsValue> = kinded.get(&mut cx, 1)?;
            let field = text(&mut cx, field, "kind field")?;
            let codec = Codec::get(&mut cx)?;
            let kind_type: Handle<JsValue> = kinded.get(&mut cx, 2)?;
            let Tag::Enum(kind_type) = read::kind(&mut cx, codec, kind_type)? else {
                return cx.throw_type_error("a disjoint model's kind type is a TypeName");
            };
            Some(Kinded {
                column,
                field,
                kind_type,
            })
        }
        Err(_) if kinded.is_a::<JsNull, _>(&mut cx) => None,
        Err(_) => {
            return cx
                .throw_type_error("a disjoint model is kinded by [column, field, type] or null");
        }
    };
    let kinds = cx.argument::<JsArray>(3)?.to_vec(&mut cx)?;
    let mut tables = Vec::with_capacity(kinds.len());
    for kind in kinds {
        let Ok(kind) = kind.downcast::<JsArray, _>(&mut cx) else {
            return cx.throw_type_error("a kind is [value, [model, table] or null]");
        };
        let value: Handle<JsValue> = kind.get(&mut cx, 0)?;
        let value = text(&mut cx, value, "kind value")?;
        let table: Handle<JsValue> = kind.get(&mut cx, 1)?;
        if table.is_a::<JsNull, _>(&mut cx) {
            tables.push((value, None));
        } else {
            tables.push((value, Some(described(&mut cx, table)?)));
        }
    }
    match DisjointCore::model(DIALECT, &name, (base_name, base), kinded, tables) {
        Ok(core) => Ok(cx.boxed(DisjointHandle(Arc::new(core))).upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}
