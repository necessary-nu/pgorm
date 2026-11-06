//! COPY's data flow — `COPY … FROM STDIN` and `COPY … TO STDOUT` through
//! pgorm's `CopyTrait` — on a pool's connection, a connection, or a
//! transaction or savepoint.
//!
//! The sink and the stream `CopyTrait` returns borrow their connection or
//! transaction exclusively, so neither can sit in a JavaScript handle beside
//! what it borrows. A session task on the runtime owns the connection — or, in
//! a transaction, the frame's task runs the session itself, holding the
//! frame's lock — and drives the COPY, and JavaScript's handle reaches it over
//! channels: data in, a byte budget bounding what is queued and not yet
//! accepted, and one pull at a time out.
//!
//! The session ends with the connection kept or discarded. Every outcome
//! PostgreSQL reported — a row refused at `finish`, a query that failed part
//! way, a COPY abandoned with `CopyFail` because its source failed — leaves
//! the connection at a known point and keeps it. An abort, the connection,
//! pool or transaction closing, a COPY TO left unread, and a COPY FROM the
//! server refused before its data flow started, after which pgorm closes the
//! connection, discard it. Each JavaScript call that ends a session settles
//! only once the session has released what it ran on, so the next operation
//! on the connection or transaction finds it free.

use std::{
    fmt,
    sync::{
        Arc, Mutex as SyncMutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

use bytes::{Bytes, BytesMut};
use futures_util::{FutureExt, SinkExt, StreamExt, future::select_all};
use neon::{
    prelude::*,
    types::{Finalize, JsUint8Array, buffer::TypedArray},
};
use pgorm::{CopyInSink, CopyOutStream, CopyRowDecoder, CopyRowEncoder, CopyTrait};
use tokio::sync::{Mutex, Semaphore, TryAcquireError, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::{
    Abort, ConnectionHandle, ConnectionState, Held, Operation, Outcome, PoolHandle, PoolState,
    abort_argument,
};
use crate::{
    errors::{Failure, Redactions, failure, throw},
    settle,
    statements::{CopyFrom, CopyTo},
};

mod columns;
mod framing;
mod rows;

pub(crate) use columns::{columns_to_js, described};
use rows::RowsIn;
pub(crate) use rows::{copy_from_rows_at, copy_to_rows_at};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("poolCopyIn", pool_copy_in)?;
    cx.export_function("connectionCopyIn", connection_copy_in)?;
    cx.export_function("copyInWrite", copy_in_write)?;
    cx.export_function("copyInReady", copy_in_ready)?;
    cx.export_function("copyInFinish", copy_in_finish)?;
    cx.export_function("copyInAbort", copy_in_abort)?;
    cx.export_function("copyInEnded", copy_in_ended)?;
    cx.export_function("poolCopyOut", pool_copy_out)?;
    cx.export_function("connectionCopyOut", connection_copy_out)?;
    cx.export_function("copyOutNext", copy_out_next)?;
    cx.export_function("copyOutClose", copy_out_close)?;
    cx.export_function("copyOutClosed", copy_out_closed)?;
    cx.export_function("copyRowsEncode", rows::copy_rows_encode)?;
    cx.export_function("copyRowsNext", rows::copy_rows_next)?;
    cx.export_function("copyRowsColumns", rows::copy_rows_columns)?;
    columns::export(cx)?;
    framing::export(cx)?;
    Ok(())
}

/// The bytes JavaScript may queue for a COPY FROM beyond what its sink has
/// taken: the bound that makes a large source stream rather than accumulate.
const BUDGET: usize = 1 << 20;

/// The most a COPY TO's pull gathers from the chunks already arrived behind
/// the first. PostgreSQL sends a text or CSV row as a chunk of its own, so a
/// pull per chunk would be a round trip to JavaScript per row.
const GATHER: usize = 64 * 1024;

/// Whether a session leaves its connection for the next operation or
/// discards it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settle {
    Keep,
    Discard,
}

/// A connection is kept after a failure PostgreSQL reported, which leaves the
/// protocol at a known point; after any other, its state is unknown.
fn kept(failure: &Failure) -> Settle {
    match failure {
        Failure::Database(_) => Settle::Keep,
        _ => Settle::Discard,
    }
}

/// What ends a session from outside: the caller's abort, or what the COPY
/// runs on closing.
// [spec:pgorm:req:bindings.cancellation]
pub(crate) struct Stop {
    abort: Abort,
    ends: Vec<(CancellationToken, &'static str)>,
}

impl fmt::Debug for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stop").finish_non_exhaustive()
    }
}

