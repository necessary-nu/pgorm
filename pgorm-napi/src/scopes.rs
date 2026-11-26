//! A JavaScript context and a declared model's scope columns, as
//! pgorm-bindings decides what each holds a statement to
//! (`bindings.scopes`). `lib/scopes.js` reads a program's scope value,
//! `every(..)` or context object into the `{ label, held }` each function
//! here is given; what a scope does with it is pgorm-bindings' own.

use std::sync::Arc;

use neon::{prelude::*, types::Finalize};
use pgorm::pgorm_query::Value;
use pgorm_bindings::{
    entity::EntityInfo,
    scope::{self, Context as Given, DeclaredScopes, Holding, ScopeColumn, Scoping},
};

use crate::{
    codec::Codec,
    connect::DIALECT,
    entities::{ActiveHandle, EntityHandle, column_value},
    errors::throw,
    statements::{self, Node},
    values::read,
};

/// The Rust scope values a registered statement runs under, held by the
/// JavaScript statement until it runs.
#[derive(Debug)]
pub(crate) struct ScopingHandle(pub(crate) Arc<Scoping>);

impl Finalize for ScopingHandle {}

pub(crate) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("scopeDeclared", declared)?;
    cx.export_function("scopeCondition", condition)?;
    cx.export_function("scopeFill", fill)?;
    cx.export_function("scopeImmutable", immutable)?;
    cx.export_function("scopeUpsert", upsert)?;
    cx.export_function("scopeJoin", join)?;
    cx.export_function("scopeColumn", column)?;
    cx.export_function("entityScoping", entity_scoping)?;
    cx.export_function("entityJoinScoping", entity_join_scoping)?;
    cx.export_function("entityScopes", entity_scopes)?;
    cx.export_function("entityPipelineFor", entity_pipeline_for)?;
    Ok(())
}

fn string(cx: &mut FunctionContext, value: Handle<JsValue>, what: &str) -> NeonResult<String> {
    match value.downcast::<JsString, _>(cx) {
        Ok(text) => Ok(text.value(cx)),
        Err(_) => cx.throw_type_error(format!("a scope's {what} is a string")),
    }
}

/// The scope columns at arguments `index` and `index + 1`: the table, and
/// each column's `[field, column, scope]`.
fn scopes_at(cx: &mut FunctionContext, index: usize) -> NeonResult<DeclaredScopes> {
    let table = cx.argument::<JsString>(index)?.value(cx);
    let mut columns = Vec::new();
    for column in cx.argument::<JsArray>(index + 1)?.to_vec(cx)? {
        let parts = column.downcast_or_throw::<JsArray, _>(cx)?.to_vec(cx)?;
        let [field, sql, name] = parts.as_slice() else {
            return cx.throw_type_error("a scope column is [field, column, scope]");
        };
        columns.push(ScopeColumn {
            field: string(cx, *field, "field")?,
            column: string(cx, *sql, "column")?,
            scope: string(cx, *name, "name")?,
        });
    }
    DeclaredScopes::new(table, columns).or_else(|failure| throw(cx, failure))
}

/// The context at argument `index`, `{ label, held: [[name, value, every]] }`,
/// or `None` for `null`, a statement given none.
fn context_at<'cx>(
    cx: &mut FunctionContext<'cx>,
    index: usize,
) -> NeonResult<Option<Given<Handle<'cx, JsValue>>>> {
    let value = cx.argument::<JsValue>(index)?;
    if value.is_a::<JsNull, _>(cx) {
        return Ok(None);
    }
    let object = value.downcast_or_throw::<JsObject, _>(cx)?;
    let label = object.get::<JsString, _, _>(cx, "label")?.value(cx);
    let mut held = Vec::new();
    for item in object.get::<JsArray, _, _>(cx, "held")?.to_vec(cx)? {
        let item = item.downcast_or_throw::<JsArray, _>(cx)?;
        let name = item.get::<JsString, _, _>(cx, 0)?.value(cx);
        let value: Handle<JsValue> = item.get(cx, 1)?;
        let every = item.get::<JsBoolean, _, _>(cx, 2)?.value(cx);
        held.push((
            name,
            if every {
                Holding::Every
            } else {
                Holding::One(value)
            },
        ));
    }
    Given::new(label, held)
        .map(Some)
        .or_else(|failure| throw(cx, failure))
}

/// `scopeDeclared(table, columns)`: refuse a model holding one scope in two
/// columns, as it is declared.
// [spec:pgorm:req:bindings.scopes]
fn declared(mut cx: FunctionContext) -> JsResult<JsValue> {
    scopes_at(&mut cx, 0)?;
    Ok(cx.undefined().upcast())
}

