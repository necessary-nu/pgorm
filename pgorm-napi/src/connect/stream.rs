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

/// The open result and the connection it holds out of its slot.
struct Active {
    rows: Pin<Box<RowStream>>,
    operation: Operation,
}

/// A stream, which holds its connection until the last row or `close`.
pub(crate) struct StreamState {
    connection: Arc<ConnectionState>,
    /// The disjoint read whose codec reads each row, for a disjoint read's
    /// stream; its rows are otherwise values.
    disjoint: Option<Arc<crate::disjoint::Core>>,
    active: Arc<Mutex<Option<Active>>>,
    cancelled: CancellationToken,
    finished: CancellationToken,
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
        }
        drop(self.active.lock().await.take());
        self.finished.cancel();
    }
}

/// The JavaScript `RowStream`'s native half. Collected unfinished, it
/// releases its connection as a backstop; reaching the end, `return` and
/// `close` are what release it.
#[derive(Debug)]
pub(crate) struct StreamHandle(Arc<StreamState>);

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
    disjoint: Option<Arc<crate::disjoint::Core>>,
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
    let state = Arc::new(StreamState {
        connection,
        disjoint,
        active: Arc::new(Mutex::new(Some(Active {
            rows: Box::pin(rows),
            operation,
        }))),
        cancelled: CancellationToken::new(),
        finished: CancellationToken::new(),
        named: AtomicBool::new(false),
    });
    tokio::spawn(state.clone().watch());
    Ok(state)
}

/// `connectionStream(connection, sql, params, abort, disjoint?)`: open a
/// stream, which holds the connection until the last row or `close`, each
/// row read by `disjoint`'s codec when one is given.
// [spec:pgorm:req:bindings.streams]
// [spec:pgorm:req:bindings.disjoint+2]
fn connection_stream(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let connection = cx.argument::<JsBox<ConnectionHandle>>(0)?.0.clone();
    let (sql, params) = super::sql_and_params(&mut cx, 1)?;
    let abort = abort_argument(&mut cx, 3)?;
    let disjoint = match cx.argument_opt(4) {
        Some(value) => value
            .downcast::<JsBox<crate::disjoint::DisjointHandle>, _>(&mut cx)
            .ok()
            .map(|handle| handle.0.clone()),
        None => None,
    };
    let operation = match Operation::begin(connection) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    settle::promise(
        &mut cx,
        open(operation, sql, params, abort, disjoint),
        |cx, state| Ok(cx.boxed(StreamHandle(state)).upcast()),
    )
}

/// One pulled row: its values, or what a disjoint read's codec read it as.
enum Pulled {
    Values(Vec<Tagged>),
    Disjoint(crate::disjoint::Rows),
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
        row = active.rows.try_next() => row,
    };
    match row {
        Ok(Some(row)) => {
            if let Some(disjoint) = &state.disjoint {
                let rows = crate::disjoint::Rows::of(disjoint, vec![row])?;
                *slot = Some(active);
                return Ok(Some((Pulled::Disjoint(rows), None)));
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
            active.operation.restore();
            state.finished.cancel();
            Ok(None)
        }
        Err(error) => Err(failure(&error.into(), &state.connection.pool.secrets)),
    }
}

/// `streamNext(stream, tagged, abort)`: `[values, names?, kinds?]`, the
/// names and kinds on the first row, or `null` at the end. A failure, an abort or a decode error discards the connection.
// [spec:pgorm:req:bindings.streams]
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
            Pulled::Disjoint(rows) => {
                let pair = JsArray::new(cx, 1);
                let rows = crate::connect::job::Settled::into_js(Box::new(rows), cx)?;
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
// [spec:pgorm:req:bindings.streams]
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
