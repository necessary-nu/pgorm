//! Work a registered entity or graph runs through pgorm's own terminals —
//! `Select::all`, `ActiveModel::insert`, `Cursor::all` — on whichever
//! executor the JavaScript handle holds: a connection a pool lends for the
//! one job, a connection the caller holds, or a transaction or savepoint.
//! The terminals are generic over `ConnectionTrait`, so a job takes the one
//! [`Database`] that is either, and the operation around it — the busy lock,
//! the abort, the discard of a connection whose state the abort left unknown
//! — is the one a statement of SQL text gets.

use std::sync::{Mutex as SyncMutex, PoisonError};

use futures_util::future::BoxFuture;
use neon::{prelude::*, types::Finalize};
use pgorm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, Error, SqlText,
    metric::{InstrumentedConnection, InstrumentedTransaction},
};
use tokio_postgres::{
    Row, RowStream,
    types::{BorrowToSql, ToSql},
};

use super::{
    Operation, abort_argument, connection, metrics::Metrics, on_connection, on_pool, pool,
};
use crate::{
    errors::{Failure, Redactions},
    settle,
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("poolJob", pool_job)?;
    cx.export_function("connectionJob", connection_job)?;
    Ok(())
}

/// The executor a job runs on: a connection, or a transaction on one,
/// plain or, from an instrumented pool, pgorm's metrics wrapper around one.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Database<'a> {
    Connection(&'a DatabaseConnection),
    Transaction(&'a DatabaseTransaction<'a>),
    MeteredConnection(&'a InstrumentedConnection<Metrics>),
    MeteredTransaction(&'a InstrumentedTransaction<'a, Metrics>),
}

macro_rules! delegate {
    ($db:expr, $method:ident $(, $arg:expr)*) => {
        match $db {
            Database::Connection(connection) => connection.$method($($arg),*).await,
            Database::Transaction(transaction) => transaction.$method($($arg),*).await,
            Database::MeteredConnection(connection) => connection.$method($($arg),*).await,
            Database::MeteredTransaction(transaction) => transaction.$method($($arg),*).await,
        }
    };
}

#[pgorm::entity::prelude::async_trait::async_trait]
impl ConnectionTrait for Database<'_> {
    async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        delegate!(self, execute, statement, params)
    }

    async fn execute_raw<T, P, I>(&self, statement: &T, params: I) -> Result<u64, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        delegate!(self, execute_raw, statement, params)
    }

    async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        delegate!(self, query_one, statement, params)
    }

    async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        delegate!(self, query_opt, statement, params)
    }

    async fn query_all<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + SqlText + Sync,
    {
        delegate!(self, query_all, statement, params)
    }

    async fn query_raw<T, P, I>(&self, statement: &T, params: I) -> Result<RowStream, Error>
    where
        T: ?Sized + SqlText + Sync,
        P: BorrowToSql,
        I: IntoIterator<Item = P> + Send,
        I::IntoIter: ExactSizeIterator,
    {
        delegate!(self, query_raw, statement, params)
    }

    async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        delegate!(self, batch_execute, sql)
    }
}

/// What a job hands back, made into JavaScript on the JavaScript thread.
pub(crate) trait Settled: Send {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue>;
}

pub(crate) type Output = Box<dyn Settled>;

/// One run of pgorm's terminals on an executor, its failures redacted by the
/// pool's secrets.
pub(crate) type Job = Box<
    dyn for<'a> FnOnce(Database<'a>, &'a Redactions) -> BoxFuture<'a, Result<Output, Failure>>
        + Send,
>;

/// A job as JavaScript holds it until a terminal runs it, once.
pub(crate) struct JobHandle(SyncMutex<Option<Job>>);

impl Finalize for JobHandle {}

impl std::fmt::Debug for JobHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobHandle").finish_non_exhaustive()
    }
}

impl JobHandle {
    pub(crate) fn new(job: Job) -> Self {
        Self(SyncMutex::new(Some(job)))
    }
}

/// The job at `index`, taken out of its handle.
pub(super) fn taken(cx: &mut FunctionContext, index: usize) -> NeonResult<Result<Job, Failure>> {
    let handle = cx.argument::<JsBox<JobHandle>>(index)?;
    let job = handle
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    Ok(job.ok_or_else(|| Failure::Internal("the job has already run".to_owned())))
}

/// `poolJob(pool, job, abort)`: the job on a connection the pool lends for it
/// alone, discarded rather than returned if the job is cut off.
fn pool_job(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = pool(&mut cx)?;
    let job = match taken(&mut cx, 1)? {
        Ok(job) => job,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let abort = abort_argument(&mut cx, 2)?;
    let work = async move {
        on_pool(&state, &abort, async |db| {
            job(db.database(), &state.secrets).await
        })
        .await
    };
    settle::promise(&mut cx, work, |cx, output| output.into_js(cx))
}

/// `connectionJob(connection, job, abort)`.
fn connection_job(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = connection(&mut cx)?;
    let job = match taken(&mut cx, 1)? {
        Ok(job) => job,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let abort = abort_argument(&mut cx, 2)?;
    let secrets = state.pool.secrets.clone();
    let operation = match Operation::begin(state) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move {
        on_connection(operation, &abort, async |db| {
            job(db.database(), &secrets).await
        })
        .await
    };
    settle::promise(&mut cx, work, |cx, output| output.into_js(cx))
}
