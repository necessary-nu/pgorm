//! Disjoint models and registered `#[derive(Disjoint)]` enums, over pgorm's
//! [`DisjointShape`].
//!
//! Either is one native handle: a shape, from models JavaScript declares or
//! from the registered enum, and a codec reading each row back. Every
//! statement is the shape's — the read and its narrowing, the writes, the
//! cursor, the pages and the schema — so a model's statements are the ones
//! the Rust declaration of the same tables sends; the codec only reads the
//! rows back, a model's as tagged values `lib/disjoint.js` views by field and
//! a registration's as its entities' own models.

mod described;
mod evolve;

pub(crate) use described::table_of;
mod registered;
mod rows;

use std::{num::NonZeroU64, sync::Arc};

use neon::{prelude::*, types::Finalize};
use pgorm::pgorm_query::{IntoCondition, SelectStatement, Value, ValueTuple};
use pgorm::{ActiveValue, DisjointTable, DisjointValues, Error};

use pgorm_bindings::entity::{
    DisjointCore, Entity, KindRow, RowCodec, active_values, foreign_active,
};

use crate::{
    codec::Codec,
    connect::job::{JobHandle, Output},
    entities::{ActiveHandle, column_value, inspected},
    errors::{Failure, failure, throw},
    statements::{Node, node, predicate},
    values::read,
};

pub(crate) use rows::Rows;

/// A disjoint model or registration as JavaScript holds it.
#[derive(Debug)]
pub(crate) struct DisjointHandle(pub(crate) Arc<DisjointCore>);

impl Finalize for DisjointHandle {}

pub(crate) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    for (name, export) in EXPORTS {
        cx.export_function(name, *export)?;
    }
    Ok(())
}

type Export = fn(FunctionContext) -> JsResult<JsValue>;

const EXPORTS: &[(&str, Export)] = &[
    ("disjointModel", described::disjoint_model),
    ("disjointEntity", registered::disjoint_entity),
    ("disjointEntities", registered::disjoint_entities),
    ("disjointDescribe", describe),
    ("disjointKinds", kinds),
    ("disjointKeyColumns", key_columns),
    ("disjointResultNames", result_names),
    ("disjointFind", find),
    ("disjointNarrowKey", narrow_key),
    ("disjointNarrowKind", narrow_kind),
    ("disjointNarrowKinds", narrow_kinds),
    ("disjointNarrowFilterKind", narrow_filter_kind),
    ("disjointPage", page),
    ("disjointCounted", counted),
    ("disjointCursor", cursor),
    ("disjointInsert", insert),
    ("disjointUpdate", update),
    ("disjointChangeKind", change_kind),
    ("disjointInspect", inspect),
    ("disjointJob", job),
    ("disjointStatements", statements),
    ("disjointChange", evolve::change),
];

fn select_at(cx: &mut FunctionContext, index: usize) -> NeonResult<SelectStatement> {
    let value = cx.argument::<JsValue>(index)?;
    match node(cx, value) {
        Some(Node::Select(select)) => Ok(select),
        _ => cx.throw_type_error("expected a disjoint read's Select"),
    }
}

/// A read or write the shape built, as JavaScript holds it: its `Select`, or
/// the shape's refusal, raised before anything is sent, as a
/// `ConstructionError`.
fn built<'cx>(
    cx: &mut FunctionContext<'cx>,
    select: Result<SelectStatement, Error>,
) -> JsResult<'cx, JsValue> {
    match select {
        Ok(select) => Ok(cx.boxed(Node::Select(select)).upcast()),
        Err(error) => throw(cx, Failure::Construction(error.to_string())),
    }
}

fn strings<'cx>(
    cx: &mut Cx<'cx>,
    items: impl ExactSizeIterator<Item = String>,
) -> JsResult<'cx, JsValue> {
    let array = JsArray::new(cx, items.len());
    for (index, item) in items.enumerate() {
        let item = cx.string(item);
        array.set(cx, u32::try_from(index).unwrap_or(u32::MAX), item)?;
    }
    Ok(array.upcast())
}