/// `scopeCondition(table, columns, context)`: each `[field, value]` a
/// statement under `context` is held to equal, refused under `null` or a
/// context not holding each scope.
// [spec:pgorm:req:bindings.scopes]
fn condition(mut cx: FunctionContext) -> JsResult<JsValue> {
    let scopes = scopes_at(&mut cx, 0)?;
    let context = context_at(&mut cx, 2)?;
    let pairs = match scopes.condition(context.as_ref(), DIALECT) {
        Ok(pairs) => pairs,
        Err(failure) => return throw(&mut cx, failure),
    };
    let array = cx.empty_array();
    for (index, (field, value)) in pairs.into_iter().enumerate() {
        let pair = cx.empty_array();
        let field = cx.string(field);
        pair.set(&mut cx, 0, field)?;
        pair.set(&mut cx, 1, *value)?;
        array.set(&mut cx, index as u32, pair)?;
    }
    Ok(array.upcast())
}

/// A value an insert compares on a scope column: the `Value` its column
/// made of it, `None` for an expression, which equals no value.
struct Compared<'cx> {
    datum: Option<Value>,
    value: Handle<'cx, JsValue>,
}

impl PartialEq for Compared<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.datum.is_some() && self.datum == other.datum
    }
}

/// `scopeFill(table, columns, context, row, convert)`: each `[field, value]`
/// an insert under `context` sets in `row`, `convert(field, value)` making a
/// value of either side the field's `Value`, or `null` for an expression.
// [spec:pgorm:req:bindings.scopes]
fn fill(mut cx: FunctionContext) -> JsResult<JsValue> {
    let scopes = scopes_at(&mut cx, 0)?;
    let context = context_at(&mut cx, 2)?;
    let row = cx.argument::<JsObject>(3)?;
    let convert = cx.argument::<JsFunction>(4)?;
    let mut present = Vec::new();
    for column in scopes.columns() {
        if let Some(value) = row.get_opt::<JsValue, _, _>(&mut cx, column.field.as_str())?
            && !value.is_a::<JsUndefined, _>(&mut cx)
        {
            present.push((column.field.clone(), value));
        }
    }
    let codec = Codec::get(&mut cx)?;
    let filled = scopes.fill(
        context.as_ref(),
        |field| {
            present
                .iter()
                .find(|(name, _)| name == field)
                .map(|(_, value)| *value)
        },
        |column, value| -> NeonResult<Compared> {
            let field = cx.string(&column.field);
            let undefined = cx.undefined();
            let made = convert.call(&mut cx, undefined, [field.upcast(), *value])?;
            if made.is_a::<JsNull, _>(&mut cx) {
                return Ok(Compared {
                    datum: None,
                    value: made,
                });
            }
            let datum = read::infer(&mut cx, codec, made)?.map(|tagged| tagged.datum);
            Ok(Compared { datum, value: made })
        },
        DIALECT,
    )?;
    let fills = match filled {
        Ok(fills) => fills,
        Err(failure) => return throw(&mut cx, failure),
    };
    let array = cx.empty_array();
    for (index, (field, value)) in fills.into_iter().enumerate() {
        let pair = cx.empty_array();
        let field = cx.string(field);
        pair.set(&mut cx, 0, field)?;
        pair.set(&mut cx, 1, value.value)?;
        array.set(&mut cx, index as u32, pair)?;
    }
    Ok(array.upcast())
}

