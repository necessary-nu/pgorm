//! The columns of a COPY: described by the server through pgorm's
//! `CopyTrait::copy_in_columns` and `copy_out_columns` without starting it,
//! on a pool's connection, a connection or a transaction, or named by a
//! program for a COPY it frames itself; and what a COPY of rows' sink and
//! stream tell JavaScript about theirs.

use neon::{prelude::*, types::Finalize};
use pgorm::{CopyColumn, CopyTrait};

use super::{
    super::{
        ConnectionHandle, Held, Operation, PoolHandle, abort_argument, config::DIALECT,
        on_connection, on_pool,
    },
    in_state, out_state, rows,
};
use crate::{
    codec::Codec,
    decode,
    errors::{Failure, Redactions, failure, throw},
    settle,
    statements::{CopyStatement, copy_at},
    values::{
        TypeName,
        read::{named_type, refuse},
    },
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("copyColumnNew", copy_column_new)?;
    cx.export_function("copyColumnParts", copy_column_parts)?;
    cx.export_function("poolCopyColumns", pool_copy_columns)?;
    cx.export_function("connectionCopyColumns", connection_copy_columns)?;
    cx.export_function("copySinkColumns", copy_sink_columns)?;
    cx.export_function("copyInClosed", copy_in_closed)?;
    cx.export_function("copyRowsKinds", copy_rows_kinds)?;
    Ok(())
}

/// The JavaScript `CopyColumn`'s native half.
#[derive(Debug)]
pub(crate) struct ColumnHandle(CopyColumn);

impl Finalize for ColumnHandle {}

/// `copyColumnNew(name, type, moneyPlaces)`: a column of a built-in type,
/// named as `pg_type` names it or as an element's name followed by `[]`, or
/// by a `TypeName` in `pg_catalog`. Any other type, `money` without its
/// places and places for any other type are a `ConstructionError`.
// [spec:pgorm:req:bindings.copy-rows+2]
fn copy_column_new(mut cx: FunctionContext) -> JsResult<JsBox<ColumnHandle>> {
    let name = cx.argument::<JsString>(0)?.value(&mut cx);
    let given = cx.argument::<JsValue>(1)?;
    let type_name = match given.downcast::<JsString, _>(&mut cx) {
        Ok(text) => text.value(&mut cx),
        Err(_) => {
            let codec = Codec::get(&mut cx)?;
            match named_type(&mut cx, codec, given)? {
                Some(TypeName { name, schema }) => match schema.as_deref() {
                    None | Some("pg_catalog") => name,
                    Some(schema) => format!("{schema}.{name}"),
                },
                None => return cx.throw_type_error("a COPY column's type is a name or a TypeName"),
            }
        }
    };
    let places = match cx.argument_opt(2) {
        Some(places) if places.is_a::<JsNumber, _>(&mut cx) => {
            let places = places
                .downcast_or_throw::<JsNumber, _>(&mut cx)?
                .value(&mut cx);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Some(places as u32)
        }
        _ => None,
    };
    match pgorm_bindings::copy_column(name, &type_name, places, DIALECT) {
        Ok(column) => Ok(cx.boxed(ColumnHandle(column))),
        Err(refusal) => refuse(&mut cx, refusal.into_message()),
    }
}

/// `copyColumnParts(column)`: `[name, type, moneyPlaces]`, the type a
/// `TypeName` qualified by the schema the server names it in, and the places
/// `null` for any column but a `money` one.
// [spec:pgorm:req:bindings.copy-rows+2]
fn copy_column_parts(mut cx: FunctionContext) -> JsResult<JsArray> {
    let column = cx.argument::<JsBox<ColumnHandle>>(0)?.0.clone();
    let codec = Codec::get(&mut cx)?;
    let ty = column.type_();
    let schema = cx.string(ty.schema());
    let type_name = codec.make(&mut cx, ("typename", ty.name().to_owned(), schema))?;
    let name = cx.string(column.name());
    let places: Handle<JsValue> = match column.money_places() {
        Some(places) => cx.number(places).upcast(),
        None => cx.null().upcast(),
    };
    let parts = JsArray::new(&mut cx, 3);
    parts.set(&mut cx, 0, name)?;
    parts.set(&mut cx, 1, type_name)?;
    parts.set(&mut cx, 2, places)?;
    Ok(parts)
}

/// Each of `columns` as the handle a JavaScript `CopyColumn` holds.
pub(crate) fn columns_to_js<'cx, C>(cx: &mut Cx<'cx>, columns: C) -> JsResult<'cx, JsValue>
where
    C: AsRef<[CopyColumn]>,
{
    let columns = columns.as_ref();
    let array = JsArray::new(cx, columns.len());
    for (at, column) in columns.iter().enumerate() {
        let handle = cx.boxed(ColumnHandle(column.clone()));
        array.set(cx, rows::index(at), handle)?;
    }
    Ok(array.upcast())
}

