//! A disjoint model's shape, described from the models JavaScript declares.

use std::sync::Arc;

use neon::prelude::*;
use pgorm::pgorm_query::{ColumnType, Name, TableName, TypeName as QueryTypeName};
use pgorm::{ColumnTypeTrait, DisjointColumn, DisjointShape, DisjointTable};
use serde_json::json;

use super::{Core, DisjointHandle, RowCodec};
use crate::{
    codec::Codec,
    errors::{Failure, throw},
    values::{Tag, read},
};

/// The column type a model column of `tag` is written with: a scalar kind's
/// own (`ArrayType::column_type`), an enum of the labels `values` lists, a
/// created range over its subtype's, and an array of its element's.
fn column_type(tag: &Tag, values: &[String]) -> Result<ColumnType, Failure> {
    Ok(match tag {
        Tag::Scalar(kind) => kind.column_type().ok_or_else(|| {
            Failure::Construction(format!(
                "no column decodes as `{}`, so a disjoint model's column cannot be one",
                kind.kind_name()
            ))
        })?,
        Tag::Enum(name) => ColumnType::Enum {
            name: Name::runtime(&name.name),
            schema: name.schema.as_ref().map(Name::runtime),
            variants: values.iter().map(Name::runtime).collect(),
        },
        Tag::Created(kind) => {
            let subtype = Arc::new(column_type(&Tag::Scalar(kind.subtype.clone()), &[])?);
            let (name, schema) = (
                Name::runtime(&kind.name.name),
                kind.name.schema.as_ref().map(Name::runtime),
            );
            match kind.multirange {
                true => ColumnType::CreatedMultirange {
                    name,
                    schema,
                    subtype,
                },
                false => ColumnType::CreatedRange {
                    name,
                    schema,
                    subtype,
                },
            }
        }
        Tag::Array(element) => ColumnType::Array(Arc::new(column_type(element, values)?)),
    })
}

fn property<'cx>(
    cx: &mut FunctionContext<'cx>,
    object: Handle<'cx, JsObject>,
    key: &str,
) -> NeonResult<Handle<'cx, JsValue>> {
    object.get_value(cx, key)
}

fn text(cx: &mut FunctionContext, value: Handle<JsValue>, what: &str) -> NeonResult<String> {
    match value.downcast::<JsString, _>(cx) {
        Ok(text) => Ok(text.value(cx)),
        Err(_) => cx.throw_type_error(format!("a described table's {what} is a string")),
    }
}

fn optional_text(
    cx: &mut FunctionContext,
    value: Handle<JsValue>,
    what: &str,
) -> NeonResult<Option<String>> {
    if value.is_a::<JsNull, _>(cx) || value.is_a::<JsUndefined, _>(cx) {
        return Ok(None);
    }
    text(cx, value, what).map(Some)
}

fn flag(cx: &mut FunctionContext, value: Handle<JsValue>) -> NeonResult<bool> {
    match value.downcast::<JsBoolean, _>(cx) {
        Ok(flag) => Ok(flag.value(cx)),
        Err(_) => cx.throw_type_error("a described column's flags are booleans"),
    }
}

/// One model column, as JavaScript describes it.
fn column_of<'cx>(
    cx: &mut FunctionContext<'cx>,
    column: Handle<'cx, JsValue>,
) -> NeonResult<DisjointColumn> {
    let Ok(column) = column.downcast::<JsObject, _>(cx) else {
        return cx.throw_type_error("a described column is an object");
    };
    let name = property(cx, column, "name")?;
    let name = text(cx, name, "column name")?;
    let codec = Codec::get(cx)?;
    let kind = property(cx, column, "kind")?;
    let kind = read::kind(cx, codec, kind)?;
    let array = property(cx, column, "array")?;
    let kind = match flag(cx, array)? {
        true => Tag::Array(Box::new(kind)),
        false => kind,
    };
    let values = property(cx, column, "values")?;
    let values: Vec<String> = match values.downcast::<JsArray, _>(cx) {
        Ok(values) => values
            .to_vec(cx)?
            .into_iter()
            .map(|value| text(cx, value, "enum value"))
            .collect::<NeonResult<_>>()?,
        Err(_) => Vec::new(),
    };
    let mut def = match column_type(&kind, &values) {
        Ok(column_type) => column_type.def(),
        Err(failure) => return throw(cx, failure),
    };
    let nullable = property(cx, column, "nullable")?;
    if flag(cx, nullable)? {
        def = def.nullable();
    }
    let generated = property(cx, column, "generated")?;
    def = match optional_text(cx, generated, "generated")?.as_deref() {
        Some("always") => def.identity(),
        Some("byDefault") => def.identity_by_default(),
        Some(_) => return cx.throw_type_error(r#"a column is generated "always" or "byDefault""#),
        None => def,
    };
    let column_def = DisjointColumn::new(name, def);
    let default = property(cx, column, "default")?;
    Ok(match flag(cx, default)? {
        true => column_def.with_unstated_default(),
        false => column_def,
    })
}

/// A list of names a described table gives, each a string.
fn names<'cx>(
    cx: &mut FunctionContext<'cx>,
    list: Handle<'cx, JsValue>,
    what: &str,
) -> NeonResult<Vec<String>> {
    let Ok(list) = list.downcast::<JsArray, _>(cx) else {
        return cx.throw_type_error(format!("a described table's {what} is an array"));
    };
    list.to_vec(cx)?
        .into_iter()
        .map(|name| text(cx, name, what))
        .collect()
}