impl Stop {
    fn pool(abort: Abort, pool: &PoolState) -> Self {
        Self {
            abort,
            ends: vec![(pool.closed.clone(), "the pool closed during the COPY")],
        }
    }

    fn connection(abort: Abort, connection: &ConnectionState) -> Self {
        Self {
            abort,
            ends: vec![
                (
                    connection.pool.closed.clone(),
                    "the pool closed during the COPY",
                ),
                (
                    connection.closed.clone(),
                    "the connection closed during the COPY",
                ),
            ],
        }
    }

    /// A transaction's or savepoint's frame, which also ends when its handle
    /// is collected unfinished or the whole transaction is aborted.
    pub(crate) fn frame(
        abort: Abort,
        connection: &ConnectionState,
        abandoned: &CancellationToken,
        aborted: &CancellationToken,
    ) -> Self {
        let mut stop = Self::connection(abort, connection);
        stop.ends.extend([
            (
                abandoned.clone(),
                "the transaction was abandoned during the COPY",
            ),
            (aborted.clone(), "the transaction ended during the COPY"),
        ]);
        stop
    }

    async fn fired(&self) -> Failure {
        let ends = self.ends.iter().map(|(token, why)| {
            Box::pin(async move {
                token.cancelled().await;
                *why
            })
        });
        tokio::select! {
            biased;
            () = self.abort.token().cancelled() => Failure::Cancelled,
            (why, _, _) = select_all(ends) => Failure::Lifecycle(why.to_owned()),
        }
    }
}

/// How a session ended, as its JavaScript half reads it once released.
#[derive(Debug, Default)]
pub(super) struct Ending {
    released: CancellationToken,
    failure: SyncMutex<Option<Failure>>,
}

impl Ending {
    /// Record why the session ended early; the first reason stands.
    fn fail(&self, failure: Failure) {
        let mut slot = self.failure.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(failure);
        }
    }

    fn failure(&self) -> Failure {
        self.early()
            .unwrap_or_else(|| Failure::Lifecycle("the COPY has ended".to_owned()))
    }

    /// Why the session ended early, if it did.
    fn early(&self) -> Option<Failure> {
        self.failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    async fn released(&self) {
        self.released.cancelled().await;
    }
}

/// Held by whatever owns a session's connection and dropped once it has kept
/// or discarded it — or, if that owner is itself cut off, as it is dropped —
/// marking the session released and waking every JavaScript call waiting on
/// it.
pub(crate) struct Release {
    released: CancellationToken,
    budget: Option<Arc<Semaphore>>,
}

impl fmt::Debug for Release {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Release").finish_non_exhaustive()
    }
}

impl Drop for Release {
    fn drop(&mut self) {
        if let Some(budget) = &self.budget {
            budget.close();
        }
        self.released.cancel();
    }
}

enum InMessage {
    /// A chunk, and the budget it took, given back once the sink takes it.
    Data(Bytes, u32),
    Finish(oneshot::Sender<Result<u64, Failure>>),
}

/// What a JavaScript `copyIn` and its session share.
pub(crate) struct InState {
    data: mpsc::UnboundedSender<InMessage>,
    budget: Arc<Semaphore>,
    /// The source failed: abandon the COPY with `CopyFail`.
    abandon: CancellationToken,
    ending: Ending,
    /// Set for `copyInRows`, whose rows JavaScript encodes as it writes them.
    rows: Option<RowsIn>,
}

impl fmt::Debug for InState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InState").finish_non_exhaustive()
    }
}

/// The session's end of a `copyIn`.
pub(crate) struct InLink {
    messages: mpsc::UnboundedReceiver<InMessage>,
    state: Arc<InState>,
}

impl fmt::Debug for InLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InLink").finish_non_exhaustive()
    }
}

impl InState {
    pub(crate) fn channel(rows: bool) -> (Arc<Self>, InLink, Release) {
        let (data, messages) = mpsc::unbounded_channel();
        let state = Arc::new(Self {
            data,
            budget: Arc::new(Semaphore::new(BUDGET)),
            abandon: CancellationToken::new(),
            ending: Ending::default(),
            rows: rows.then(RowsIn::default),
        });
        let release = Release {
            released: state.ending.released.clone(),
            budget: Some(state.budget.clone()),
        };
        let link = InLink {
            messages,
            state: state.clone(),
        };
        (state, link, release)
    }
}

