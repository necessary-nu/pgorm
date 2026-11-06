//! Pools, connections, transactions, row streams, COPY and listeners, which
//! JavaScript holds as boxed handles and releases explicitly.
//!
//! A pool's connections live on the runtime; a JavaScript `Connection` owns
//! one checked out of it until `close`, and every operation takes it out of
//! its slot for the operation's length, so a second operation on it while one
//! is running is refused rather than queued or raced. An operation whose
//! outcome becomes unknown — aborted by its caller, or cut off by the pool
//! closing — discards the connection instead of returning it to the pool.
//! Nothing here waits on garbage collection to release a connection: closing
//! a handle releases it, and a handle that is collected unclosed releases
//! what it holds only as a backstop.

mod config;
mod copy;
mod hooks;
pub(crate) mod job;
mod listen;
pub(crate) mod metrics;
mod migrations;
mod pools;
pub(crate) mod stream;
mod transaction;

use std::{
    fmt,
    future::Future,
    ops::Deref,
    sync::{Arc, Mutex as SyncMutex, PoisonError, Weak},
    time::Duration,
};

use neon::{prelude::*, types::Finalize};
use pgorm::{
    ConnectionTrait, DatabaseConnection,
    metric::{InstrumentedConnection, InstrumentedPool},
    types::ToSql,
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

use crate::{
    codec::Codec,
    errors::{Failure, Redactions, failure},
    params::{self, Param},
    rows::{self, Decoded},
    settle, statements,
    values::read,
};

use config::Built;
use job::Database;
use metrics::Metrics;

pub(crate) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("abortToken", abort_token)?;
    cx.export_function("abort", abort)?;
    cx.export_function("poolClose", pool_close)?;
    cx.export_function("poolClosed", pool_closed)?;
    cx.export_function("queryFingerprint", metrics::query_fingerprint)?;
    cx.export_function("poolAcquire", pool_acquire)?;
    cx.export_function("poolRun", pool_run)?;
    cx.export_function("connectionRun", connection_run)?;
    cx.export_function("connectionPing", connection_ping)?;
    cx.export_function("connectionClose", connection_close)?;
    cx.export_function("connectionDiscard", connection_discard)?;
    cx.export_function("connectionClosed", connection_closed)?;
    hooks::export(cx)?;
    pools::export(cx)?;
    transaction::export(cx)?;
    stream::export(cx)?;
    copy::export(cx)?;
    listen::export(cx)?;
    job::export(cx)?;
    migrations::export(cx)?;
    Ok(())
}

/// A cancellation the caller's `AbortSignal` fires.
// [spec:pgorm:req:napi.cancellation+2]
#[derive(Debug, Clone)]
pub(crate) struct Abort(CancellationToken);

impl Finalize for Abort {}

fn abort_token(mut cx: FunctionContext) -> JsResult<JsBox<Abort>> {
    Ok(cx.boxed(Abort(CancellationToken::new())))
}

fn abort(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    cx.argument::<JsBox<Abort>>(0)?.0.cancel();
    Ok(cx.undefined())
}

/// The abort token at `index`, if the caller passed one.
pub(crate) fn abort_argument(cx: &mut FunctionContext, index: usize) -> NeonResult<Abort> {
    match cx.argument_opt(index) {
        Some(value) if !value.is_a::<JsUndefined, _>(cx) && !value.is_a::<JsNull, _>(cx) => {
            Ok((**value.downcast_or_throw::<JsBox<Abort>, _>(cx)?).clone())
        }
        _ => Ok(Abort(CancellationToken::new())),
    }
}

impl Abort {
    /// `work`, or `Cancelled` once the caller aborts.
    pub(crate) async fn guard<T>(
        &self,
        work: impl Future<Output = Result<T, Failure>>,
    ) -> Result<T, Failure> {
        tokio::select! {
            biased;
            () = self.0.cancelled() => Err(Failure::Cancelled),
            outcome = work => outcome,
        }
    }

    pub(crate) fn token(&self) -> &CancellationToken {
        &self.0
    }
}