fn string_list(cx: &mut FunctionContext, index: usize) -> NeonResult<Vec<String>> {
    let array = cx.argument::<JsArray>(index)?.to_vec(cx)?;
    array
        .into_iter()
        .map(|item| match item.downcast::<JsString, _>(cx) {
            Ok(item) => Ok(item.value(cx)),
            Err(_) => cx.throw_type_error("expected a list of strings"),
        })
        .collect()
}

/// What a write gives `table`'s columns: a model's `[column, Value]` pairs,
/// each `Set`, or a registration's `ActiveModel` of `entity`.
fn values_of<'cx>(
    cx: &mut FunctionContext<'cx>,
    table: &DisjointTable,
    entity: Option<&Entity>,
    given: Handle<'cx, JsValue>,
) -> NeonResult<DisjointValues> {
    let outcome = match entity {
        None => {
            let Ok(pairs) = given.downcast::<JsArray, _>(cx) else {
                return cx.throw_type_error("a model's values are [column, Value] pairs");
            };
            let codec = Codec::get(cx)?;
            let mut values = Vec::new();
            for pair in pairs.to_vec(cx)? {
                let Ok(pair) = pair.downcast::<JsArray, _>(cx) else {
                    return cx.throw_type_error("a model's values are [column, Value] pairs");
                };
                let column: Handle<JsString> = pair.get(cx, 0)?;
                let column = column.value(cx);
                let data: Handle<JsValue> = pair.get(cx, 1)?;
                let Some(value) = read::infer(cx, codec, data)? else {
                    return cx.throw_type_error("a model's values are Values");
                };
                values.push((column, ActiveValue::Set(value.datum)));
            }
            table.values(values)
        }
        Some(entity) => {
            let Ok(active) = given.downcast::<JsBox<ActiveHandle>, _>(cx) else {
                return cx.throw_type_error("a registered disjoint entity writes an ActiveModel");
            };
            let active = active.0.clone();
            if let Some(refusal) = foreign_active(entity.info(), active.info()) {
                return cx.throw_type_error(refusal);
            }
            return active_values(table, entity, &active).or_else(|failure| throw(cx, failure));
        }
    };
    outcome.or_else(|error| throw(cx, Failure::Construction(error.to_string())))
}

/// The values the base's columns `columns` are given, in order: a model's
/// already `Value`s, a registration's converted through each column's
/// declared kind.
fn base_values(
    cx: &mut FunctionContext,
    core: &DisjointCore,
    columns: &[String],
    index: usize,
) -> NeonResult<Vec<Value>> {
    let given = cx.argument::<JsArray>(index)?.to_vec(cx)?;
    if given.len() != columns.len() {
        let failure = Failure::Construction(format!(
            "{} value(s) given for {} column(s)",
            given.len(),
            columns.len()
        ));
        return throw(cx, failure);
    }
    let codec = Codec::get(cx)?;
    let mut values = Vec::with_capacity(given.len());
    for (column, data) in columns.iter().zip(given) {
        let value = match &core.codec {
            RowCodec::Models => match read::infer(cx, codec, data)? {
                Some(value) => value.datum,
                None => return cx.throw_type_error("a model's values are Values"),
            },
            RowCodec::Entities { base, .. } => {
                let info = match base.info().column(column) {
                    Ok(info) => info.clone(),
                    Err(failure) => return throw(cx, failure),
                };
                column_value(cx, &info, data)?
            }
        };
        values.push(value);
    }
    Ok(values)
}