/// The JavaScript `copyIn`'s native half. Collected before it finished —
/// its caller gone while waiting on its source — it abandons the COPY,
/// which writes nothing and leaves the connection usable.
#[derive(Debug)]
pub(crate) struct CopyInHandle(pub(super) Arc<InState>);

impl Finalize for CopyInHandle {}

impl Drop for CopyInHandle {
    fn drop(&mut self) {
        self.0.abandon.cancel();
    }
}

async fn open_in<'a, C>(db: &'a mut C, copy: &CopyFrom) -> Result<CopyInSink<'a>, pgorm::Error>
where
    C: CopyTrait + Send,
{
    match copy {
        CopyFrom::Text(statement) => db.copy_in(statement).await,
        CopyFrom::Csv(statement) => db.copy_in(statement).await,
        CopyFrom::Binary(statement) => db.copy_in(statement).await,
    }
}

/// Run a `COPY … FROM STDIN` on `db`: open it, report the opening on
/// `opened`, feed it what JavaScript writes and finish it when JavaScript
/// says, returning whether `db`'s connection is kept.
// [spec:pgorm:req:bindings.copy]
pub(crate) async fn copy_in<C>(
    db: &mut C,
    copy: &CopyFrom,
    link: InLink,
    opened: oneshot::Sender<Result<(), Failure>>,
    stop: &Stop,
    secrets: &Redactions,
) -> Settle
where
    C: CopyTrait + Send,
{
    let InLink {
        mut messages,
        state,
    } = link;
    let stopped = stop.fired();
    tokio::pin!(stopped);
    if let Some(rows) = &state.rows {
        let described = tokio::select! {
            biased;
            failure = &mut stopped => {
                let _ = opened.send(Err(failure));
                return Settle::Discard;
            }
            described = rows::describe_in(db, copy) => described,
        };
        match described {
            Ok(columns) => {
                let _ = rows.encoder.set(CopyRowEncoder::new(columns));
            }
            // A description refused is a statement refused before the COPY
            // starts, which leaves the connection as any failed statement.
            Err(error) => {
                let failure = failure(&error, secrets);
                let settle = kept(&failure);
                let _ = opened.send(Err(failure));
                return settle;
            }
        }
    }
    let sink = tokio::select! {
        biased;
        failure = &mut stopped => {
            let _ = opened.send(Err(failure));
            return Settle::Discard;
        }
        sink = open_in(db, copy) => sink,
    };
    let mut sink = match sink {
        Ok(sink) => sink,
        Err(error) => {
            // A COPY FROM the server refused before its data flow started
            // leaves a reply the next request would read as its own, so
            // pgorm closes the connection.
            let _ = opened.send(Err(failure(&error, secrets)));
            return Settle::Discard;
        }
    };
    if opened.send(Ok(())).is_err() {
        return Settle::Keep;
    }
    loop {
        let message = tokio::select! {
            biased;
            failure = &mut stopped => {
                state.ending.fail(failure);
                return Settle::Discard;
            }
            () = state.abandon.cancelled() => return Settle::Keep,
            message = messages.recv() => message,
        };
        match message {
            // Every return drops the sink unfinished, which sends `CopyFail`:
            // the server writes nothing and answers with an error the driver
            // reads past before the connection's next request.
            None => return Settle::Keep,
            Some(InMessage::Data(chunk, permits)) => {
                let fed = tokio::select! {
                    biased;
                    failure = &mut stopped => {
                        state.ending.fail(failure);
                        return Settle::Discard;
                    }
                    () = state.abandon.cancelled() => return Settle::Keep,
                    fed = sink.feed(chunk) => fed,
                };
                state.budget.add_permits(permits as usize);
                if let Err(error) = fed {
                    let failure = failure(&error, secrets);
                    let settle = kept(&failure);
                    state.ending.fail(failure);
                    return settle;
                }
            }
            Some(InMessage::Finish(reply)) => {
                let rows = tokio::select! {
                    biased;
                    failure = &mut stopped => {
                        let _ = reply.send(Err(failure));
                        return Settle::Discard;
                    }
                    rows = sink.finish() => rows.map_err(|error| failure(&error, secrets)),
                };
                let settle = match &rows {
                    Ok(_) => Settle::Keep,
                    Err(failure) => kept(failure),
                };
                let _ = reply.send(rows);
                return settle;
            }
        }
    }
}

