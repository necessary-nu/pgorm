//! Row streams: pgorm's `query_raw` row stream, pulled a row at a time, so
//! the driver's bounded buffer and the socket hold back the server while
//! JavaScript is not asking for rows.

use std::{
    fmt,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use futures_util::TryStreamExt;
use neon::{prelude::*, types::Finalize};
use pgorm::{ConnectionTrait, types::ToSql};
use tokio::sync::Mutex;
use tokio_postgres::RowStream;
use tokio_util::sync::CancellationToken;

use super::{Abort, ConnectionHandle, ConnectionState, Operation, abort_argument};
use crate::{
    codec::Codec,
    decode,
    errors::{Failure, failure},
    params::Param,
    rows, settle,
    values::Tagged,
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("connectionStream", connection_stream)?;
    cx.export_function("streamNext", stream_next)?;
    cx.export_function("streamClose", stream_close)?;
    cx.export_function("streamClosed", stream_closed)?;
    Ok(())
}

/// What a stream holds while it reads: its connection, taken out of its
/// slot, or its transaction, reserved. Released at the last row; dropped
/// before it, the connection is discarded, and the transaction with it.
pub(crate) enum Hold {
    Connection(Box<Operation>),
    Transaction(super::transaction::Lease),
}

impl Hold {
    fn release(self) {
        match self {
            Self::Connection(operation) => (*operation).restore(),
            Self::Transaction(lease) => lease.complete(),
        }
    }
}

/// The open result and what it holds.
struct Active {
    rows: Pin<Box<RowStream>>,
    hold: Hold,
}

/// What a stream that decodes its own rows makes of each: a disjoint read's
/// codec's rows, or a registered read's models.
pub(crate) type RowDecode =
    Arc<dyn Fn(tokio_postgres::Row) -> Result<crate::connect::job::Output, Failure> + Send + Sync>;

/// A stream, which holds its connection until the last row or `close`.
pub(crate) struct StreamState {
    connection: Arc<ConnectionState>,
    /// The decode of each row, for a disjoint read's or a registered read's
    /// stream; its rows are otherwise values.
    decode: Option<RowDecode>,
    active: Arc<Mutex<Option<Active>>>,
    cancelled: CancellationToken,
    finished: CancellationToken,
    /// Fires when the transaction a transaction's stream reads in ends.
    ended: CancellationToken,
    named: AtomicBool,
}

impl fmt::Debug for StreamState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamState")
            .field("closed", &self.closed())
            .finish_non_exhaustive()
    }
}

impl StreamState {
    fn closed(&self) -> bool {
        self.cancelled.is_cancelled() || self.finished.is_cancelled()
    }

    /// Release an idle stream's connection once the stream, its connection
    /// or its pool closes, without waiting for JavaScript's next pull.
    async fn watch(self: Arc<Self>) {
        tokio::select! {
            () = self.finished.cancelled() => return,
            () = self.cancelled.cancelled() => {}
            () = self.connection.closing() => {}
            () = self.ended.cancelled() => {}
        }
        drop(self.active.lock().await.take());
        self.finished.cancel();
    }
}

/// The JavaScript `RowStream`'s native half. Collected unfinished, it
/// releases its connection as a backstop; reaching the end, `return` and
/// `close` are what release it.
#[derive(Debug)]
pub(crate) struct StreamHandle(pub(crate) Arc<StreamState>);

impl Finalize for StreamHandle {}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.0.cancelled.cancel();
    }
}

async fn open(
    operation: Operation,
    sql: String,
    params: Vec<Param>,
    abort: Abort,
    decode: Option<RowDecode>,
) -> Result<Arc<StreamState>, Failure> {
    let connection = operation.state.clone();
    let secrets = &connection.pool.secrets;
    let db = operation.connection()?;
    let rows = tokio::select! {
        biased;
        () = abort.token().cancelled() => return Err(Failure::Cancelled),
        () = connection.closing() => {
            return Err(Failure::Lifecycle("the connection closed as the stream opened".to_owned()));
        }
        rows = db.query_raw(
            &sql,
            params.iter().map(|param| param as &(dyn ToSql + Sync)),
        ) => rows.map_err(|error| failure(&error, secrets))?,
    };
    Ok(held(
        connection,
        rows,
        Hold::Connection(Box::new(operation)),
        CancellationToken::new(),
        decode,
    ))
}