/// What a statement's result becomes in JavaScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Terminal {
    /// The affected-row count.
    Execute,
    /// Every row.
    All,
    /// Exactly one row.
    One,
    /// At most one row.
    Optional,
}

impl Terminal {
    fn read(cx: &mut FunctionContext, index: usize) -> NeonResult<Self> {
        let name = cx.argument::<JsString>(index)?.value(cx);
        match name.as_str() {
            "execute" => Ok(Self::Execute),
            "all" => Ok(Self::All),
            "one" => Ok(Self::One),
            "optional" => Ok(Self::Optional),
            _ => cx.throw_type_error("unknown result terminal"),
        }
    }
}

/// A statement and its bound parameters, ready to run anywhere.
#[derive(Debug)]
pub(crate) struct Statement {
    pub(crate) terminal: Terminal,
    sql: String,
    params: Vec<Param>,
}

impl Statement {
    /// The terminal, statement text and parameters at `index` onwards.
    fn read(cx: &mut FunctionContext, index: usize) -> NeonResult<Self> {
        let terminal = Terminal::read(cx, index)?;
        let (sql, params) = sql_and_params(cx, index + 1)?;
        Ok(Self {
            terminal,
            sql,
            params,
        })
    }

    /// Run on `db`, a connection or a transaction, and decode the result.
    /// A cardinality the terminal does not admit is a `DecodeError`, decided
    /// after the query and never in place of a database or decode failure.
    // [spec:pgorm:req:bindings.results+2]
    pub(crate) async fn run<C>(self, db: &C, secrets: &Redactions) -> Result<Outcome, Failure>
    where
        C: ConnectionTrait + ?Sized,
    {
        let bound: Vec<&(dyn ToSql + Sync)> = self
            .params
            .iter()
            .map(|param| param as &(dyn ToSql + Sync))
            .collect();
        if self.terminal == Terminal::Execute {
            return db
                .execute(&self.sql, &bound)
                .await
                .map(Outcome::Count)
                .map_err(|error| failure(&error, secrets));
        }
        let rows = db
            .query_all(&self.sql, &bound)
            .await
            .map_err(|error| failure(&error, secrets))?;
        let expected = match self.terminal {
            // [spec:pgorm:req:bindings.row-count]
            Terminal::One if rows.len() != 1 => "exactly one row",
            Terminal::Optional if rows.len() > 1 => "at most one row",
            _ => "",
        };
        if !expected.is_empty() {
            return Err(Failure::Decode(format!(
                "expected {expected}, received {}",
                rows.len()
            )));
        }
        rows::decode(&rows).map(Outcome::Rows)
    }
}

/// The statement at `index` and its parameters at `index + 1`: SQL text and
/// the values it binds, or a statement JavaScript built, which carries its
/// own and is built here exactly as `inspect()` builds it.
// [spec:pgorm:req:bindings.statements]
pub(crate) fn sql_and_params(
    cx: &mut FunctionContext,
    index: usize,
) -> NeonResult<(String, Vec<Param>)> {
    let sql = cx.argument::<JsValue>(index)?;
    if let Some(node) = statements::node(cx, sql) {
        let (sql, values) = statements::compile(cx, &node)?;
        if sql.contains('\0') {
            return read::refuse(cx, "statement text cannot contain NUL");
        }
        let params = values
            .0
            .into_iter()
            .map(|value| Param::Value(pgorm::ValueHolder(value)))
            .collect();
        return Ok((sql, params));
    }
    let sql = read::sql(cx, sql)?;
    let values = cx.argument::<JsArray>(index + 1)?;
    let codec = Codec::get(cx)?;
    let params = params::read(cx, codec, values)?;
    Ok((sql, params))
}

/// A statement's result: a count, or decoded rows.
#[derive(Debug)]
pub(crate) enum Outcome {
    Count(u64),
    Rows(Decoded),
}

