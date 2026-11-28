//! How an operation's outcome crosses from the runtime back to JavaScript.

use crate::errors::FailureJs as _;
use std::{future::Future, panic::AssertUnwindSafe};

use futures_util::FutureExt as _;
use neon::prelude::*;

use crate::{errors::Failure, runtime::runtime};

/// Start `work` on the runtime and return the promise its outcome settles,
/// `resolve` turning a success into the value the promise resolves with.
///
/// The promise is settled by a closure sent through the instance's shared
/// channel, so on the JavaScript thread of the instance that started the work.
/// The channel handle the work holds keeps that thread's event loop alive
/// while the work is in flight, and releases it once the settling closure has
/// been queued. Work that panics rejects with an `InternalError`, so no
/// promise is left pending by a failure on the runtime: the panic is caught
/// around the work alone, so the task still holds the `Deferred` and settles
/// it. Nothing of the caller's is held across the unwind but what the work
/// owns, which the unwind drops, so asserting it unwind-safe observes no
/// broken invariant.
// [spec:pgorm:req:napi.promises]
pub(crate) fn promise<'cx, T, W, R>(
    cx: &mut FunctionContext<'cx>,
    work: W,
    resolve: R,
) -> JsResult<'cx, JsPromise>
where
    T: Send + 'static,
    W: Future<Output = Result<T, Failure>> + Send + 'static,
    R: for<'a> FnOnce(&mut Cx<'a>, T) -> JsResult<'a, JsValue> + Send + 'static,
{
    let runtime = runtime(cx)?;
    let channel = cx.channel();
    let (deferred, promise) = cx.promise();
    runtime.spawn(async move {
        let outcome = AssertUnwindSafe(work)
            .catch_unwind()
            .await
            .unwrap_or_else(|payload| Err(pgorm_bindings::panicked(payload.as_ref())));
        // A send fails only once the instance is being torn down — the
        // process exiting or its worker terminated — when no JavaScript is
        // left to settle the promise for, and the outcome is dropped.
        // [spec:pgorm:req:napi.exit]
        let _ = deferred.try_settle_with(&channel, move |mut cx| match outcome {
            Ok(value) => resolve(&mut cx, value),
            Err(failure) => {
                let error = failure.into_js(&mut cx)?;
                cx.throw(error)
            }
        });
    });
    Ok(promise)
}

/// A promise already rejected with `failure`, for an operation refused
/// before any work could start.
pub(crate) fn rejected<'cx>(
    cx: &mut FunctionContext<'cx>,
    failure: Failure,
) -> JsResult<'cx, JsPromise> {
    let (deferred, promise) = cx.promise();
    let error = failure.into_js(cx)?;
    deferred.reject(cx, error);
    Ok(promise)
}
