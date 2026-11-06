//! A COPY of rows: the binary COPY whose session describes its columns
//! before it starts, whose rows JavaScript hands over as arrays to be encoded
//! against those columns' types, and whose data is decoded back into rows as
//! it arrives.

use std::sync::{
    OnceLock, PoisonError,
    atomic::{AtomicBool, Ordering},
};

use bytes::BytesMut;
use neon::{prelude::*, types::JsUint8Array};
use pgorm::{CopyColumn, CopyRowDecoder, CopyRowEncoder, CopyTrait};
use tokio::sync::oneshot;

use super::{OutState, Pull, abort_argument, in_state, out_state};
use crate::{
    codec::Codec,
    decode,
    errors::{Failure, throw},
    params, settle,
    statements::{CopyFrom, CopyTo, copy_from_at, copy_to_at},
    values::{Tagged, read::refuse, write},
};

/// A COPY of rows' writing side: the encoder over the columns its session
/// describes before the COPY starts, and whether the format's header has
/// gone out.
#[derive(Debug, Default)]
pub(crate) struct RowsIn {
    pub(super) encoder: OnceLock<CopyRowEncoder>,
    headed: AtomicBool,
}

impl RowsIn {
    /// The header, the first time it is asked for, into `out`.
    pub(super) fn head(&self, out: &mut BytesMut) {
        if !self.headed.swap(true, Ordering::AcqRel) {
            CopyRowEncoder::header(out);
        }
    }
}

pub(super) async fn describe_in<C>(
    db: &mut C,
    copy: &CopyFrom,
) -> Result<Vec<CopyColumn>, pgorm::Error>
where
    C: CopyTrait + Send,
{
    match copy {
        CopyFrom::Text(statement) => db.copy_in_columns(statement).await,
        CopyFrom::Csv(statement) => db.copy_in_columns(statement).await,
        CopyFrom::Binary(statement) => db.copy_in_columns(statement).await,
    }
}

/// Whether `statement` may run as a COPY of rows: the binary format is the
/// one whose rows pgorm writes and reads as values.
fn rows_format<'cx>(cx: &mut FunctionContext<'cx>, binary: bool, rows: bool) -> NeonResult<()> {
    if rows && !binary {
        return cx.throw_type_error(
            "rows travel in binary COPY alone: build the statement with the \"binary\" format",
        );
    }
    Ok(())
}

/// The optional `rows` flag at `index`: whether the COPY's data is rows of
/// values rather than bytes.
pub(crate) fn rows_argument(cx: &mut FunctionContext, index: usize) -> NeonResult<bool> {
    match cx.argument_opt(index) {
        Some(value) if value.is_a::<JsBoolean, _>(cx) => {
            Ok(value.downcast_or_throw::<JsBoolean, _>(cx)?.value(cx))
        }
        _ => Ok(false),
    }
}

/// The COPY FROM at `index` and the `rows` flag at `rows_index`, a COPY of
/// rows refused unless it is binary.
pub(crate) fn copy_from_rows_at(
    cx: &mut FunctionContext,
    index: usize,
    rows_index: usize,
) -> NeonResult<(CopyFrom, bool)> {
    let copy = copy_from_at(cx, index)?;
    let rows = rows_argument(cx, rows_index)?;
    rows_format(cx, matches!(copy, CopyFrom::Binary(_)), rows)?;
    Ok((copy, rows))
}

/// The COPY TO at `index` and the `rows` flag at `rows_index`, as
/// [`copy_from_rows_at`] reads them.
pub(crate) fn copy_to_rows_at(
    cx: &mut FunctionContext,
    index: usize,
    rows_index: usize,
) -> NeonResult<(CopyTo, bool)> {
    let copy = copy_to_at(cx, index)?;
    let rows = rows_argument(cx, rows_index)?;
    rows_format(cx, matches!(copy, CopyTo::Binary(_)), rows)?;
    Ok((copy, rows))
}

pub(super) async fn describe_out<C>(
    db: &mut C,
    copy: &CopyTo,
) -> Result<Vec<CopyColumn>, pgorm::Error>
where
    C: CopyTrait + Send,
{
    match copy {
        CopyTo::Text(statement) => db.copy_out_columns(statement).await,
        CopyTo::Csv(statement) => db.copy_out_columns(statement).await,
        CopyTo::Binary(statement) => db.copy_out_columns(statement).await,
    }
}