impl Outcome {
    /// `execute`'s count as a number, exact; the rows as `[names, rows]`.
    pub(crate) fn into_js<'cx>(self, cx: &mut Cx<'cx>, tagged: bool) -> JsResult<'cx, JsValue> {
        match self {
            Self::Count(count) => {
                #[allow(clippy::cast_precision_loss)]
                if count > (1 << 53) - 1 {
                    let error = Failure::Decode(format!(
                        "{count} rows is past the integers a number holds exactly"
                    ))
                    .into_js(cx)?;
                    return cx.throw(error);
                }
                #[allow(clippy::cast_precision_loss)]
                Ok(cx.number(count as f64).upcast())
            }
            Self::Rows(decoded) => rows::to_js(cx, decoded, tagged),
        }
    }
}

/// A pool, shared by the JavaScript `Pool` and every connection it lent.
pub(crate) struct PoolState {
    pool: pgorm::DatabasePool,
    pub(crate) secrets: Redactions,
    pub(crate) closed: CancellationToken,
    acquire_timeout: Duration,
    connections: SyncMutex<Vec<Weak<ConnectionState>>>,
    /// The listeners opened with this pool's configuration, which close
    /// with it though their connections are not the pool's.
    listeners: SyncMutex<Vec<Weak<listen::ListenerState>>>,
}

impl fmt::Debug for PoolState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoolState")
            .field("closed", &self.closed.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl PoolState {
    pub(crate) fn ensure_open(&self) -> Result<(), Failure> {
        if self.closed.is_cancelled() {
            Err(Failure::Lifecycle("the pool is closed".to_owned()))
        } else {
            Ok(())
        }
    }

    /// Refuse new work, close idle connections, and hand back every lent
    /// connection for release. Every lent connection, its operations,
    /// transactions and streams watch this pool's token, so cancelling it
    /// closes them all and cuts short what runs.
    fn close_now(&self) -> Vec<Arc<ConnectionState>> {
        self.closed.cancel();
        self.pool.close();
        let mut registry = self
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let connections = registry.iter().filter_map(Weak::upgrade).collect();
        registry.clear();
        connections
    }

    /// The listeners still open, for release once the pool has closed.
    fn take_listeners(&self) -> Vec<Arc<listen::ListenerState>> {
        let mut registry = self
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let listeners = registry.iter().filter_map(Weak::upgrade).collect();
        registry.clear();
        listeners
    }
}

/// A pool as a JavaScript `Pool` holds it: its state, which every
/// instrumented pool wrapping it shares, and for an instrumented pool,
/// pgorm's wrapper around it, which every connection it lends is checked out
/// through.
#[derive(Debug, Clone)]
pub(crate) struct Lender {
    pub(crate) state: Arc<PoolState>,
    metered: Option<InstrumentedPool<Metrics>>,
}

impl Deref for Lender {
    type Target = PoolState;

    fn deref(&self) -> &PoolState {
        &self.state
    }
}

impl Lender {
    /// The pool `built` holds, lending its connections plain.
    fn new(built: Built) -> Self {
        Self {
            state: Arc::new(PoolState {
                pool: built.pool,
                secrets: built.secrets,
                closed: CancellationToken::new(),
                acquire_timeout: built.acquire_timeout,
                connections: SyncMutex::new(Vec::new()),
                listeners: SyncMutex::new(Vec::new()),
            }),
            metered: None,
        }
    }

    /// A connection, within the acquire budget, unless the pool closes
    /// first. Through an instrumented pool the acquisition is reported, and
    /// the connection reports what runs on it.
    async fn get(&self) -> Result<Held, Failure> {
        self.ensure_open()?;
        let acquire = async {
            match &self.metered {
                Some(metered) => metered.get().await.map(Held::Metered),
                None => self.pool.get().await.map(Held::Plain),
            }
        };
        let connection = tokio::select! {
            () = self.closed.cancelled() => {
                return Err(Failure::Lifecycle("the pool is closed".to_owned()));
            }
            acquired = tokio::time::timeout(self.acquire_timeout, acquire) => acquired
                .map_err(|_| Failure::Timeout(format!(
                    "no connection became free within {} ms",
                    self.acquire_timeout.as_millis()
                )))?
                .map_err(|error| failure(&error, &self.secrets))?,
        };
        if self.closed.is_cancelled() {
            connection.discard();
            return Err(Failure::Lifecycle("the pool is closed".to_owned()));
        }
        Ok(connection)
    }
}

/// A connection checked out of a pool: a plain one, or one an instrumented
/// pool lent, which reports what runs on it.
// [spec:pgorm:req:bindings.metrics]
#[derive(Debug)]
pub(crate) enum Held {
    Plain(DatabaseConnection),
    Metered(InstrumentedConnection<Metrics>),
}

impl Held {
    pub(crate) fn database(&self) -> Database<'_> {
        match self {
            Self::Plain(connection) => Database::Connection(connection),
            Self::Metered(connection) => Database::InstrumentedConnection(connection),
        }
    }

    pub(crate) fn discard(self) {
        match self {
            Self::Plain(connection) => connection.discard(),
            Self::Metered(connection) => connection.into_inner().discard(),
        }
    }
}