/// The kind row's values: `None` for a kind without a table given none,
/// empty for one with a table given none.
fn kind_row<'cx>(
    cx: &mut FunctionContext<'cx>,
    core: &DisjointCore,
    kind: &str,
    index: usize,
) -> NeonResult<Option<DisjointValues>> {
    let given = cx.argument::<JsValue>(index)?;
    let given =
        (!given.is_a::<JsNull, _>(cx) && !given.is_a::<JsUndefined, _>(cx)).then_some(given);
    match core.kind_row(kind, given.is_some()) {
        Ok(KindRow::Absent) => Ok(None),
        Ok(KindRow::Empty(values)) => Ok(Some(values)),
        Ok(KindRow::Given { table, entity }) => match given {
            Some(given) => values_of(cx, table, entity, given).map(Some),
            None => Ok(None),
        },
        Err(failure) => throw(cx, failure),
    }
}

/// `disjointDescribe(disjoint)`: the model or registration as JSON.
// [spec:pgorm:req:bindings.disjoint+2]
fn describe(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    Ok(cx.string(core.description.to_string()).upcast())
}

/// `disjointKinds(disjoint)`: the kinds' values, in order.
fn kinds(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    strings(&mut cx, core.kinds().into_iter())
}

/// `disjointKeyColumns(disjoint)`: the base's key columns, in key order.
fn key_columns(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    strings(&mut cx, core.key_names().into_iter())
}

/// `disjointResultNames(disjoint, source)`: the names a read gives the
/// columns of the decoded source `source`, the base being 0.
fn result_names(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let source = cx.argument::<JsNumber>(1)?.value(&mut cx) as usize;
    let Some(names) = core.shape.result_names(source) else {
        return throw(
            &mut cx,
            Failure::Construction("no such decoded source".to_owned()),
        );
    };
    strings(&mut cx, names.into_iter())
}

/// `disjointFind(disjoint)`: the shape's read.
// [spec:pgorm:req:bindings.disjoint+2]
fn find(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    built(&mut cx, Ok(core.shape.find()))
}

/// `disjointNarrowKey(disjoint, select, key)`: the read of the base row of
/// the key given in key order.
fn narrow_key(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let mut select = select_at(&mut cx, 1)?;
    let key = base_values(&mut cx, &core, &core.key_names(), 2)?;
    let narrowed = core.shape.key_is(key).map(|condition| {
        select.cond_where(condition);
        select
    });
    built(&mut cx, narrowed)
}

/// `disjointNarrowKind(disjoint, select, kind)`: `select` narrowed to base
/// rows of the kind `kind`.
fn narrow_kind(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let mut select = select_at(&mut cx, 1)?;
    let kind = cx.argument::<JsString>(2)?.value(&mut cx);
    let narrowed = core.shape.kind_is(&kind).map(|condition| {
        select.and_where(condition);
        select
    });
    built(&mut cx, narrowed)
}

/// `disjointNarrowKinds(disjoint, select, kinds)`: `select` narrowed to base
/// rows of one of `kinds`.
fn narrow_kinds(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let mut select = select_at(&mut cx, 1)?;
    let kinds = string_list(&mut cx, 2)?;
    let narrowed = core.shape.kind_in(&kinds).map(|condition| {
        select.and_where(condition);
        select
    });
    built(&mut cx, narrowed)
}

/// `disjointNarrowFilterKind(disjoint, select, kind, predicate)`: `select`
/// narrowed to base rows of the kind `kind` whose row passes `predicate`.
fn narrow_filter_kind(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let mut select = select_at(&mut cx, 1)?;
    let kind = cx.argument::<JsString>(2)?.value(&mut cx);
    let value = cx.argument::<JsValue>(3)?;
    let condition = predicate(&mut cx, value)?.into_condition();
    let narrowed = core.shape.filter_kind(&kind, condition).map(|condition| {
        select.cond_where(condition);
        select
    });
    built(&mut cx, narrowed)
}

/// `disjointPage(disjoint, select, size, page)`: page `page`, as pgorm's
/// paginator reads it.
fn page(mut cx: FunctionContext) -> JsResult<JsValue> {
    let select = select_at(&mut cx, 1)?;
    let size = cx.argument::<JsNumber>(2)?.value(&mut cx) as u64;
    let page = cx.argument::<JsNumber>(3)?.value(&mut cx) as u64;
    let Some(size) = NonZeroU64::new(size) else {
        return throw(
            &mut cx,
            Failure::Construction("a page holds a row".to_owned()),
        );
    };
    built(&mut cx, pgorm::page_statement(&select, size, page))
}

