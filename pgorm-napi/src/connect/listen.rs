//! LISTEN and NOTIFY: a pool's dedicated listening connection, whose
//! notifications JavaScript pulls one at a time, and `pg_notify` sent from a
//! pool's connection, a connection or a transaction.
//!
//! A listener is pgorm's pair, a `Listener` that changes the connection's
//! `LISTEN` set and the `Notifications` stream, both held by the native
//! handle. The connection is the listener's alone, opened with the pool's
//! configuration and closed with the listener or its pool; it is never one the
//! pool lends.

use std::{
    fmt,
    num::NonZeroUsize,
    sync::{Arc, PoisonError},
};

use futures_util::{FutureExt, StreamExt, future::BoxFuture};
use neon::{prelude::*, types::Finalize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::{
    Abort, PoolState, abort_argument,
    job::{Job, JobHandle, Output, Settled},
    pool,
};
use crate::{
    errors::{Failure, failure},
    settle,
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("poolListener", pool_listener)?;
    cx.export_function("listenerSend", listener_send)?;
    cx.export_function("listenerNext", listener_next)?;
    cx.export_function("listenerClose", listener_close)?;
    cx.export_function("listenerClosed", listener_closed)?;
    cx.export_function("notifyJob", notify_job)?;
    Ok(())
}

/// A listener: its `LISTEN` set, and its notifications until it closes.
pub(crate) struct ListenerState {
    pool: Arc<PoolState>,
    listener: pgorm::Listener,
    /// `None` once closed: dropping the stream closes the connection.
    notifications: Arc<Mutex<Option<pgorm::Notifications>>>,
    closed: CancellationToken,
}

impl fmt::Debug for ListenerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListenerState")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl ListenerState {
    fn is_closed(&self) -> bool {
        self.closed.is_cancelled() || self.pool.closed.is_cancelled()
    }

    fn ensure_open(&self) -> Result<(), Failure> {
        self.pool.ensure_open()?;
        if self.closed.is_cancelled() {
            Err(Failure::Lifecycle("the listener is closed".to_owned()))
        } else {
            Ok(())
        }
    }

    /// Resolves once the listener or its pool closes.
    async fn closing(&self) {
        tokio::select! {
            () = self.closed.cancelled() => {}
            () = self.pool.closed.cancelled() => {}
        }
    }

    /// Close at once, dropping the stream unless a pull holds it; that pull
    /// sees the close and drops it.
    pub(super) fn close_now(&self) {
        self.closed.cancel();
        if let Ok(mut slot) = self.notifications.try_lock() {
            drop(slot.take());
        }
    }

    /// Close and resolve once the stream, and with it the connection, is
    /// released.
    pub(super) async fn release(&self) {
        self.closed.cancel();
        drop(self.notifications.lock().await.take());
    }
}

/// The JavaScript `Listener`'s native half. Collected unclosed, it closes the
/// listener's connection; a pull in flight keeps the JavaScript object, and
/// so this handle, reachable.
#[derive(Debug)]
pub(crate) struct ListenerHandle(Arc<ListenerState>);

impl Finalize for ListenerHandle {}

impl Drop for ListenerHandle {
    fn drop(&mut self) {
        self.0.close_now();
    }
}

fn listener(cx: &mut FunctionContext) -> NeonResult<Arc<ListenerState>> {
    Ok(cx.argument::<JsBox<ListenerHandle>>(0)?.0.clone())
}

/// `poolListener(pool, capacity, abort)`: a listener on a connection of its
/// own, opened with the pool's configuration, holding at most `capacity`
/// notifications unread.
// [spec:pgorm:req:bindings.listen+2]
fn pool_listener(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = pool(&mut cx)?;
    let capacity = cx.argument::<JsNumber>(1)?.value(&mut cx);
    let abort = abort_argument(&mut cx, 2)?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let Some(capacity) = NonZeroUsize::new(capacity as usize) else {
        return settle::rejected(
            &mut cx,
            Failure::Construction("capacity is a positive integer".to_owned()),
        );
    };
    let work = async move {
        state.ensure_open()?;
        let (listener, notifications) = abort
            .guard(async {
                tokio::select! {
                    () = state.closed.cancelled() => {
                        Err(Failure::Lifecycle("the pool is closed".to_owned()))
                    }
                    opened = state.pool.listener(capacity) => {
                        opened.map_err(|error| failure(&error, &state.secrets))
                    }
                }
            })
            .await?;
        let listening = Arc::new(ListenerState {
            pool: state.state.clone(),
            listener,
            notifications: Arc::new(Mutex::new(Some(notifications))),
            closed: CancellationToken::new(),
        });
        let mut registry = state
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.closed.is_cancelled() {
            drop(registry);
            listening.close_now();
            return Err(Failure::Lifecycle("the pool is closed".to_owned()));
        }
        registry.retain(|item| item.strong_count() > 0);
        registry.push(Arc::downgrade(&listening));
        Ok(listening)
    };
    settle::promise(&mut cx, work, |cx, listening| {
        Ok(cx.boxed(ListenerHandle(listening)).upcast())
    })
}

/// The statement `listenerSend` sends.
#[derive(Debug)]
enum Verb {
    Listen(String),
    Unlisten(String),
    UnlistenAll,
}