/// The two directions' shared state, as an opening reads it.
pub(super) trait Released {
    fn ending(&self) -> &Ending;
}

impl Released for InState {
    fn ending(&self) -> &Ending {
        &self.ending
    }
}

impl Released for OutState {
    fn ending(&self) -> &Ending {
        &self.ending
    }
}

/// The session's state once the server has accepted its COPY, or, once the
/// session has released what it ran on, why it did not start.
async fn started<T: Released>(
    state: Arc<T>,
    opened: oneshot::Receiver<Result<(), Failure>>,
) -> Result<Arc<T>, Failure> {
    let outcome = opened.await;
    match outcome {
        Ok(Ok(())) => Ok(state),
        Ok(Err(failure)) => {
            state.ending().released().await;
            Err(failure)
        }
        Err(_) => {
            state.ending().released().await;
            Err(state.ending().failure())
        }
    }
}

/// Start `session` and return the promise of its opening, which resolves
/// with the handle `make` boxes.
fn opening<'cx, S, T, H>(
    cx: &mut FunctionContext<'cx>,
    state: Arc<T>,
    session: S,
    opened: oneshot::Receiver<Result<(), Failure>>,
    make: fn(Arc<T>) -> H,
) -> JsResult<'cx, JsPromise>
where
    S: Future<Output = ()> + Send + 'static,
    T: Released + Send + Sync + 'static,
    H: Finalize + 'static,
{
    let work = async move {
        tokio::spawn(session);
        started(state, opened).await
    };
    settle::promise(cx, work, move |cx, state| {
        Ok(cx.boxed(make(state)).upcast())
    })
}

/// The promise of the opening of a session a transaction's frame runs, sent
/// to it as `sent` says.
pub(super) fn opened<'cx, T, H>(
    cx: &mut FunctionContext<'cx>,
    state: Arc<T>,
    sent: Result<(), Failure>,
    opened: oneshot::Receiver<Result<(), Failure>>,
    make: fn(Arc<T>) -> H,
) -> JsResult<'cx, JsPromise>
where
    T: Released + Send + Sync + 'static,
    H: Finalize + 'static,
{
    if let Err(failure) = sent {
        return settle::rejected(cx, failure);
    }
    settle::promise(cx, started(state, opened), move |cx, state| {
        Ok(cx.boxed(make(state)).upcast())
    })
}

/// `poolCopyIn(pool, copy, abort, rows)`: a COPY FROM on a connection the pool
/// lends it alone, returned when the COPY finishes or is abandoned and
/// discarded when it is cut off.
// [spec:pgorm:req:bindings.copy]
fn pool_copy_in(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let pool = cx.argument::<JsBox<PoolHandle>>(0)?.0.clone();
    let (copy, rows) = copy_from_rows_at(&mut cx, 1, 3)?;
    let abort = abort_argument(&mut cx, 2)?;
    let (state, link, release) = InState::channel(rows);
    let (opened, receive) = oneshot::channel();
    let session = async move {
        let _release = release;
        let stop = Stop::pool(abort.clone(), &pool);
        let mut connection = match abort.guard(pool.get()).await {
            Ok(connection) => connection,
            Err(failure) => {
                let _ = opened.send(Err(failure));
                return;
            }
        };
        let settle = match &mut connection {
            Held::Plain(db) => copy_in(db, &copy, link, opened, &stop, &pool.secrets).await,
            Held::Metered(db) => copy_in(db, &copy, link, opened, &stop, &pool.secrets).await,
        };
        match settle {
            Settle::Keep => drop(connection),
            Settle::Discard => connection.discard(),
        }
    };
    opening(&mut cx, state, session, receive, CopyInHandle)
}