/// The columns at `index`, each a handle a program's or a description's
/// `CopyColumn` holds; anything else in the array is a `TypeError`.
pub(super) fn columns_from(cx: &mut FunctionContext, index: usize) -> NeonResult<Vec<CopyColumn>> {
    let handles = cx.argument::<JsArray>(index)?.to_vec(cx)?;
    handles
        .into_iter()
        .map(|handle| {
            Ok(handle
                .downcast_or_throw::<JsBox<ColumnHandle>, _>(cx)?
                .0
                .clone())
        })
        .collect()
}

/// The columns `copy` would carry, in its order, as the server describes
/// them, running nothing. A refusal is the server's, and leaves `db` as a
/// failed statement does.
// [spec:pgorm:req:bindings.copy-rows+2]
pub(crate) async fn described<C>(
    db: &mut C,
    copy: &CopyStatement,
    secrets: &Redactions,
) -> Result<Vec<CopyColumn>, Failure>
where
    C: CopyTrait + Send,
{
    let columns = match copy {
        CopyStatement::CopyFrom(copy) => rows::describe_in(db, copy).await,
        CopyStatement::CopyTo(copy) => rows::describe_out(db, copy).await,
    };
    columns.map_err(|error| failure(&error, secrets))
}

/// The columns `copy` would carry, described on a connection a pool lent,
/// plain or instrumented.
async fn described_on(
    held: &mut Held,
    copy: &CopyStatement,
    secrets: &Redactions,
) -> Result<Vec<CopyColumn>, Failure> {
    match held {
        Held::Plain(db) => described(db, copy, secrets).await,
        Held::Metered(db) => described(db, copy, secrets).await,
    }
}

/// `poolCopyColumns(pool, copy, abort)`: the description on a connection the
/// pool lends for it alone.
// [spec:pgorm:req:bindings.copy-rows+2]
fn pool_copy_columns(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let pool = cx.argument::<JsBox<PoolHandle>>(0)?.0.clone();
    let copy = copy_at(&mut cx, 1)?;
    let abort = abort_argument(&mut cx, 2)?;
    let work = async move {
        on_pool(&pool, &abort, async |db| {
            described_on(db, &copy, &pool.secrets).await
        })
        .await
    };
    settle::promise(&mut cx, work, columns_to_js)
}

/// `connectionCopyColumns(connection, copy, abort)`: the description on the
/// connection, which it holds as a statement does.
// [spec:pgorm:req:bindings.copy-rows+2]
fn connection_copy_columns(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let connection = cx.argument::<JsBox<ConnectionHandle>>(0)?.0.clone();
    let copy = copy_at(&mut cx, 1)?;
    let abort = abort_argument(&mut cx, 2)?;
    let secrets = connection.pool.secrets.clone();
    let operation = match Operation::begin(connection) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move {
        on_connection(operation, &abort, async |db| {
            described_on(db, &copy, &secrets).await
        })
        .await
    };
    settle::promise(&mut cx, work, columns_to_js)
}

/// `copySinkColumns(copyIn)`: the columns a COPY of rows' sink takes, as its
/// session described them before the COPY started.
// [spec:pgorm:req:bindings.copy-rows+2]
fn copy_sink_columns(mut cx: FunctionContext) -> JsResult<JsValue> {
    let state = in_state(&mut cx)?;
    let Some(encoder) = state.rows.as_ref().and_then(|rows| rows.encoder.get()) else {
        return cx.throw_type_error("copySinkColumns reads the columns of a COPY of rows");
    };
    columns_to_js(&mut cx, encoder.columns())
}

/// `copyInClosed(copyIn)`: whether the COPY has ended, abandoned or cut off.
fn copy_in_closed(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let state = in_state(&mut cx)?;
    let closed = state.abandon.is_cancelled() || state.ending.released.is_cancelled();
    Ok(cx.boolean(closed))
}

/// `copyRowsKinds(stream)`: each column's kind as a model's declaration is
/// keyed, for a model to hold its fields to as it reads a row.
// [spec:pgorm:req:bindings.copy-rows+2]
// [spec:pgorm:req:bindings.model-records+1]
fn copy_rows_kinds(mut cx: FunctionContext) -> JsResult<JsArray> {
    let state = out_state(&mut cx)?;
    let columns: Vec<CopyColumn> = match &state.rows {
        Some(rows) => rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|decoder| decoder.columns().to_vec())
            .unwrap_or_default(),
        None => return cx.throw_type_error("copyRowsKinds keys the columns of copyOutRows"),
    };
    let keys = columns
        .iter()
        .map(|column| decode::field(column, None).map(|null| null.tag.key()))
        .collect::<Result<Vec<_>, _>>();
    let keys = match keys {
        Ok(keys) => keys,
        Err(failure) => return throw(&mut cx, failure),
    };
    let array = JsArray::new(&mut cx, keys.len());
    for (at, key) in keys.iter().enumerate() {
        let key = cx.string(key);
        array.set(&mut cx, rows::index(at), key)?;
    }
    Ok(array)
}