/// `disjointCounted(disjoint, select)`: the statement pgorm's paginator
/// counts the read's rows with.
fn counted(mut cx: FunctionContext) -> JsResult<JsValue> {
    let select = select_at(&mut cx, 1)?;
    built(&mut cx, pgorm::count_statement(&select))
}

/// `disjointCursor(disjoint, select, columns, after, before, window,
/// descending)`: `[statement, reversed]`, the statement pgorm's own `Cursor`
/// reads its window with, each boundary value under its column's casts, and
/// whether its rows come reversed.
fn cursor(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let select = select_at(&mut cx, 1)?;
    let columns = string_list(&mut cx, 2)?;
    let mut cursor = match core.shape.cursor_by(select, &columns) {
        Ok(cursor) => cursor,
        Err(error) => return throw(&mut cx, Failure::Construction(error.to_string())),
    };
    for (index, after) in [(3, true), (4, false)] {
        let given = cx.argument::<JsValue>(index)?;
        if given.is_a::<JsNull, _>(&mut cx) {
            continue;
        }
        let values = ValueTuple::from(base_values(&mut cx, &core, &columns, index)?);
        if after {
            cursor.after_with(values);
        } else {
            cursor.before_with(values);
        }
    }
    let window = cx.argument::<JsValue>(5)?;
    if let Ok(window) = window.downcast::<JsArray, _>(&mut cx) {
        let last: Handle<JsBoolean> = window.get(&mut cx, 0)?;
        let last = last.value(&mut cx);
        let rows: Handle<JsNumber> = window.get(&mut cx, 1)?;
        let rows = rows.value(&mut cx) as u64;
        if last {
            cursor.last(rows);
        } else {
            cursor.first(rows);
        }
    }
    if cx.argument::<JsBoolean>(6)?.value(&mut cx) {
        cursor.desc();
    }
    let statement = built(&mut cx, cursor.statement())?;
    let pair = JsArray::new(&mut cx, 2);
    pair.set(&mut cx, 0, statement)?;
    let reversed = cx.boolean(cursor.reversed());
    pair.set(&mut cx, 1, reversed)?;
    Ok(pair.upcast())
}

/// The base's values and the kind row's for the write at arguments 1–3.
fn written(
    cx: &mut FunctionContext,
    core: &DisjointCore,
) -> NeonResult<(DisjointValues, String, Option<DisjointValues>)> {
    let kind = cx.argument::<JsString>(2)?.value(cx);
    let position = match core.kind_index(&kind) {
        Ok(position) => position,
        Err(failure) => return throw(cx, failure),
    };
    let (base_entity, _) = core.entities(position);
    let given = cx.argument::<JsValue>(1)?;
    let base = values_of(cx, core.shape.base(), base_entity, given)?;
    let row = kind_row(cx, core, &kind, 3)?;
    Ok((base, kind, row))
}

/// `disjointInsert(disjoint, base, kind, row)`: the statement inserting a
/// value of the kind `kind`.
// [spec:pgorm:req:bindings.disjoint+2]
fn insert(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let (base, kind, row) = written(&mut cx, &core)?;
    built(&mut cx, core.shape.insert(&base, &kind, row.as_ref()))
}

/// `disjointUpdate(disjoint, base, kind, row)`: the statement updating a
/// value of the kind `kind`, found by the key `base` holds.
// [spec:pgorm:req:bindings.disjoint+2]
fn update(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let (base, kind, row) = written(&mut cx, &core)?;
    built(&mut cx, core.shape.update(&base, &kind, row.as_ref()))
}