/// The JavaScript `Pool`'s native half.
#[derive(Debug)]
pub(crate) struct PoolHandle(Lender);

impl Finalize for PoolHandle {}

/// One connection checked out of a pool, in its slot while no operation
/// holds it.
pub(crate) struct ConnectionState {
    pub(crate) pool: Arc<PoolState>,
    pub(crate) closed: CancellationToken,
    slot: Arc<Mutex<Option<Held>>>,
}

impl fmt::Debug for ConnectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionState")
            .field("closed", &self.closed.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl ConnectionState {
    /// Close, returning an idle connection to the pool; one an operation
    /// holds is discarded when that operation sees the close.
    fn close_now(&self) {
        self.closed.cancel();
        if let Ok(mut slot) = self.slot.try_lock() {
            drop(slot.take());
        }
    }

    /// Close, discarding the connection rather than returning it: one an
    /// operation holds is discarded by that operation once it sees the close.
    // [spec:pgorm:req:bindings.connections+3]
    fn discard_now(&self) {
        self.closed.cancel();
        if let Ok(mut slot) = self.slot.try_lock()
            && let Some(connection) = slot.take()
        {
            connection.discard();
        }
    }

    pub(crate) fn ensure_open(&self) -> Result<(), Failure> {
        self.pool.ensure_open()?;
        if self.closed.is_cancelled() {
            Err(Failure::Lifecycle("the connection is closed".to_owned()))
        } else {
            Ok(())
        }
    }

    /// Resolves once the connection or its pool closes.
    pub(crate) async fn closing(&self) {
        tokio::select! {
            () = self.closed.cancelled() => {}
            () = self.pool.closed.cancelled() => {}
        }
    }
}

/// The JavaScript `Connection`'s native half. Collected unclosed, it
/// returns its connection to the pool if the connection is idle, and leaves
/// alone one an operation still holds, which goes back when that operation's
/// last reference to it is gone: collection never cuts work short.
#[derive(Debug)]
pub(crate) struct ConnectionHandle(Arc<ConnectionState>);

impl Finalize for ConnectionHandle {}

impl Drop for ConnectionHandle {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.0.slot.try_lock()
            && let Some(connection) = slot.take()
        {
            self.0.closed.cancel();
            drop(connection);
        }
    }
}

/// A connection taken out of its slot for one operation. Restored, it goes
/// back; dropped unrestored — the operation aborted or cut off — it is
/// discarded and its JavaScript connection closed, because its state is
/// unknown.
// [spec:pgorm:req:bindings.connections+3]
// [spec:pgorm:req:bindings.cancellation]
pub(crate) struct Operation {
    slot: OwnedMutexGuard<Option<Held>>,
    connection: Option<Held>,
    pub(crate) state: Arc<ConnectionState>,
}

impl fmt::Debug for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Operation").finish_non_exhaustive()
    }
}

impl Operation {
    pub(crate) fn begin(state: Arc<ConnectionState>) -> Result<Self, Failure> {
        state.ensure_open()?;
        let mut slot = state.slot.clone().try_lock_owned().map_err(|_| {
            Failure::Lifecycle(
                "the connection is busy: an operation, a transaction or a stream holds it"
                    .to_owned(),
            )
        })?;
        let connection = slot
            .take()
            .ok_or_else(|| Failure::Lifecycle("the connection is closed".to_owned()))?;
        Ok(Self {
            slot,
            connection: Some(connection),
            state,
        })
    }