/// `table` with the unique key `key` describes: `{ name, columns,
/// nullsNotDistinct }`, its name null for PostgreSQL to choose one.
// [spec:pgorm:req:bindings.model-unique-keys+2]
fn with_unique_key<'cx>(
    cx: &mut FunctionContext<'cx>,
    table: DisjointTable,
    key: Handle<'cx, JsValue>,
) -> NeonResult<DisjointTable> {
    let Ok(key) = key.downcast::<JsObject, _>(cx) else {
        return cx.throw_type_error("a described unique key is an object");
    };
    let name = property(cx, key, "name")?;
    let name = optional_text(cx, name, "unique key's name")?;
    let columns = property(cx, key, "columns")?;
    let columns = names(cx, columns, "unique key's columns")?;
    let nulls_not_distinct = property(cx, key, "nullsNotDistinct")?;
    let nulls_not_distinct = flag(cx, nulls_not_distinct)?;
    let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
    table
        .with_unique_key(name.as_deref(), &columns, nulls_not_distinct)
        .or_else(|error| throw(cx, Failure::Construction(error.to_string())))
}

/// One model's table, as JavaScript describes it: its schema, its name, its
/// columns in field order, its key's columns and its unique keys.
// [spec:pgorm:req:bindings.model-unique-keys+2]
pub(crate) fn table_of<'cx>(
    cx: &mut FunctionContext<'cx>,
    table: Handle<'cx, JsValue>,
) -> NeonResult<DisjointTable> {
    let Ok(table) = table.downcast::<JsObject, _>(cx) else {
        return cx.throw_type_error("a described table is an object");
    };
    let schema = property(cx, table, "schema")?;
    let schema = optional_text(cx, schema, "schema")?;
    let name = property(cx, table, "table")?;
    let name = text(cx, name, "table")?;
    let columns = property(cx, table, "columns")?;
    let Ok(columns) = columns.downcast::<JsArray, _>(cx) else {
        return cx.throw_type_error("a described table's columns are an array");
    };
    let columns = columns
        .to_vec(cx)?
        .into_iter()
        .map(|column| column_of(cx, column))
        .collect::<NeonResult<Vec<_>>>()?;
    let key = property(cx, table, "key")?;
    let key = names(cx, key, "key")?;
    let unique_keys = property(cx, table, "uniqueKeys")?;
    let Ok(unique_keys) = unique_keys.downcast::<JsArray, _>(cx) else {
        return cx.throw_type_error("a described table's unique keys are an array");
    };
    let unique_keys = unique_keys.to_vec(cx)?;
    let table = match schema {
        Some(schema) => TableName::SchemaTable(Name::runtime(schema), Name::runtime(name)),
        None => TableName::Table(Name::runtime(name)),
    };
    let key: Vec<&str> = key.iter().map(String::as_str).collect();
    let mut table = DisjointTable::new(table, columns, &key)
        .or_else(|error| throw(cx, Failure::Construction(error.to_string())))?;
    for key in unique_keys {
        table = with_unique_key(cx, table, key)?;
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
            Some((column, field, kind_type))
        }
        Err(_) if kinded.is_a::<JsNull, _>(&mut cx) => None,
        Err(_) => {
            return cx
                .throw_type_error("a disjoint model is kinded by [column, field, type] or null");
        }
    };
    let kinds = cx.argument::<JsArray>(3)?.to_vec(&mut cx)?;
    let mut descriptions = Vec::with_capacity(kinds.len());
    let mut tables = Vec::with_capacity(kinds.len());
    for kind in kinds {
        let Ok(kind) = kind.downcast::<JsArray, _>(&mut cx) else {
            return cx.throw_type_error("a kind is [value, [model, table] or null]");
        };
        let value: Handle<JsValue> = kind.get(&mut cx, 0)?;
        let value = text(&mut cx, value, "kind value")?;
        let table: Handle<JsValue> = kind.get(&mut cx, 1)?;
        if table.is_a::<JsNull, _>(&mut cx) {
            descriptions.push(json!({"value": value, "model": null}));
            tables.push((value, None));
        } else {
            let (model, table) = described(&mut cx, table)?;
            descriptions.push(json!({"value": value, "model": model}));
            tables.push((value, Some(table)));
        }
    }
    let (shape, kind_field, kind_type) = match kinded {
        Some((kind_column, kind_field, kind_type)) => {
            let mut type_name = QueryTypeName::new(Name::runtime(&kind_type.name));
            if let Some(schema) = &kind_type.schema {
                type_name = type_name.schema(Name::runtime(schema));
            }
            (
                DisjointShape::new(base, &kind_column, type_name, tables),
                Some(kind_field),
                json!({"schema": kind_type.schema, "name": kind_type.name}),
            )
        }
        None => (
            DisjointShape::unkinded(base, tables),
            None,
            json!({"schema": null, "name": null}),
        ),
    };
    let shape = match shape {
        Ok(shape) => shape,
        Err(error) => return throw(&mut cx, Failure::Construction(error.to_string())),
    };
    let core = Core {
        shape,
        codec: RowCodec::Models,
        description: json!({
            "name": name,
            "base": base_name,
            "kindField": kind_field,
            "kindType": kind_type,
            "kinds": descriptions,
        }),
        rust: None,
    };
    Ok(cx.boxed(DisjointHandle(Arc::new(core))).upcast())
}
