//! LISTEN and NOTIFY: a pool's dedicated listening connection, whose
//! notifications JavaScript pulls one at a time, and `pg_notify` sent from a
//! pool's connection, a connection or a transaction.
//!
//! A listener is pgorm's pair, a `Listener` that changes the connection's
//! `LISTEN` set and the `Notifications` stream, both held by the native
//! handle. The connection is the listener's alone, opened with the pool's
//! configuration and closed with the listener or its pool; it is never one the
//! pool lends.

use crate::errors::FailureJs as _;
use std::{num::NonZeroUsize, sync::Arc};

use futures_util::{FutureExt, future::BoxFuture};
use neon::{prelude::*, types::Finalize};

use pgorm_bindings::ListenRequest;

use super::{
    abort_argument,
    config::{DIALECT, Shared},
    job::{Job, JobHandle, Output, Settled},
    pool,
};
use crate::{
    errors::{Failure, failure},
    settle,
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("poolListener", pool_listener)?;
    cx.export_function("connectListener", connect_listener)?;
    cx.export_function("listenerSend", listener_send)?;
    cx.export_function("listenerNext", listener_next)?;
    cx.export_function("listenerClose", listener_close)?;
    cx.export_function("listenerClosed", listener_closed)?;
    cx.export_function("notifyJob", notify_job)?;
    Ok(())
}

/// A listener: its `LISTEN` set, and its notifications until it closes.
pub(crate) use pgorm_bindings::ListenerState;

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
// [spec:pgorm:req:bindings.listen+3]
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
    let work = async move { ListenerState::on_pool(&state, capacity, abort.token()).await };
    settle::promise(&mut cx, work, |cx, listening| {
        Ok(cx.boxed(ListenerHandle(listening)).upcast())
    })
}

/// `connectListener(dsn, options, capacity, abort)`: a listener on a
/// connection of its own, belonging to no pool, its connection string and
/// TLS options read as a pool's are, every refusal thrown before anything is
/// sent.
// [spec:pgorm:req:bindings.listen+3]
// [spec:pgorm:req:bindings.tls]
fn connect_listener(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let dsn = cx.argument::<JsValue>(0)?;
    let options = cx.argument::<JsObject>(1)?;
    let shared = Shared::read(&mut cx, dsn, options)?;
    let capacity = cx.argument::<JsNumber>(2)?.value(&mut cx);
    let abort = abort_argument(&mut cx, 3)?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let Some(capacity) = NonZeroUsize::new(capacity as usize) else {
        return settle::rejected(
            &mut cx,
            Failure::Construction("capacity is a positive integer".to_owned()),
        );
    };
    let (dsn, tls, connect_timeout) = shared.listener_parts();
    let (configured, connector) =
        match pgorm_bindings::listener_config(dsn, tls, connect_timeout, DIALECT) {
            Ok(parts) => parts,
            Err(refusal) => {
                let error = Failure::Construction(refusal.into_message()).into_js(&mut cx)?;
                return cx.throw(error);
            }
        };
    let work = async move {
        ListenerState::connect(configured, connector, capacity, DIALECT, abort.token()).await
    };
    settle::promise(&mut cx, work, |cx, listening| {
        Ok(cx.boxed(ListenerHandle(listening)).upcast())
    })
}

/// `listenerSend(listener, verb, channel, abort)`: `LISTEN`, `UNLISTEN` or
/// `UNLISTEN *`. An abort closes the listener: the statement may have
/// reached the server, leaving the channels it listens on unknown.
// [spec:pgorm:req:bindings.listen+3]
// [spec:pgorm:req:bindings.cancellation]
fn listener_send(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = listener(&mut cx)?;
    let verb = cx.argument::<JsString>(1)?.value(&mut cx);
    let channel = cx.argument::<JsString>(2)?.value(&mut cx);
    let abort = abort_argument(&mut cx, 3)?;
    let request = match verb.as_str() {
        "listen" => ListenRequest::Listen(channel),
        "unlisten" => ListenRequest::Unlisten(channel),
        "unlistenAll" => ListenRequest::UnlistenAll,
        _ => return cx.throw_type_error("unknown listener statement"),
    };
    let work = async move { state.send(request, abort.token()).await };
    settle::promise(&mut cx, work, |cx, ()| Ok(cx.undefined().upcast()))
}

/// `listenerNext(listener, abort)`: `[channel, payload, processId]`, or
/// `null` once the listener is closed.
// [spec:pgorm:req:bindings.listen+3]
fn listener_next(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = listener(&mut cx)?;
    let abort = abort_argument(&mut cx, 1)?;
    settle::promise(
        &mut cx,
        async move { state.next(abort.token()).await },
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
// [spec:pgorm:req:bindings.listen+3]
fn listener_close(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = listener(&mut cx)?;
    state.close_now();
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
// [spec:pgorm:req:bindings.listen+3]
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
