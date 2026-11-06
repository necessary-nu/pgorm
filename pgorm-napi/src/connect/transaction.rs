//! Transactions and savepoints. pgorm opens a transaction only around a
//! closure, whose `DatabaseTransaction` borrows its connection, and a
//! savepoint's its parent, exclusively; that borrow cannot cross into
//! JavaScript, so a task on the runtime serves each frame's commands inside
//! its closure, nested savepoint frames inside their parent's, and
//! JavaScript's handles send it owned commands. While a savepoint is open its
//! parent's frame is suspended inside it, holding the parent's lock: the
//! parent refuses a statement, a commit, a rollback or another savepoint until
//! the child finishes, and refuses a second request while one runs, rather
//! than queue or race it.

use std::sync::Arc;

use neon::{prelude::*, types::Finalize};
use pgorm::{IsolationLevel, TransactionMode};
use tokio::sync::{OwnedMutexGuard, oneshot};

use pgorm_bindings::transaction::{self, Flow, Job, Tx};

use super::{
    Abort, ConnectionHandle, Operation, Statement, abort_argument,
    copy::{self, CopyInHandle, CopyOutHandle, InState, OutState, Release, Settle},
    job::taken,
};
use crate::{settle, statements::copy_at};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("connectionBegin", connection_begin)?;
    cx.export_function("transactionRun", transaction_run)?;
    cx.export_function("transactionBegin", transaction_begin)?;
    cx.export_function("transactionFinish", transaction_finish)?;
    cx.export_function("transactionAbort", transaction_abort)?;
    cx.export_function("transactionWaitClosed", transaction_wait_closed)?;
    cx.export_function("transactionClosed", transaction_closed)?;
    cx.export_function("transactionJob", transaction_job)?;
    cx.export_function("transactionCopyIn", transaction_copy_in)?;
    cx.export_function("transactionCopyOut", transaction_copy_out)?;
    cx.export_function("transactionCopyColumns", transaction_copy_columns)?;
    cx.export_function("transactionStream", stream::transaction_stream)?;
    cx.export_function("transactionFinishAttempt", transaction_finish_attempt)?;
    retry::export(cx)?;
    Ok(())
}

mod retry;
mod stream;

/// One transaction or savepoint, as its JavaScript handle reaches it.
pub(crate) type State = pgorm_bindings::transaction::State;

/// The JavaScript `Transaction`'s native half. Collected unfinished, it rolls
/// back as a backstop; `commit`, `rollback` and `close` are what end it.
#[derive(Debug)]
pub(crate) struct TransactionHandle(Arc<State>);

impl Finalize for TransactionHandle {}

impl Drop for TransactionHandle {
    fn drop(&mut self) {
        self.0.abandon();
    }
}

fn state(cx: &mut FunctionContext) -> NeonResult<Arc<State>> {
    Ok(cx.argument::<JsBox<TransactionHandle>>(0)?.0.clone())
}

/// A transaction's mode and isolation level, named as JavaScript names them.
fn mode(cx: &mut FunctionContext, at: usize) -> NeonResult<TransactionMode> {
    let mode = cx.argument::<JsString>(at)?.value(cx);
    let isolation = cx.argument::<JsValue>(at + 1)?;
    let isolation = if isolation.is_a::<JsNull, _>(cx) || isolation.is_a::<JsUndefined, _>(cx) {
        None
    } else {
        let name = isolation.downcast_or_throw::<JsString, _>(cx)?.value(cx);
        Some(match name.as_str() {
            "readUncommitted" => IsolationLevel::ReadUncommitted,
            "readCommitted" => IsolationLevel::ReadCommitted,
            "repeatableRead" => IsolationLevel::RepeatableRead,
            "serializable" => IsolationLevel::Serializable,
            _ => {
                return cx.throw_type_error(
                    "isolation is \"readUncommitted\", \"readCommitted\", \"repeatableRead\" or \
                     \"serializable\"",
                );
            }
        })
    };
    match (mode.as_str(), isolation) {
        ("default", None) => Ok(TransactionMode::Default),
        ("readWrite", isolation) => Ok(TransactionMode::ReadWrite { isolation }),
        ("readOnly", isolation) => Ok(TransactionMode::ReadOnly { isolation }),
        ("deferrable", None) => Ok(TransactionMode::DeferrableSnapshot),
        ("default" | "deferrable", Some(_)) => crate::values::read::refuse(
            cx,
            "the default and deferrable modes take no isolation level: choose readWrite or readOnly",
        ),
        _ => cx
            .throw_type_error("mode is \"default\", \"readWrite\", \"readOnly\" or \"deferrable\""),
    }
}