/// `listenerSend(listener, verb, channel, abort)`: `LISTEN`, `UNLISTEN` or
/// `UNLISTEN *`. An abort closes the listener: the statement may have
/// reached the server, leaving the channels it listens on unknown.
// [spec:pgorm:req:bindings.listen+2]
// [spec:pgorm:req:bindings.cancellation]
fn listener_send(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = listener(&mut cx)?;
    let verb = cx.argument::<JsString>(1)?.value(&mut cx);
    let channel = cx.argument::<JsString>(2)?.value(&mut cx);
    let abort = abort_argument(&mut cx, 3)?;
    let verb = match verb.as_str() {
        "listen" => Verb::Listen(channel),
        "unlisten" => Verb::Unlisten(channel),
        "unlistenAll" => Verb::UnlistenAll,
        _ => return cx.throw_type_error("unknown listener statement"),
    };
    let work = async move {
        state.ensure_open()?;
        let sent = async {
            match &verb {
                Verb::Listen(channel) => state.listener.listen(channel).await,
                Verb::Unlisten(channel) => state.listener.unlisten(channel).await,
                Verb::UnlistenAll => state.listener.unlisten_all().await,
            }
        };
        tokio::select! {
            biased;
            () = abort.token().cancelled() => {
                state.close_now();
                Err(Failure::Cancelled)
            }
            () = state.closing() => Err(Failure::Lifecycle("the listener is closed".to_owned())),
            sent = sent => sent.map_err(|error| failure(&error, &state.pool.secrets)),
        }
    };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

/// A notification as JavaScript receives it.
#[derive(Debug)]
struct Received {
    channel: String,
    payload: String,
    process_id: i32,
}

/// The next notification, or `None` once the listener has closed: before the
/// pull was made, while it waits, or before the runtime starts it. A pool
/// that closes under a waiting pull fails it. A failure the stream ends with
/// — the connection lost, or its capacity overrun — closes the listener; an
/// abort does not, since the notification it was waiting for stays queued
/// for the next pull.
// [spec:pgorm:req:bindings.listen+2]
async fn next(
    state: Arc<ListenerState>,
    abort: Abort,
    closed_before: bool,
) -> Result<Option<Received>, Failure> {
    if closed_before {
        return Ok(None);
    }
    let ended = |state: &ListenerState| {
        if state.pool.closed.is_cancelled() {
            Err(Failure::Lifecycle(
                "the pool closed while the listener waited".to_owned(),
            ))
        } else {
            Ok(None)
        }
    };
    let mut slot = state.notifications.clone().try_lock_owned().map_err(|_| {
        Failure::Lifecycle("the listener is busy: one notification is pulled at a time".to_owned())
    })?;
    let Some(notifications) = slot.as_mut() else {
        return ended(&state);
    };
    let item = tokio::select! {
        biased;
        () = abort.token().cancelled() => return Err(Failure::Cancelled),
        () = state.pool.closed.cancelled() => {
            drop(slot.take());
            return ended(&state);
        }
        () = state.closed.cancelled() => {
            drop(slot.take());
            return ended(&state);
        }
        item = notifications.next() => item,
    };
    match item {
        Some(Ok(notification)) => Ok(Some(Received {
            channel: notification.channel().to_owned(),
            payload: notification.payload().to_owned(),
            process_id: notification.process_id(),
        })),
        Some(Err(error)) => {
            drop(slot.take());
            state.closed.cancel();
            Err(failure(&error, &state.pool.secrets))
        }
        None => {
            drop(slot.take());
            state.closed.cancel();
            Ok(None)
        }
    }
}

/// `listenerNext(listener, abort)`: `[channel, payload, processId]`, or
/// `null` once the listener is closed.
// [spec:pgorm:req:bindings.listen+2]
fn listener_next(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = listener(&mut cx)?;
    let abort = abort_argument(&mut cx, 1)?;
    let closed_before = state.is_closed();
    settle::promise(
        &mut cx,
        next(state, abort, closed_before),
        |cx, received| {
            let Some(received) = received else {
                return Ok(cx.null().upcast());
            };
            let item = JsArray::new(cx, 3);
            let channel = cx.string(received.channel);
            item.set(cx, 0, channel)?;
            let payload = cx.string(received.payload);
            item.set(cx, 1, payload)?;
            let process_id = cx.number(received.process_id);
            item.set(cx, 2, process_id)?;
            Ok(item.upcast())
        },
    )
}

/// `listenerClose(listener)`: close the listener's connection, ending a pull
/// in flight, and resolve once it is released.
// [spec:pgorm:req:bindings.listen+2]
fn listener_close(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = listener(&mut cx)?;
    state.closed.cancel();
    settle::promise(
        &mut cx,
        async move {
            state.release().await;
            Ok(())
        },
        |cx, ()| Ok(cx.undefined().upcast()),
    )
}

fn listener_closed(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let closed = listener(&mut cx)?.is_closed();
    Ok(cx.boolean(closed))
}

/// What a `notify` job hands back: nothing.
struct Notified;

impl Settled for Notified {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        Ok(cx.undefined().upcast())
    }
}

/// `notifyJob(channel, payload)`: pgorm's `notify` as a job, which the pool,
/// connection or transaction it runs on runs as it runs a statement.
// [spec:pgorm:req:bindings.listen+2]
fn notify_job(mut cx: FunctionContext) -> JsResult<JsBox<JobHandle>> {
    let channel = cx.argument::<JsString>(0)?.value(&mut cx);
    let payload = cx.argument::<JsString>(1)?.value(&mut cx);
    let job: Job = Box::new(
        move |db, secrets| -> BoxFuture<'_, Result<Output, Failure>> {
            async move {
                pgorm::notify(&db, &channel, &payload)
                    .await
                    .map_err(|error| failure(&error, secrets))?;
                Ok(Box::new(Notified) as Output)
            }
            .boxed()
        },
    );
    Ok(cx.boxed(JobHandle::new(job)))
}