/// `connectionCopyIn(connection, copy, abort, rows)`: a COPY FROM holding the
/// connection, which refuses other work until it ends.
// [spec:pgorm:req:bindings.copy]
fn connection_copy_in(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let connection = cx.argument::<JsBox<ConnectionHandle>>(0)?.0.clone();
    let (copy, rows) = copy_from_rows_at(&mut cx, 1, 3)?;
    let abort = abort_argument(&mut cx, 2)?;
    let operation = match Operation::begin(connection) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let (state, link, release) = InState::channel(rows);
    let (opened, receive) = oneshot::channel();
    let session = async move {
        let _release = release;
        let mut operation = operation;
        let stop = Stop::connection(abort, &operation.state);
        let secrets = operation.state.pool.secrets.clone();
        let settle = match operation.connection_mut() {
            Ok(Held::Plain(db)) => copy_in(db, &copy, link, opened, &stop, &secrets).await,
            Ok(Held::Metered(db)) => copy_in(db, &copy, link, opened, &stop, &secrets).await,
            Err(failure) => {
                let _ = opened.send(Err(failure));
                Settle::Discard
            }
        };
        if settle == Settle::Keep {
            operation.restore();
        }
    };
    opening(&mut cx, state, session, receive, CopyInHandle)
}

fn in_state(cx: &mut FunctionContext) -> NeonResult<Arc<InState>> {
    Ok(cx.argument::<JsBox<CopyInHandle>>(0)?.0.clone())
}

/// The budget a chunk of `length` bytes takes: all of it, or the whole
/// budget for a chunk past it, which then waits for the queue to empty.
fn permits(length: usize) -> u32 {
    u32::try_from(length.min(BUDGET)).unwrap_or(u32::MAX)
}

/// `copyInWrite(copyIn, chunk)`: queue a copy of `chunk` for the COPY, and
/// `true`; or `false`, queuing nothing, while the budget is spent. Throws
/// once the COPY has ended.
// [spec:pgorm:req:bindings.copy]
fn copy_in_write(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let state = in_state(&mut cx)?;
    let chunk = cx.argument::<JsUint8Array>(1)?;
    let chunk = Bytes::copy_from_slice(chunk.as_slice(&cx));
    let permits = permits(chunk.len());
    match state.budget.try_acquire_many(permits) {
        Ok(permit) => {
            permit.forget();
            if state.data.send(InMessage::Data(chunk, permits)).is_err() {
                return throw(&mut cx, state.ending.failure());
            }
            Ok(cx.boolean(true))
        }
        Err(TryAcquireError::NoPermits) => Ok(cx.boolean(false)),
        Err(TryAcquireError::Closed) => throw(&mut cx, state.ending.failure()),
    }
}

/// `copyInReady(copyIn, length)`: resolve once a chunk of `length` bytes fits
/// the budget, or reject with why the COPY ended.
fn copy_in_ready(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = in_state(&mut cx)?;
    let length = cx.argument::<JsNumber>(1)?.value(&mut cx);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let permits = permits(length.max(0.0) as usize);
    let work = async move {
        match state.budget.acquire_many(permits).await {
            Ok(permit) => {
                drop(permit);
                Ok(())
            }
            Err(_) => {
                state.ending.released().await;
                Err(state.ending.failure())
            }
        }
    };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

/// `copyInFinish(copyIn)`: complete the COPY and resolve with the rows it
/// wrote, once its connection is free again. A row the server refused is
/// reported here.
// [spec:pgorm:req:bindings.copy]
fn copy_in_finish(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = in_state(&mut cx)?;
    if let Some(rows) = &state.rows {
        let mut tail = BytesMut::new();
        rows.head(&mut tail);
        CopyRowEncoder::trailer(&mut tail);
        let _ = state.data.send(InMessage::Data(tail.freeze(), 0));
    }
    let (reply, receive) = oneshot::channel();
    let sent = state.data.send(InMessage::Finish(reply)).is_ok();
    let work = async move {
        let outcome = if sent { receive.await.ok() } else { None };
        state.ending.released().await;
        outcome.unwrap_or_else(|| Err(state.ending.failure()))
    };
    settle::promise(&mut cx, work, |cx, rows| {
        Outcome::Count(rows).into_js(cx, false)
    })
}

/// `copyInAbort(copyIn)`: abandon the COPY because its source failed, and
/// resolve once its connection is free again: outside a transaction nothing
/// was written; inside one, the transaction is aborted.
// [spec:pgorm:req:bindings.copy]
fn copy_in_abort(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = in_state(&mut cx)?;
    state.abandon.cancel();
    let work = async move {
        state.ending.released().await;
        Ok(())
    };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

/// `copyInEnded(copyIn)`: settle once the session has released what it ran
/// on, rejecting with why if it was cut off — the connection, pool or
/// transaction closing — so that `copyIn` stops waiting on its source. In
/// flight for the COPY's whole length, it holds the event loop as the COPY
/// does.
// [spec:pgorm:req:bindings.copy]
// [spec:pgorm:req:bindings.cancellation]
fn copy_in_ended(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = in_state(&mut cx)?;
    let work = async move {
        state.ending.released().await;
        state.ending.early().map_or(Ok(()), Err)
    };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

/// One pull of a COPY TO, under its caller's abort.
struct Pull {
    abort: Abort,
    reply: oneshot::Sender<Result<Option<Bytes>, Failure>>,
}

/// What a JavaScript `CopyOutStream` and its session share.
pub(crate) struct OutState {
    pulls: mpsc::UnboundedSender<Pull>,
    busy: Arc<Mutex<()>>,
    /// Closed before its end: the session discards its connection, which the
    /// server would otherwise go on sending the rest to.
    close: CancellationToken,
    finished: AtomicBool,
    ending: Ending,
    /// Set for `copyOutRows`: the decoder over the columns its session
    /// describes before the COPY starts.
    rows: Option<SyncMutex<Option<CopyRowDecoder>>>,
}

impl fmt::Debug for OutState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutState").finish_non_exhaustive()
    }
}

/// The session's end of a `CopyOutStream`.
pub(crate) struct OutLink {
    pulls: mpsc::UnboundedReceiver<Pull>,
    state: Arc<OutState>,
}

impl fmt::Debug for OutLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutLink").finish_non_exhaustive()
    }
}