/// `disjointChangeKind(disjoint, key, kind, row)`: the statement changing
/// the kind of the row whose key `key` gives in key order.
// [spec:pgorm:req:bindings.disjoint+2]
fn change_kind(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let key = base_values(&mut cx, &core, &core.key_names(), 1)?;
    let kind = cx.argument::<JsString>(2)?.value(&mut cx);
    let row = kind_row(&mut cx, &core, &kind, 3)?;
    built(&mut cx, core.shape.change_kind(key, &kind, row.as_ref()))
}

/// Which terminal a job runs.
#[derive(Clone, Copy)]
enum Terminal {
    All,
    One,
    Write,
    Count,
}

fn terminal(
    cx: &mut FunctionContext,
    index: usize,
    query: &mut SelectStatement,
) -> NeonResult<Terminal> {
    let name = cx.argument::<JsString>(index)?.value(cx);
    Ok(match name.as_str() {
        "all" => Terminal::All,
        "one" => {
            query.narrow_limit_to_one();
            Terminal::One
        }
        "write" => Terminal::Write,
        "count" => Terminal::Count,
        _ => return cx.throw_type_error("unknown disjoint terminal"),
    })
}

/// `disjointInspect(disjoint, select, terminal)`: `[sql, values]` the
/// terminal sends.
fn inspect(mut cx: FunctionContext) -> JsResult<JsValue> {
    let mut select = select_at(&mut cx, 1)?;
    terminal(&mut cx, 2, &mut select)?;
    inspected(&mut cx, select.build())
}

/// `disjointJob(disjoint, select, terminal)`: the job running a read —
/// every row, the first of a read narrowed to one, the one row a write reads
/// back, or a count. No row for `"one"` or `"write"` is pgorm's
/// `RecordNotFound`, a `DecodeError`.
// [spec:pgorm:req:bindings.disjoint+2]
// [spec:pgorm:req:bindings.registered-pools]
fn job(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let mut query = select_at(&mut cx, 1)?;
    let terminal = terminal(&mut cx, 2, &mut query)?;
    let (sql, values) = query.build();
    let job: crate::connect::job::Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            use pgorm::ConnectionTrait;
            let holders: Vec<pgorm::ValueHolder> =
                values.0.into_iter().map(pgorm::ValueHolder).collect();
            let params: Vec<&(dyn pgorm::types::ToSql + Sync)> = holders
                .iter()
                .map(|holder| holder as &(dyn pgorm::types::ToSql + Sync))
                .collect();
            let rows = db
                .query_all(&sql, &params)
                .await
                .map_err(|error| failure(&error, secrets))?;
            match terminal {
                Terminal::Count => {
                    let count = rows
                        .first()
                        .ok_or_else(|| Failure::Internal("a count reads one row".to_owned()))?
                        .try_get::<_, i64>("num_items")
                        .map_err(|error| Failure::Decode(error.to_string()))?;
                    Ok(Box::new(rows::Count(count)) as Output)
                }
                Terminal::All => Ok(Box::new(rows::Rows::of(&core, rows)?) as Output),
                Terminal::One | Terminal::Write => {
                    let row = rows
                        .into_iter()
                        .next()
                        .ok_or_else(|| Failure::Decode(Error::RecordNotFound.to_string()))?;
                    Ok(Box::new(rows::Rows::of(&core, vec![row])?) as Output)
                }
            }
        })
    });
    Ok(cx.boxed(JobHandle::new(job)).upcast())
}

/// `disjointStatements(disjoint)`: `[kind, statement]` for each statement
/// creating the kind type, the tables and their constraints, in the order
/// they run.
// [spec:pgorm:req:bindings.disjoint+2]
fn statements(mut cx: FunctionContext) -> JsResult<JsValue> {
    let core = cx.argument::<JsBox<DisjointHandle>>(0)?.0.clone();
    let statements = match core.shape.create() {
        Ok(statements) => statements,
        Err(error) => return throw(&mut cx, Failure::Construction(error.to_string())),
    };
    Ok(evolve::ddl_pairs(&mut cx, statements)?.upcast())
}