/// The message a refusal of pgorm's carries, without the variant's prefix.
pub(super) fn refusal(error: pgorm::Error) -> String {
    match error {
        pgorm::Error::Type(message) => message,
        other => other.to_string(),
    }
}

/// `copyRowsEncode(copyIn, rows)`: `rows`, each an array of its columns'
/// values, in the binary format as a `Uint8Array` for `copyInWrite` — the
/// header before the first. Each value is read as a parameter is, and
/// written against its column's type as a parameter is bound; a row of
/// another width, or a value its column's type does not take, is refused with
/// a `ConstructionError` naming the column.
// [spec:pgorm:req:bindings.copy-rows+2]
pub(super) fn copy_rows_encode(mut cx: FunctionContext) -> JsResult<JsUint8Array> {
    let state = in_state(&mut cx)?;
    let Some(rows) = &state.rows else {
        return cx.throw_type_error("copyRowsEncode writes the rows of copyInRows");
    };
    let Some(encoder) = rows.encoder.get() else {
        return throw(&mut cx, state.ending.failure());
    };
    let batch = cx.argument::<JsArray>(1)?.to_vec(&mut cx)?;
    let codec = Codec::get(&mut cx)?;
    let mut body = BytesMut::new();
    for row in batch {
        encode_row(&mut cx, codec, encoder, row, &mut body)?;
    }
    let mut out = BytesMut::new();
    rows.head(&mut out);
    out.extend_from_slice(&body);
    JsUint8Array::from_slice(&mut cx, &out)
}

/// Append `row`, an array of its columns' values, to `out` in the binary
/// format: each value read as a parameter is, and written against its
/// column's type as a parameter is bound. A row of another width, or a value
/// its column's type does not take, is refused with a `ConstructionError`
/// naming the column, and `out` is left as it was.
// [spec:pgorm:req:bindings.copy-rows+2]
pub(super) fn encode_row<'cx>(
    cx: &mut FunctionContext<'cx>,
    codec: Codec<'cx>,
    encoder: &CopyRowEncoder,
    row: Handle<'cx, JsValue>,
    out: &mut BytesMut,
) -> NeonResult<()> {
    let Ok(row) = row.downcast::<JsArray, _>(cx) else {
        return cx.throw_type_error("a COPY row is an array of its columns' values");
    };
    let params = params::read(cx, codec, row)?;
    match encoder.encode(&params, out) {
        Ok(()) => Ok(()),
        Err(error) => refuse(cx, refusal(error)),
    }
}

