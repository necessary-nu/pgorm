//! A pool's hooks, each a JavaScript function the JavaScript thread that
//! built the pool runs while pgorm holds the connection the hook is about.
//!
//! The hook itself runs on the runtime, through pgorm-bindings' `hook`,
//! which serves the statements the function sends until it settles. The
//! function is called through the pool's channel, on the JavaScript thread,
//! with the hook's connection and a handle it settles once what it returned
//! has: `lib/connections.js` wraps the program's function so that it does,
//! whether that threw, returned a value or returned a promise.

use std::{
    fmt,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use neon::{prelude::*, types::Finalize};
use pgorm_bindings::{HookCall, HookEvent, HookJobs};
use tokio::sync::oneshot;

use super::Statement;
use crate::{
    errors::{Failure, Redactions},
    settle,
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("hookRun", hook_run)?;
    cx.export_function("hookSettle", hook_settle)?;
    cx.export_function("hookClosed", hook_closed)?;
    cx.export_function("hookAge", hook_age)?;
    cx.export_function("hookRecycleCount", hook_recycle_count)?;
    Ok(())
}

/// The connection a hook runs on, while it runs.
struct HookState {
    jobs: HookJobs,
    secrets: Redactions,
    age: Duration,
    recycle_count: usize,
}

/// The JavaScript `HookConnection`'s native half.
struct HookHandle(Arc<HookState>);

impl fmt::Debug for HookHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HookHandle")
            .field("closed", &self.0.jobs.0.is_closed())
            .finish_non_exhaustive()
    }
}

impl Finalize for HookHandle {}

/// How the function tells its hook it has finished. Collected unsettled, it
/// settles the hook as refused, so no hook waits on a function that dropped
/// it.
struct Settle(Mutex<Option<oneshot::Sender<Result<(), String>>>>);

impl fmt::Debug for Settle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settle").finish_non_exhaustive()
    }
}

impl Finalize for Settle {}

/// A hook's function, rooted, and the channel to the JavaScript thread that
/// built the pool. The channel is unreferenced: it does not keep that
/// thread's event loop alive, which the operation waiting on the connection
/// does while a hook runs.
#[derive(Clone)]
pub(crate) struct Callable {
    function: Arc<Root<JsFunction>>,
    channel: Channel,
}

impl fmt::Debug for Callable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Callable").finish_non_exhaustive()
    }
}

impl Callable {
    /// `function`, called on this thread whenever its hook runs.
    pub(crate) fn new<'cx>(cx: &mut Cx<'cx>, function: Handle<'cx, JsFunction>) -> Self {
        let mut channel = cx.channel();
        channel.unref(cx);
        Self {
            function: Arc::new(function.root(cx)),
            channel,
        }
    }
}

/// The pgorm hook that calls `callable` for `event`. What the function
/// throws or rejects with refuses the connection, its message the refusal's
/// text.
// [spec:pgorm:req:bindings.pool-hooks]
pub(crate) fn hook(event: HookEvent, callable: Callable, secrets: Redactions) -> pgorm::Hook {
    pgorm_bindings::hook(event, move |call: HookCall| {
        let callable = callable.clone();
        let secrets = secrets.clone();
        async move {
            let (settle, settled) = oneshot::channel();
            let state = Arc::new(HookState {
                jobs: call.jobs,
                secrets,
                age: call.age,
                recycle_count: call.recycle_count,
            });
            let function = callable.function.clone();
            callable
                .channel
                .try_send(move |mut cx| {
                    let function = function.to_inner(&mut cx);
                    let connection = cx.boxed(HookHandle(state));
                    let done = cx.boxed(Settle(Mutex::new(Some(settle))));
                    function
                        .call_with(&cx)
                        .arg(connection)
                        .arg(done)
                        .exec(&mut cx)
                })
                .map_err(|_| "the JavaScript thread that built the pool has ended".to_owned())?;
            settled
                .await
                .map_err(|_| "the hook's function ended without settling".to_owned())?
        }
    })
}

fn state(cx: &mut FunctionContext) -> NeonResult<Arc<HookState>> {
    Ok(cx.argument::<JsBox<HookHandle>>(0)?.0.clone())
}

/// `hookRun(connection, terminal, sql, params, tagged)`: a statement on the
/// hook's connection, which the hook serves before it returns.
// [spec:pgorm:req:bindings.pool-hooks]
fn hook_run(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let statement = Statement::read(&mut cx, 1)?;
    let tagged = cx.argument::<JsBoolean>(4)?.value(&mut cx);
    let work = async move {
        let closed = || {
            Failure::Lifecycle("the hook has returned, so its connection is the pool's".to_owned())
        };
        let (reply, answer) = oneshot::channel();
        let secrets = state.secrets.clone();
        if !state.jobs.submit(move |db| {
            Box::pin(async move {
                drop(reply.send(statement.run(db, &secrets).await));
            })
        }) {
            return Err(closed());
        }
        answer.await.map_err(|_| closed())?
    };
    settle::promise(&mut cx, work, move |cx, outcome| {
        outcome.into_js(cx, tagged)
    })
}

/// `hookSettle(done, message)`: the hook's function has finished, refusing
/// the connection with `message` unless it is `undefined`.
fn hook_settle(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let done = cx.argument::<JsBox<Settle>>(0)?;
    let message = cx.argument_opt(1);
    let outcome = match message {
        Some(value) if !value.is_a::<JsUndefined, _>(&mut cx) => {
            Err(value.to_string(&mut cx)?.value(&mut cx))
        }
        _ => Ok(()),
    };
    if let Some(sender) = done.0.lock().unwrap_or_else(PoisonError::into_inner).take() {
        drop(sender.send(outcome));
    }
    Ok(cx.undefined())
}

fn hook_closed(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let closed = state(&mut cx)?.jobs.0.is_closed();
    Ok(cx.boolean(closed))
}

/// `hookAge(connection)`: milliseconds since the connection opened.
fn hook_age(mut cx: FunctionContext) -> JsResult<JsNumber> {
    let age = state(&mut cx)?.age;
    Ok(cx.number(age.as_secs_f64() * 1000.0))
}

fn hook_recycle_count(mut cx: FunctionContext) -> JsResult<JsNumber> {
    let count = state(&mut cx)?.recycle_count;
    #[allow(clippy::cast_precision_loss)]
    Ok(cx.number(count as f64))
}
