//! A transaction run again from `BEGIN` while it fails with a serialization
//! failure or a deadlock: pgorm's `transaction_with_retry`, whose closure is
//! called once per attempt, each attempt a frame the JavaScript callback runs
//! on.
//!
//! Each attempt's callback ends with a commit or a rollback request, whose
//! reply waits on pgorm: `true` once pgorm has rolled the attempt back, or
//! seen its `COMMIT` fail retryably, and opened the next; otherwise the
//! outcome of the whole transaction.
//!
//! What the attempts share is held behind a lock the closure owns a handle
//! to, rather than borrowed by it: a closure pgorm calls once per attempt that
//! borrows from the task around it makes that task's future `Send` for no
//! lifetime the compiler can name, and `tokio::spawn` refuses it.

use std::sync::Arc;

use neon::{prelude::*, types::Finalize};
use pgorm_bindings::transaction::Retrying;

use super::{Abort, ConnectionHandle, Operation, TransactionHandle, abort_argument, mode, settle};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("connectionBeginRetrying", connection_begin_retrying)?;
    cx.export_function("retryingAttempt", retrying_attempt)?;
    cx.export_function("retryingAbort", retrying_abort)?;
    Ok(())
}

/// A retrying transaction as JavaScript holds it. Collected, it ends the
/// whole transaction, whichever attempt it is on.
pub(crate) struct RetryingHandle(Arc<Retrying>);

impl Finalize for RetryingHandle {}

impl Drop for RetryingHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl std::fmt::Debug for RetryingHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetryingHandle").finish_non_exhaustive()
    }
}

/// `connectionBeginRetrying(connection, mode, isolation, retries)`: a
/// retrying transaction on `connection`, its first `BEGIN` sent at once
/// and each attempt handed out by `retryingAttempt`.
fn connection_begin_retrying(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let connection = cx.argument::<JsBox<ConnectionHandle>>(0)?.0.clone();
    let mode = mode(&mut cx, 1)?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let retries = cx.argument::<JsNumber>(3)?.value(&mut cx) as u32;
    let operation = match Operation::begin(connection) {
        Ok(operation) => operation,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move { Ok(Retrying::begin(operation, mode, retries)) };
    settle::promise(&mut cx, work, |cx, state| {
        Ok(cx.boxed(RetryingHandle(state)).upcast())
    })
}

/// `retryingAttempt(retrying, abort)`: the next attempt's transaction, once
/// pgorm has opened it.
fn retrying_attempt(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = cx.argument::<JsBox<RetryingHandle>>(0)?.0.clone();
    let abort: Abort = abort_argument(&mut cx, 1)?;
    let work = async move { state.attempt(abort.token()).await };
    settle::promise(&mut cx, work, |cx, attempt| {
        Ok(cx.boxed(TransactionHandle(attempt)).upcast())
    })
}

/// `retryingAbort(retrying)`: end the whole transaction, whichever attempt
/// it is on, discarding its connection.
fn retrying_abort(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    cx.argument::<JsBox<RetryingHandle>>(0)?.0.abort();
    Ok(cx.undefined())
}