impl OutState {
    pub(crate) fn channel(rows: bool) -> (Arc<Self>, OutLink, Release) {
        let (pulls, receiver) = mpsc::unbounded_channel();
        let state = Arc::new(Self {
            pulls,
            busy: Arc::new(Mutex::new(())),
            close: CancellationToken::new(),
            finished: AtomicBool::new(false),
            ending: Ending::default(),
            rows: rows.then(|| SyncMutex::new(None)),
        });
        let release = Release {
            released: state.ending.released.clone(),
            budget: None,
        };
        let link = OutLink {
            pulls: receiver,
            state: state.clone(),
        };
        (state, link, release)
    }

    fn closed(&self) -> bool {
        self.close.is_cancelled() || self.ending.released.is_cancelled()
    }
}

/// The JavaScript `CopyOutStream`'s native half. Collected before its end,
/// it closes the stream, discarding the connection; reaching the end and
/// `close` are what release it.
#[derive(Debug)]
pub(crate) struct CopyOutHandle(pub(super) Arc<OutState>);

impl Finalize for CopyOutHandle {}

impl Drop for CopyOutHandle {
    fn drop(&mut self) {
        self.0.close.cancel();
    }
}

async fn open_out<'a, C>(db: &'a mut C, copy: &CopyTo) -> Result<CopyOutStream<'a>, pgorm::Error>
where
    C: CopyTrait + Send,
{
    match copy {
        CopyTo::Text(statement) => db.copy_out(statement).await,
        CopyTo::Csv(statement) => db.copy_out(statement).await,
        CopyTo::Binary(statement) => db.copy_out(statement).await,
    }
}

/// The next piece of a COPY TO's data: the chunk the server sends next and
/// those already arrived behind it, up to [`GATHER`] bytes, or `None` at the
/// end. An end or a failure met behind gathered data is held in `tail` for
/// the pull after.
async fn gather(
    stream: &mut CopyOutStream<'_>,
    tail: &mut Option<Result<(), Failure>>,
    secrets: &Redactions,
) -> Result<Option<Bytes>, Failure> {
    if let Some(end) = tail.take() {
        return end.map(|()| None);
    }
    let first = match stream.next().await {
        None => return Ok(None),
        Some(Err(error)) => return Err(failure(&error, secrets)),
        Some(Ok(chunk)) => chunk,
    };
    if first.len() >= GATHER {
        return Ok(Some(first));
    }
    let mut gathered = BytesMut::from(first.as_ref());
    while gathered.len() < GATHER {
        match stream.next().now_or_never() {
            None => break,
            Some(None) => {
                *tail = Some(Ok(()));
                break;
            }
            Some(Some(Err(error))) => {
                *tail = Some(Err(failure(&error, secrets)));
                break;
            }
            Some(Some(Ok(chunk))) => gathered.extend_from_slice(&chunk),
        }
    }
    Ok(Some(gathered.freeze()))
}