/// The data a decoder was given refused as not the binary format, or as
/// ending where it may not.
/// The decoder's next whole row, each field decoded as a result column of
/// its type is; `None` until more data arrives, and after the trailer.
// [spec:pgorm:req:bindings.copy-rows+2]
pub(super) fn next_decoded(decoder: &mut CopyRowDecoder) -> Result<Option<Vec<Tagged>>, Failure> {
    let Some(fields) = decoder
        .next_fields()
        .map_err(|error| Failure::Decode(refusal(error)))?
    else {
        return Ok(None);
    };
    fields
        .iter()
        .zip(decoder.columns())
        .map(|(raw, column)| decode::field(column, raw.as_deref()))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// Every whole row the decoder holds.
fn decoded(decoder: &mut CopyRowDecoder) -> Result<Vec<Vec<Tagged>>, Failure> {
    let mut rows = Vec::new();
    while let Some(row) = next_decoded(decoder)? {
        rows.push(row);
    }
    Ok(rows)
}

/// The rows the decoder of `state` yields once `chunk` is added, or the data's
/// end checked when there is no chunk.
fn rows_from(state: &OutState, chunk: Option<&[u8]>) -> Result<Vec<Vec<Tagged>>, Failure> {
    let Some(rows) = &state.rows else {
        return Err(Failure::Lifecycle("not a COPY of rows".to_owned()));
    };
    let mut decoder = rows.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(decoder) = decoder.as_mut() else {
        return Err(Failure::Lifecycle("the COPY has not started".to_owned()));
    };
    match chunk {
        Some(chunk) => decoder.push(chunk),
        None => decoder
            .finish()
            .map_err(|error| Failure::Decode(refusal(error)))?,
    }
    decoded(decoder)
}

/// `copyRowsNext(stream, abort)`: the rows that have arrived, at least one,
/// each an array of its values, or `null` at the end, which resolves once
/// the connection is free again. One pull runs at a time. A field that does
/// not decode rejects with a `DecodeError` and closes the stream, discarding
/// its connection, which the server would go on sending to.
// [spec:pgorm:req:bindings.copy-rows+2]
pub(super) fn copy_rows_next(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = out_state(&mut cx)?;
    let abort = abort_argument(&mut cx, 1)?;
    if state.rows.is_none() {
        return cx.throw_type_error("copyRowsNext reads the rows of copyOutRows");
    }
    if state.finished.load(Ordering::Acquire) {
        let work = async move {
            state.ending.released().await;
            Ok(None)
        };
        return settle::promise(&mut cx, work, rows_to_js);
    }
    if state.close.is_cancelled() {
        let closed = Failure::Lifecycle("the COPY stream is closed".to_owned());
        return settle::rejected(&mut cx, closed);
    }
    let Ok(busy) = state.busy.clone().try_lock_owned() else {
        let busy =
            Failure::Lifecycle("the COPY stream is busy: one pull runs at a time".to_owned());
        return settle::rejected(&mut cx, busy);
    };
    let work = async move {
        let _busy = busy;
        loop {
            let (reply, receive) = oneshot::channel();
            let sent = state
                .pulls
                .send(Pull {
                    abort: abort.clone(),
                    reply,
                })
                .is_ok();
            let outcome = if sent { receive.await.ok() } else { None };
            let chunk = match outcome {
                Some(Ok(chunk)) => chunk,
                Some(Err(failure)) => {
                    state.ending.released().await;
                    return Err(failure);
                }
                None => {
                    state.ending.released().await;
                    return Err(state.ending.failure());
                }
            };
            let ended = chunk.is_none();
            match rows_from(&state, chunk.as_deref()) {
                Ok(rows) if ended => {
                    state.ending.released().await;
                    return Ok((!rows.is_empty()).then_some(rows));
                }
                Ok(rows) if !rows.is_empty() => return Ok(Some(rows)),
                Ok(_) => {}
                Err(failure) => {
                    state.close.cancel();
                    state.ending.released().await;
                    return Err(failure);
                }
            }
        }
    };
    settle::promise(&mut cx, work, rows_to_js)
}

fn rows_to_js<'cx>(cx: &mut Cx<'cx>, rows: Option<Vec<Vec<Tagged>>>) -> JsResult<'cx, JsValue> {
    let Some(rows) = rows else {
        return Ok(cx.null().upcast());
    };
    let codec = Codec::get(cx)?;
    let array = JsArray::new(cx, rows.len());
    for (at, row) in rows.iter().enumerate() {
        let values = row_to_js(cx, codec, row)?;
        array.set(cx, index(at), values)?;
    }
    Ok(array.upcast())
}

/// One row as an array of its plain values.
pub(super) fn row_to_js<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    row: &[Tagged],
) -> JsResult<'cx, JsArray> {
    let values = JsArray::new(cx, row.len());
    for (column, value) in row.iter().enumerate() {
        let value = write::plain(cx, codec, value)?;
        values.set(cx, index(column), value)?;
    }
    Ok(values)
}

pub(super) fn index(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// `copyRowsColumns(stream)`: the names of the columns each row holds, in
/// order, as the server describes them.
// [spec:pgorm:req:bindings.copy-rows+2]
pub(super) fn copy_rows_columns(mut cx: FunctionContext) -> JsResult<JsArray> {
    let state = out_state(&mut cx)?;
    let names: Vec<String> = match &state.rows {
        Some(rows) => rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|decoder| {
                decoder
                    .columns()
                    .iter()
                    .map(|column| column.name().to_owned())
                    .collect()
            })
            .unwrap_or_default(),
        None => return cx.throw_type_error("copyRowsColumns names the columns of copyOutRows"),
    };
    let array = JsArray::new(&mut cx, names.len());
    for (at, name) in names.iter().enumerate() {
        let name = cx.string(name);
        array.set(&mut cx, index(at), name)?;
    }
    Ok(array)
}