    pub(crate) fn connection(&self) -> Result<Database<'_>, Failure> {
        self.connection
            .as_ref()
            .map(Held::database)
            .ok_or_else(|| Failure::Internal("the operation has no connection".to_owned()))
    }

    pub(crate) fn connection_mut(&mut self) -> Result<&mut Held, Failure> {
        self.connection
            .as_mut()
            .ok_or_else(|| Failure::Internal("the operation has no connection".to_owned()))
    }

    /// Put the connection back for the next operation, or return it to the
    /// pool if its JavaScript connection closed meanwhile.
    pub(crate) fn restore(mut self) {
        if self.state.ensure_open().is_ok() {
            *self.slot = self.connection.take();
        }
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            self.state.closed.cancel();
            connection.discard();
        }
    }
}

fn pool<'cx>(cx: &mut FunctionContext<'cx>) -> NeonResult<Lender> {
    Ok(cx.argument::<JsBox<PoolHandle>>(0)?.0.clone())
}

fn connection<'cx>(cx: &mut FunctionContext<'cx>) -> NeonResult<Arc<ConnectionState>> {
    Ok(cx.argument::<JsBox<ConnectionHandle>>(0)?.0.clone())
}

/// `poolClose(pool)`: refuse new work, cancel what runs, and resolve once
/// every lent connection is released.
// [spec:pgorm:req:bindings.connections+3]
fn pool_close(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = pool(&mut cx)?;
    let connections = state.close_now();
    let listeners = state.take_listeners();
    settle::promise(
        &mut cx,
        async move {
            for connection in connections {
                drop(connection.slot.lock().await.take());
            }
            for listener in listeners {
                listener.release().await;
            }
            Ok(())
        },
        |cx, ()| Ok(cx.undefined().upcast()),
    )
}

fn pool_closed(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let closed = pool(&mut cx)?.closed.is_cancelled();
    Ok(cx.boolean(closed))
}

/// `poolAcquire(pool, abort)`: a connection of the caller's own, until it
/// closes it.
// [spec:pgorm:req:bindings.connections+3]
fn pool_acquire(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = pool(&mut cx)?;
    let abort = abort_argument(&mut cx, 1)?;
    let work = async move {
        let connection = abort.guard(state.get()).await?;
        let lent = Arc::new(ConnectionState {
            pool: state.state.clone(),
            closed: CancellationToken::new(),
            slot: Arc::new(Mutex::new(Some(connection))),
        });
        let mut registry = state
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.closed.is_cancelled() {
            drop(registry);
            lent.close_now();
            return Err(Failure::Lifecycle("the pool is closed".to_owned()));
        }
        registry.retain(|item| item.strong_count() > 0);
        registry.push(Arc::downgrade(&lent));
        Ok(lent)
    };
    settle::promise(&mut cx, work, |cx, lent| {
        Ok(cx.boxed(ConnectionHandle(lent)).upcast())
    })
}

/// `poolRun(pool, terminal, sql, params, tagged, abort)`: a statement on a
/// connection the pool lends for it alone. Aborted, or cut off by the pool
/// closing, the connection is discarded rather than returned.
// [spec:pgorm:req:bindings.connections+3]
fn pool_run(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = pool(&mut cx)?;
    let statement = Statement::read(&mut cx, 1)?;
    let tagged = cx.argument::<JsBoolean>(4)?.value(&mut cx);
    let abort = abort_argument(&mut cx, 5)?;
    let work = async move {
        on_pool(&state, &abort, async |db| {
            statement.run(&db.database(), &state.secrets).await
        })
        .await
    };
    settle::promise(&mut cx, work, move |cx, outcome| {
        outcome.into_js(cx, tagged)
    })
}

/// Run `work` on a connection the pool lends for it alone, returned to the
/// pool once `work` finishes. Aborted, or cut off by the pool closing, the
/// connection is discarded rather than returned, its state unknown.
pub(crate) async fn on_pool<T, W>(pool: &Lender, abort: &Abort, work: W) -> Result<T, Failure>
where
    W: AsyncFnOnce(&mut Held) -> Result<T, Failure>,
{
    let mut connection = abort.guard(pool.get()).await?;
    let outcome = tokio::select! {
        biased;
        () = abort.token().cancelled() => Err(Failure::Cancelled),
        () = pool.closed.cancelled() => {
            Err(Failure::Lifecycle("the pool closed while the statement ran".to_owned()))
        }
        outcome = work(&mut connection) => {
            drop(connection);
            return outcome;
        }
    };
    connection.discard();
    outcome
}