/// Run a `COPY … TO STDOUT` on `db`: open it, report the opening on
/// `opened`, and answer each pull with the data that has arrived, returning
/// whether `db`'s connection is kept: at the end, or after a failure the
/// server reported, and not when the stream closes before its end.
// [spec:pgorm:req:bindings.copy]
pub(crate) async fn copy_out<C>(
    db: &mut C,
    copy: &CopyTo,
    link: OutLink,
    opened: oneshot::Sender<Result<(), Failure>>,
    stop: &Stop,
    secrets: &Redactions,
) -> Settle
where
    C: CopyTrait + Send,
{
    let OutLink { mut pulls, state } = link;
    let stopped = stop.fired();
    tokio::pin!(stopped);
    if let Some(rows) = &state.rows {
        let described = tokio::select! {
            biased;
            failure = &mut stopped => {
                let _ = opened.send(Err(failure));
                return Settle::Discard;
            }
            described = rows::describe_out(db, copy) => described,
        };
        match described {
            Ok(columns) => {
                *rows.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some(CopyRowDecoder::new(columns));
            }
            Err(error) => {
                let failure = failure(&error, secrets);
                let settle = kept(&failure);
                let _ = opened.send(Err(failure));
                return settle;
            }
        }
    }
    let stream = tokio::select! {
        biased;
        failure = &mut stopped => {
            let _ = opened.send(Err(failure));
            return Settle::Discard;
        }
        stream = open_out(db, copy) => stream,
    };
    let mut stream = match stream {
        Ok(stream) => stream,
        Err(error) => {
            let failure = failure(&error, secrets);
            let settle = kept(&failure);
            let _ = opened.send(Err(failure));
            return settle;
        }
    };
    if opened.send(Ok(())).is_err() {
        return Settle::Discard;
    }
    let mut tail = None;
    loop {
        let pull = tokio::select! {
            biased;
            failure = &mut stopped => {
                state.ending.fail(failure);
                return Settle::Discard;
            }
            () = state.close.cancelled() => return Settle::Discard,
            pull = pulls.recv() => pull,
        };
        let Some(Pull { abort, reply }) = pull else {
            return Settle::Discard;
        };
        let chunk = tokio::select! {
            biased;
            () = abort.token().cancelled() => {
                let _ = reply.send(Err(Failure::Cancelled));
                return Settle::Discard;
            }
            failure = &mut stopped => {
                state.ending.fail(failure.clone());
                let _ = reply.send(Err(failure));
                return Settle::Discard;
            }
            () = state.close.cancelled() => {
                let _ = reply.send(Err(Failure::Lifecycle("the COPY stream is closed".to_owned())));
                return Settle::Discard;
            }
            chunk = gather(&mut stream, &mut tail, secrets) => chunk,
        };
        match chunk {
            Ok(Some(chunk)) => {
                let _ = reply.send(Ok(Some(chunk)));
            }
            Ok(None) => {
                state.finished.store(true, Ordering::Release);
                let _ = reply.send(Ok(None));
                return Settle::Keep;
            }
            Err(failure) => {
                let settle = kept(&failure);
                state.ending.fail(failure.clone());
                let _ = reply.send(Err(failure));
                return settle;
            }
        }
    }
}

/// `poolCopyOut(pool, copy, abort, rows)`: a COPY TO on a connection of its own,
/// returned to the pool at its end and discarded if it closes before.
// [spec:pgorm:req:bindings.copy]
fn pool_copy_out(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let pool = cx.argument::<JsBox<PoolHandle>>(0)?.0.clone();
    let (copy, rows) = copy_to_rows_at(&mut cx, 1, 3)?;
    let abort = abort_argument(&mut cx, 2)?;
    let (state, link, release) = OutState::channel(rows);
    let (opened, receive) = oneshot::channel();
    let session = async move {
        let _release = release;
        let stop = Stop::pool(abort.clone(), &pool);
        let mut connection = match abort.guard(pool.get()).await {
            Ok(connection) => connection,
            Err(failure) => {
                let _ = opened.send(Err(failure));
                return;
            }
        };
        let settle = match &mut connection {
            Held::Plain(db) => copy_out(db, &copy, link, opened, &stop, &pool.secrets).await,
            Held::Metered(db) => copy_out(db, &copy, link, opened, &stop, &pool.secrets).await,
        };
        match settle {
            Settle::Keep => drop(connection),
            Settle::Discard => connection.discard(),
        }
    };
    opening(&mut cx, state, session, receive, CopyOutHandle)
}