/// `connectionBegin(connection, mode, isolation, abort)`.
fn connection_begin(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let connection = cx.argument::<JsBox<ConnectionHandle>>(0)?.0.clone();
    let mode = mode(&mut cx, 1)?;
    let abort = abort_argument(&mut cx, 3)?;
    let operation = match Operation::begin(connection) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move { transaction::begin(operation, mode, abort.token()).await };
    settle::promise(&mut cx, work, |cx, state| {
        Ok(cx.boxed(TransactionHandle(state)).upcast())
    })
}

/// `transactionRun(transaction, terminal, sql, params, tagged, abort)`.
fn transaction_run(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let statement = Statement::read(&mut cx, 1)?;
    let tagged = cx.argument::<JsBoolean>(4)?.value(&mut cx);
    let abort = abort_argument(&mut cx, 5)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let secrets = state.connection().pool.secrets.clone();
    let work = async move {
        state
            .request(
                lock,
                move |db| Box::pin(async move { statement.run(&db, &secrets).await }),
                abort.token(),
            )
            .await
    };
    settle::promise(&mut cx, work, move |cx, outcome| {
        outcome.into_js(cx, tagged)
    })
}

/// `transactionBegin(transaction, abort)`: a savepoint.
fn transaction_begin(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let abort = abort_argument(&mut cx, 1)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move { state.begin(lock, abort.token()).await };
    settle::promise(&mut cx, work, |cx, child| {
        Ok(cx.boxed(TransactionHandle(child)).upcast())
    })
}