/// Run `work` on the connection `operation` took from its slot — taken on
/// the JavaScript thread as the operation was called, so operations are
/// refused in the order JavaScript started them — failing when the
/// connection or pool closes or the caller aborts, in which case the
/// connection is discarded.
pub(crate) async fn on_connection<T, W>(
    mut operation: Operation,
    abort: &Abort,
    work: W,
) -> Result<T, Failure>
where
    W: AsyncFnOnce(&mut Held) -> Result<T, Failure>,
{
    let state = operation.state.clone();
    let outcome = tokio::select! {
        biased;
        () = abort.token().cancelled() => return Err(Failure::Cancelled),
        () = state.closed.cancelled() => {
            return Err(Failure::Lifecycle("the connection closed while the statement ran".to_owned()));
        }
        () = state.pool.closed.cancelled() => {
            return Err(Failure::Lifecycle("the pool closed while the statement ran".to_owned()));
        }
        outcome = work(operation.connection_mut()?) => outcome,
    };
    operation.restore();
    outcome
}

/// `connectionRun(connection, terminal, sql, params, tagged, abort)`.
// [spec:pgorm:req:bindings.connections+3]
fn connection_run(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = connection(&mut cx)?;
    let statement = Statement::read(&mut cx, 1)?;
    let tagged = cx.argument::<JsBoolean>(4)?.value(&mut cx);
    let abort = abort_argument(&mut cx, 5)?;
    let secrets = state.pool.secrets.clone();
    let operation = match Operation::begin(state) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move {
        on_connection(operation, &abort, async |db| {
            statement.run(&db.database(), &secrets).await
        })
        .await
    };
    settle::promise(&mut cx, work, move |cx, outcome| {
        outcome.into_js(cx, tagged)
    })
}

/// `connectionPing(connection, abort)`: whether the server answers.
fn connection_ping(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = connection(&mut cx)?;
    let abort = abort_argument(&mut cx, 1)?;
    let secrets = state.pool.secrets.clone();
    let operation = match Operation::begin(state) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move {
        on_connection(operation, &abort, async |db| {
            let row = db
                .database()
                .query_one("SELECT TRUE", &[])
                .await
                .map_err(|error| failure(&error, &secrets))?;
            row.try_get::<_, bool>(0)
                .map_err(|error| failure(&pgorm::Error::Postgres(error), &secrets))
        })
        .await
    };
    settle::promise(&mut cx, work, |cx, alive| Ok(cx.boolean(alive).upcast()))
}

/// `connectionClose(connection)`: return the connection to the pool, or, if
/// a transaction or stream holds it, end that and discard it; resolves once
/// it is released.
// [spec:pgorm:req:bindings.connections+3]
// [spec:pgorm:req:napi.lifecycle+3]
fn connection_close(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = connection(&mut cx)?;
    state.close_now();
    settle::promise(
        &mut cx,
        async move {
            drop(state.slot.lock().await.take());
            Ok(())
        },
        |cx, ()| Ok(cx.undefined().upcast()),
    )
}

/// `connectionDiscard(connection)`: close the connection and discard it, as
/// `DatabaseConnection::discard` does, so the pool opens a fresh one in its
/// place; resolves once it is released.
// [spec:pgorm:req:bindings.connections+3]
fn connection_discard(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = connection(&mut cx)?;
    state.discard_now();
    settle::promise(
        &mut cx,
        async move {
            if let Some(connection) = state.slot.lock().await.take() {
                connection.discard();
            }
            Ok(())
        },
        |cx, ()| Ok(cx.undefined().upcast()),
    )
}

fn connection_closed(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let state = connection(&mut cx)?;
    let closed = state.ensure_open().is_err();
    Ok(cx.boolean(closed))
}