/// `scopeImmutable(table, columns, fields)`: refuse an update setting a
/// scope column.
// [spec:pgorm:req:bindings.scopes]
fn immutable(mut cx: FunctionContext) -> JsResult<JsValue> {
    let scopes = scopes_at(&mut cx, 0)?;
    let mut fields = Vec::new();
    for field in cx.argument::<JsArray>(2)?.to_vec(&mut cx)? {
        fields.push(string(&mut cx, field, "field")?);
    }
    match scopes.immutable(fields.iter().map(String::as_str)) {
        Ok(()) => Ok(cx.undefined().upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// `scopeUpsert(table, columns, action)`: refuse a conflict action whose
/// `DO UPDATE SET` assigns a scope column.
// [spec:pgorm:req:bindings.scopes]
fn upsert(mut cx: FunctionContext) -> JsResult<JsValue> {
    let scopes = scopes_at(&mut cx, 0)?;
    let action = cx.argument::<JsValue>(2)?;
    let clause = statements::conflict_clause(&mut cx, action)?;
    match scopes.upsert(&clause) {
        Ok(()) => Ok(cx.undefined().upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// `scopeJoin(table, columns)`: refuse a join into a scoped table by a form
/// taking no context.
// [spec:pgorm:req:bindings.scopes]
fn join(mut cx: FunctionContext) -> JsResult<JsValue> {
    let scopes = scopes_at(&mut cx, 0)?;
    if scopes.columns().is_empty() {
        return Ok(cx.undefined().upcast());
    }
    let refusal = scope::unscoped_join(scopes.table(), &scopes.names(), DIALECT);
    throw(&mut cx, refusal)
}

/// `scopeColumn(nullable, array, generated)`: refuse a column declared to
/// hold a scope where it is nullable, an array or generated.
// [spec:pgorm:req:bindings.scopes]
fn column(mut cx: FunctionContext) -> JsResult<JsValue> {
    let nullable = cx.argument::<JsBoolean>(0)?.value(&mut cx);
    let array = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    let generated = cx.argument::<JsBoolean>(2)?.value(&mut cx);
    match scope::check_column(nullable, array, generated) {
        Ok(()) => Ok(cx.undefined().upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// The context at argument `index` as the scopes `info`'s registry knows
/// convert its values, `None` for a statement given none: a scope it does
/// not know is no Rust scope a registered statement reads, and is left out.
fn registered_context(
    cx: &mut FunctionContext,
    info: &EntityInfo,
    index: usize,
) -> NeonResult<Option<Given<Value>>> {
    let Some(context) = context_at(cx, index)? else {
        return Ok(None);
    };
    let scopes = info.scopes.get();
    let mut held = Vec::new();
    for (name, holding) in context.held() {
        let Some(known) = scopes.iter().find(|known| known.name == name) else {
            continue;
        };
        let holding = match holding {
            Holding::One(value) => Holding::One(column_value(cx, &known.column, *value)?),
            Holding::Every => Holding::Every,
        };
        held.push((name.to_owned(), holding));
    }
    Given::new(context.label(), held)
        .map(Some)
        .or_else(|failure| throw(cx, failure))
}

/// `entityScoping(registered, context)`: the Rust scope values a
/// registered statement on the entity of `registered`, an entity or one of
/// its ActiveModels, runs under, refused under `null` or a context not
/// holding the scopes the entity declares.
// [spec:pgorm:req:bindings.scopes-registered]
fn entity_scoping(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registered = cx.argument::<JsValue>(0)?;
    let info = match registered.downcast::<JsBox<EntityHandle>, _>(&mut cx) {
        Ok(entity) => entity.0.info().clone(),
        Err(_) => cx.argument::<JsBox<ActiveHandle>>(0)?.0.info().clone(),
    };
    let context = registered_context(&mut cx, &info, 1)?;
    match info.scoping(context.as_ref(), DIALECT) {
        Ok(scoping) => Ok(cx.boxed(ScopingHandle(scoping)).upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// `entityScopes(entity)`: the scopes the entity declares, by name.
fn entity_scopes(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = cx.argument::<JsBox<EntityHandle>>(0)?.0.clone();
    let array = cx.empty_array();
    for (index, name) in entity.info().scoped.iter().enumerate() {
        let name = cx.string(name);
        array.set(&mut cx, index as u32, name)?;
    }
    Ok(array.upcast())
}

/// `entityPipelineFor(entity, scoping)`: `Pipeline::from_for` the entity
/// under the scoping `entityScoping` made.
// [spec:pgorm:req:bindings.scopes-registered]
fn entity_pipeline_for(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = cx.argument::<JsBox<EntityHandle>>(0)?.0.clone();
    let scoping = cx.argument::<JsBox<ScopingHandle>>(1)?.0.clone();
    Ok(cx.boxed(Node::from(entity.pipeline(&scoping))).upcast())
}

/// `entityJoinScoping(entity, relation, context)`: how a join along
/// `entity`'s relation is held to its joined table's scopes under
/// `context`, `null` for a plain join, refused where the table is scoped and
/// `context` does not hold its scopes.
// [spec:pgorm:req:bindings.scopes-registered]
fn entity_join_scoping(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = cx.argument::<JsBox<EntityHandle>>(0)?.0.clone();
    let relation = cx.argument::<JsString>(1)?.value(&mut cx);
    let def = match entity.relation(&relation) {
        Ok(def) => def,
        Err(failure) => return throw(&mut cx, failure),
    };
    let context = registered_context(&mut cx, entity.info(), 2)?;
    match entity.info().join_scoping(&def, context.as_ref(), DIALECT) {
        Ok(Some(scoping)) => Ok(cx.boxed(ScopingHandle(scoping)).upcast()),
        Ok(None) => Ok(cx.null().upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}