/// `transactionFinish(transaction, commit)`: commit or roll back, resolving
/// once the parent — the connection, or the savepoint's transaction — is
/// free again.
fn transaction_finish(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let commit = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move { state.finish(lock, commit).await };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

/// `transactionAbort(transaction)`: end the whole transaction now,
/// discarding its connection, and resolve once its task has ended.
fn transaction_abort(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    state.abort();
    let work = async move {
        state.finished().await;
        Ok(())
    };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

/// `transactionWaitClosed(transaction)`: resolve once the frame has
/// released its parent.
fn transaction_wait_closed(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let work = async move {
        state.wait_closed().await;
        Ok(())
    };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

fn transaction_closed(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let closed = state(&mut cx)?.closed();
    Ok(cx.boolean(closed))
}

/// `transactionJob(transaction, job, abort)`: a registered entity's job in
/// the transaction, holding its lock as a statement does.
// [spec:pgorm:req:bindings.entities+6]
// [spec:pgorm:req:bindings.registered-pools]
fn transaction_job(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let job = match taken(&mut cx, 1)? {
        Ok(job) => job,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let abort = abort_argument(&mut cx, 2)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let secrets = state.connection().pool.secrets.clone();
    let work = async move {
        state
            .request(
                lock,
                move |db| Box::pin(async move { job(db, &secrets).await }),
                abort.token(),
            )
            .await
    };
    settle::promise(&mut cx, work, |cx, output| output.into_js(cx))
}

/// `transactionCopyColumns(transaction, copy, abort)`: a COPY's columns,
/// described in the transaction, holding its lock as a statement does; a
/// refused description fails the transaction as a refused statement does.
// [spec:pgorm:req:bindings.copy-rows+2]
fn transaction_copy_columns(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let copy = Box::new(copy_at(&mut cx, 1)?);
    let abort = abort_argument(&mut cx, 2)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let secrets = state.connection().pool.secrets.clone();
    let work = async move {
        state
            .request_on(
                lock,
                move |tx| {
                    Box::pin(async move {
                        match tx {
                            Tx::Plain(tx) => copy::described(&mut **tx, &copy, &secrets).await,
                            Tx::Metered(tx) => copy::described(&mut **tx, &copy, &secrets).await,
                        }
                    })
                },
                abort.token(),
            )
            .await
    };
    settle::promise(&mut cx, work, copy::columns_to_js)
}

/// `transactionCopyIn(transaction, copy, abort, rows)`: a COPY FROM in the
/// transaction, holding its lock as a statement does until the COPY ends.
// [spec:pgorm:req:bindings.copy]
fn transaction_copy_in(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let (copy, rows) = copy::copy_from_rows_at(&mut cx, 1, 3)?;
    let copy = Box::new(copy);
    let abort = abort_argument(&mut cx, 2)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let (shared, link, release) = InState::channel(rows);
    let (opened, receive) = oneshot::channel();
    let stop = copy_stop(&state, abort);
    let secrets = state.connection().pool.secrets.clone();
    let frame = state.clone();
    let job: Job = Box::new(move |tx| {
        Box::pin(async move {
            let settle = match tx {
                Tx::Plain(tx) => {
                    copy::copy_in(&mut **tx, &copy, link, opened, &stop, &secrets).await
                }
                Tx::Metered(tx) => {
                    copy::copy_in(&mut **tx, &copy, link, opened, &stop, &secrets).await
                }
            };
            after_copy(&frame, settle, lock, release)
        })
    });
    copy::opened(&mut cx, shared, state.send_job(job), receive, CopyInHandle)
}

/// `transactionCopyOut(transaction, copy, abort, rows)`: a COPY TO in the
/// transaction, which refuses other work until the stream ends or closes.
// [spec:pgorm:req:bindings.copy]
fn transaction_copy_out(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let (copy, rows) = copy::copy_to_rows_at(&mut cx, 1, 3)?;
    let copy = Box::new(copy);
    let abort = abort_argument(&mut cx, 2)?;
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let (shared, link, release) = OutState::channel(rows);
    let (opened, receive) = oneshot::channel();
    let stop = copy_stop(&state, abort);
    let secrets = state.connection().pool.secrets.clone();
    let frame = state.clone();
    let job: Job = Box::new(move |tx| {
        Box::pin(async move {
            let settle = match tx {
                Tx::Plain(tx) => {
                    copy::copy_out(&mut **tx, &copy, link, opened, &stop, &secrets).await
                }
                Tx::Metered(tx) => {
                    copy::copy_out(&mut **tx, &copy, link, opened, &stop, &secrets).await
                }
            };
            after_copy(&frame, settle, lock, release)
        })
    });
    copy::opened(&mut cx, shared, state.send_job(job), receive, CopyOutHandle)
}

/// What ends a COPY in the frame `state` from outside it.
fn copy_stop(state: &State, abort: Abort) -> copy::Stop {
    let [abandoned, aborted] = state.stops();
    copy::Stop::frame(abort, state.connection(), &abandoned, &aborted)
}

/// Free the frame once a COPY in it has ended: its lock first, so that a
/// statement JavaScript runs as the COPY settles finds it free. A COPY whose
/// connection is discarded ends the whole transaction, nothing in it
/// committed, as an aborted statement does.
// [spec:pgorm:req:bindings.copy]
fn after_copy(state: &State, settle: Settle, lock: OwnedMutexGuard<()>, release: Release) -> Flow {
    drop(lock);
    let flow = state.copy_ended(settle == Settle::Discard);
    drop(release);
    flow
}

/// `transactionFinishAttempt(transaction, commit, retryable)`: end one
/// attempt of a retrying transaction, resolving `true` once pgorm has opened
/// the next attempt and `false` once the transaction has ended.
// [spec:pgorm:req:bindings.transaction-retry]
fn transaction_finish_attempt(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let commit = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    let retryable = cx.argument::<JsBoolean>(2)?.value(&mut cx);
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move { state.finish_attempt(lock, commit, retryable).await };
    settle::promise(&mut cx, work, |cx, again| Ok(cx.boolean(again).upcast()))
}