/// `connectionCopyOut(connection, copy, abort, rows)`: a COPY TO holding the
/// connection, which refuses other work until the stream ends or closes.
// [spec:pgorm:req:bindings.copy]
fn connection_copy_out(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let connection = cx.argument::<JsBox<ConnectionHandle>>(0)?.0.clone();
    let (copy, rows) = copy_to_rows_at(&mut cx, 1, 3)?;
    let abort = abort_argument(&mut cx, 2)?;
    let operation = match Operation::begin(connection) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let (state, link, release) = OutState::channel(rows);
    let (opened, receive) = oneshot::channel();
    let session = async move {
        let _release = release;
        let mut operation = operation;
        let stop = Stop::connection(abort, &operation.state);
        let secrets = operation.state.pool.secrets.clone();
        let settle = match operation.connection_mut() {
            Ok(Held::Plain(db)) => copy_out(db, &copy, link, opened, &stop, &secrets).await,
            Ok(Held::Metered(db)) => copy_out(db, &copy, link, opened, &stop, &secrets).await,
            Err(failure) => {
                let _ = opened.send(Err(failure));
                Settle::Discard
            }
        };
        if settle == Settle::Keep {
            operation.restore();
        }
    };
    opening(&mut cx, state, session, receive, CopyOutHandle)
}

fn out_state(cx: &mut FunctionContext) -> NeonResult<Arc<OutState>> {
    Ok(cx.argument::<JsBox<CopyOutHandle>>(0)?.0.clone())
}

/// `copyOutNext(stream, abort)`: the next piece of data as a `Uint8Array`,
/// or `null` at the end, which resolves once the connection is free again.
/// One pull runs at a time.
// [spec:pgorm:req:bindings.copy]
fn copy_out_next(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = out_state(&mut cx)?;
    let abort = abort_argument(&mut cx, 1)?;
    if state.finished.load(Ordering::Acquire) {
        let work = async move {
            state.ending.released().await;
            Ok(None)
        };
        return settle::promise(&mut cx, work, chunk_to_js);
    }
    if state.close.is_cancelled() {
        let closed = Failure::Lifecycle("the COPY stream is closed".to_owned());
        return settle::rejected(&mut cx, closed);
    }
    let Ok(busy) = state.busy.clone().try_lock_owned() else {
        let busy =
            Failure::Lifecycle("the COPY stream is busy: one chunk is pulled at a time".to_owned());
        return settle::rejected(&mut cx, busy);
    };
    let (reply, receive) = oneshot::channel();
    let sent = state.pulls.send(Pull { abort, reply }).is_ok();
    let work = async move {
        let _busy = busy;
        let outcome = if sent { receive.await.ok() } else { None };
        match outcome {
            Some(Ok(Some(chunk))) => Ok(Some(chunk)),
            Some(outcome) => {
                state.ending.released().await;
                outcome
            }
            None => {
                state.ending.released().await;
                Err(state.ending.failure())
            }
        }
    };
    settle::promise(&mut cx, work, chunk_to_js)
}

fn chunk_to_js<'cx>(cx: &mut Cx<'cx>, chunk: Option<Bytes>) -> JsResult<'cx, JsValue> {
    match chunk {
        Some(chunk) => Ok(JsUint8Array::from_slice(cx, &chunk)?.upcast()),
        None => Ok(cx.null().upcast()),
    }
}

/// `copyOutClose(stream)`: end the stream, discarding its connection if data
/// remained, and resolve once it is released.
// [spec:pgorm:req:bindings.copy]
fn copy_out_close(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = out_state(&mut cx)?;
    state.close.cancel();
    let work = async move {
        state.ending.released().await;
        Ok(())
    };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

fn copy_out_closed(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let closed = out_state(&mut cx)?.closed();
    Ok(cx.boolean(closed))
}
