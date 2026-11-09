//! Row streams: pgorm's `query_raw` row stream, pulled a row at a time, so
//! the driver's bounded buffer and the socket hold back the server while
//! JavaScript is not asking for rows.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use neon::{prelude::*, types::Finalize};
use pgorm_bindings::StreamState;

use super::{Abort, ConnectionHandle, Operation, abort_argument};
use crate::{codec::Codec, decode, errors::Failure, rows, settle, values::Tagged};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("connectionStream", connection_stream)?;
    cx.export_function("streamNext", stream_next)?;
    cx.export_function("streamClose", stream_close)?;
    cx.export_function("streamClosed", stream_closed)?;
    Ok(())
}

/// What a stream that decodes its own rows makes of each: a disjoint read's
/// codec's rows, or a registered read's models.
pub(crate) type RowDecode =
    Arc<dyn Fn(tokio_postgres::Row) -> Result<crate::connect::job::Output, Failure> + Send + Sync>;

/// A stream as JavaScript reads it: pgorm-bindings' stream, which holds its
/// connection until the last row or `close`, and what each row becomes.
pub(crate) struct Stream {
    state: Arc<StreamState>,
    /// The decode of each row, for a disjoint read's or a registered read's
    /// stream; its rows are otherwise values.
    decode: Option<RowDecode>,
    /// Whether the column names went with a row already.
    named: AtomicBool,
}

impl fmt::Debug for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stream")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl Stream {
    pub(crate) fn new(state: Arc<StreamState>, decode: Option<RowDecode>) -> Arc<Self> {
        Arc::new(Self {
            state,
            decode,
            named: AtomicBool::new(false),
        })
    }
}

/// The JavaScript `RowStream`'s native half. Collected unfinished, it
/// releases its connection as a backstop; reaching the end, `return` and
/// `close` are what release it.
#[derive(Debug)]
pub(crate) struct StreamHandle(pub(crate) Arc<Stream>);

impl Finalize for StreamHandle {}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.0.state.abandon();
    }
}

/// The decode the stream's argument `index` names: a disjoint read's codec,
/// a registered read's selector, or with neither its rows as values.
pub(crate) fn decode_argument(cx: &mut FunctionContext, index: usize) -> Option<RowDecode> {
    let value = cx.argument_opt(index)?;
    if let Ok(handle) = value.downcast::<JsBox<crate::disjoint::DisjointHandle>, _>(cx) {
        let core = handle.0.clone();
        let decode: RowDecode = Arc::new(move |row| {
            let rows = crate::disjoint::Rows::of(&core, vec![row])?;
            Ok(Box::new(rows) as crate::connect::job::Output)
        });
        return Some(decode);
    }
    value
        .downcast::<JsBox<crate::entities::reads::DecoderHandle>, _>(cx)
        .ok()
        .map(|handle| handle.row_decode())
}

/// `connectionStream(connection, sql, params, abort, decoder?)`: open a
/// stream, which holds the connection until the last row or `close`, each
/// row read by `decoder` when one is given, a disjoint read's codec or a
/// registered read's selector.
// [spec:pgorm:req:bindings.streams+1]
// [spec:pgorm:req:bindings.disjoint+2]
fn connection_stream(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let connection = cx.argument::<JsBox<ConnectionHandle>>(0)?.0.clone();
    let (sql, params) = super::sql_and_params(&mut cx, 1)?;
    let abort = abort_argument(&mut cx, 3)?;
    let decode = decode_argument(&mut cx, 4);
    let operation = match Operation::begin(connection) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move {
        let state = StreamState::on_connection(operation, sql, params, abort.token()).await?;
        Ok(Stream::new(state, decode))
    };
    settle::promise(&mut cx, work, |cx, stream| {
        Ok(cx.boxed(StreamHandle(stream)).upcast())
    })
}

/// One pulled row: its values, or what the stream's decode read it as.
enum Pulled {
    Values(Vec<Tagged>),
    Decoded(crate::connect::job::Output),
}

/// The next row, decoded, with the column names on the first; `None` at the
/// end, which returns the connection to its slot.
async fn next(
    stream: Arc<Stream>,
    abort: Abort,
) -> Result<Option<(Pulled, Option<Vec<String>>)>, Failure> {
    let pulled = stream
        .state
        .next(abort.token(), |row| {
            if let Some(decode) = &stream.decode {
                return decode(row).map(|decoded| (Pulled::Decoded(decoded), None));
            }
            let names = if stream.named.swap(true, Ordering::Relaxed) {
                None
            } else {
                Some(decode::names(&row)?)
            };
            Ok((Pulled::Values(decode::row(&row)?), names))
        })
        .await?;
    pulled.transpose()
}

/// `streamNext(stream, tagged, abort)`: `[values, names?, kinds?]`, the
/// names and kinds on the first row, or `null` at the end. A failure, an abort or a decode error discards the connection.
// [spec:pgorm:req:bindings.streams+1]
fn stream_next(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = cx.argument::<JsBox<StreamHandle>>(0)?.0.clone();
    let tagged = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    let abort = abort_argument(&mut cx, 2)?;
    settle::promise(&mut cx, next(state, abort), move |cx, item| {
        let Some((pulled, names)) = item else {
            return Ok(cx.null().upcast());
        };
        let values = match pulled {
            Pulled::Values(values) => values,
            Pulled::Decoded(decoded) => {
                let pair = JsArray::new(cx, 1);
                let rows = decoded.into_js(cx)?;
                pair.set(cx, 0, rows)?;
                return Ok(pair.upcast());
            }
        };
        let codec = Codec::get(cx)?;
        let pair = JsArray::new(cx, 3);
        if let Some(names) = names {
            let list = JsArray::new(cx, names.len());
            for (index, name) in names.iter().enumerate() {
                let name = cx.string(name);
                list.set(cx, u32::try_from(index).unwrap_or(u32::MAX), name)?;
            }
            pair.set(cx, 1, list)?;
            let kinds = rows::kinds(cx, Some(&values))?;
            pair.set(cx, 2, kinds)?;
        }
        let array = JsArray::new(cx, values.len());
        for (index, value) in values.into_iter().enumerate() {
            let value = rows::value(cx, codec, value, tagged)?;
            array.set(cx, u32::try_from(index).unwrap_or(u32::MAX), value)?;
        }
        pair.set(cx, 0, array)?;
        Ok(pair.upcast())
    })
}

/// `streamClose(stream)`: end the stream, discarding its connection if rows
/// remained, and resolve once it is released.
// [spec:pgorm:req:bindings.streams+1]
fn stream_close(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let stream = cx.argument::<JsBox<StreamHandle>>(0)?.0.clone();
    let work = async move {
        stream.state.close().await;
        Ok(())
    };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

fn stream_closed(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let closed = cx.argument::<JsBox<StreamHandle>>(0)?.0.state.closed();
    Ok(cx.boolean(closed))
}
