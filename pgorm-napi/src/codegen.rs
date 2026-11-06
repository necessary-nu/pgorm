//! Declared models generated from DDL: pgorm-codegen's TypeScript writer,
//! the one reader and model the Rust entities are generated from, behind
//! the `modelsFromSql` that `lib/codegen.js` wraps.

use neon::prelude::*;
use pgorm_codegen::{ModelWriterOptions, TableIdent, UnkindedDisjoint, typescript_models_from_sql};

use crate::errors::{Failure, throw};

pub(crate) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("modelsFromSql", models_from_sql)
}

/// A table as `lib/codegen.js` passes it: `[schema, name]`, the schema
/// `null` where it names none.
fn table<'cx>(
    cx: &mut FunctionContext<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<TableIdent> {
    let pair = value.downcast_or_throw::<JsArray, _>(cx)?;
    let schema = pair.get_value(cx, 0)?;
    let schema = match schema.downcast::<JsString, _>(cx) {
        Ok(schema) => Some(schema.value(cx)),
        Err(_) => None,
    };
    let name = pair.get::<JsString, _, _>(cx, 1)?.value(cx);
    Ok(TableIdent {
        table: name,
        schema,
    })
}

/// `modelsFromSql(ddl, schemaName, unkindedDisjoint)`: the module
/// pgorm-codegen's TypeScript writer writes for `ddl`, each unkinded set
/// `[base, kinds, unit]`; a refusal is a `ConstructionError` with
/// pgorm-codegen's message.
// [spec:pgorm:req:bindings.models-from-sql]
fn models_from_sql(mut cx: FunctionContext) -> JsResult<JsValue> {
    let ddl = cx.argument::<JsString>(0)?.value(&mut cx);
    let schema = cx.argument::<JsValue>(1)?;
    let schema_name = match schema.downcast::<JsString, _>(&mut cx) {
        Ok(schema) => Some(schema.value(&mut cx)),
        Err(_) => None,
    };
    let sets = cx.argument::<JsArray>(2)?.to_vec(&mut cx)?;
    let mut unkinded_disjoint = Vec::with_capacity(sets.len());
    for set in sets {
        let set = set.downcast_or_throw::<JsArray, _>(&mut cx)?;
        let base = set.get_value(&mut cx, 0)?;
        let base = table(&mut cx, base)?;
        let mut kinds = Vec::new();
        for kind in set.get::<JsArray, _, _>(&mut cx, 1)?.to_vec(&mut cx)? {
            kinds.push(table(&mut cx, kind)?);
        }
        let unit = set.get_value(&mut cx, 2)?;
        let unit = match unit.downcast::<JsString, _>(&mut cx) {
            Ok(unit) => Some(unit.value(&mut cx)),
            Err(_) => None,
        };
        unkinded_disjoint.push(UnkindedDisjoint { base, kinds, unit });
    }
    let options = ModelWriterOptions {
        schema_name,
        unkinded_disjoint,
    };
    match typescript_models_from_sql(&ddl, options) {
        Ok(output) => {
            let source: String = output.files.into_iter().map(|file| file.content).collect();
            Ok(cx.string(source).upcast())
        }
        Err(error) => throw(&mut cx, Failure::Construction(error.to_string())),
    }
}