/// A stream over `rows`, holding `hold` until its last row.
pub(crate) fn held(
    connection: Arc<ConnectionState>,
    rows: RowStream,
    hold: Hold,
    ended: CancellationToken,
    decode: Option<RowDecode>,
) -> Arc<StreamState> {
    let state = Arc::new(StreamState {
        connection,
        decode,
        active: Arc::new(Mutex::new(Some(Active {
            rows: Box::pin(rows),
            hold,
        }))),
        cancelled: CancellationToken::new(),
        finished: CancellationToken::new(),
        ended,
        named: AtomicBool::new(false),
    });
    tokio::spawn(state.clone().watch());
    state
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
    settle::promise(
        &mut cx,
        open(operation, sql, params, abort, decode),
        |cx, state| Ok(cx.boxed(StreamHandle(state)).upcast()),
    )
}

/// One pulled row: its values, or what the stream's decode read it as.
enum Pulled {
    Values(Vec<Tagged>),
    Decoded(crate::connect::job::Output),
}

/// The next row, decoded, with the column names on the first; `None` at the
/// end, which returns the connection to its slot.
async fn next(
    state: Arc<StreamState>,
    abort: Abort,
) -> Result<Option<(Pulled, Option<Vec<String>>)>, Failure> {
    state.connection.ensure_open()?;
    if state.cancelled.is_cancelled() {
        return Err(Failure::Lifecycle("the stream is closed".to_owned()));
    }
    if state.finished.is_cancelled() {
        return Ok(None);
    }
    let mut slot = state.active.clone().try_lock_owned().map_err(|_| {
        Failure::Lifecycle("the stream is busy: one row is pulled at a time".to_owned())
    })?;
    let Some(mut active) = slot.take() else {
        return Ok(None);
    };
    let row = tokio::select! {
        biased;
        () = abort.token().cancelled() => return Err(Failure::Cancelled),
        () = state.cancelled.cancelled() => return Err(Failure::Lifecycle("the stream is closed".to_owned())),
        () = state.connection.closing() => {
            return Err(Failure::Lifecycle("the connection closed under the stream".to_owned()));
        }
        () = state.ended.cancelled() => {
            return Err(Failure::Lifecycle("the transaction ended under the stream".to_owned()));
        }
        row = active.rows.try_next() => row,
    };
    match row {
        Ok(Some(row)) => {
            if let Some(decode) = &state.decode {
                let decoded = decode(row)?;
                *slot = Some(active);
                return Ok(Some((Pulled::Decoded(decoded), None)));
            }
            let names = if state.named.swap(true, Ordering::Relaxed) {
                None
            } else {
                Some(decode::names(&row)?)
            };
            let values = decode::row(&row)?;
            *slot = Some(active);
            Ok(Some((Pulled::Values(values), names)))
        }
        Ok(None) => {
            active.hold.release();
            state.finished.cancel();
            Ok(None)
        }
        Err(error) => {
            // The server ended the statement with its error, which leaves what
            // the stream holds in a known state, as its last row does: a
            // transaction's is aborted, and its commit reports so.
            // [spec:pgorm:req:bindings.streams+1]
            if error.as_db_error().is_some() {
                active.hold.release();
                state.finished.cancel();
            }
            Err(failure(&error.into(), &state.connection.pool.secrets))
        }
    }
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
    let state = cx.argument::<JsBox<StreamHandle>>(0)?.0.clone();
    state.cancelled.cancel();
    let work = async move {
        drop(state.active.lock().await.take());
        state.finished.cancel();
        Ok(())
    };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

fn stream_closed(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let closed = cx.argument::<JsBox<StreamHandle>>(0)?.0.closed();
    Ok(cx.boolean(closed))
}
