//! pgorm's metrics layer for a JavaScript pool: an `InstrumentedPool` whose
//! collector is an object of JavaScript functions, each report run on the
//! JavaScript thread.
//!
//! The timing and the choice of hook are pgorm's own: an instrumented pool
//! lends `InstrumentedConnection`s, so every statement, acquisition and
//! transaction is reported by `pgorm::metric`. What lives here is only the
//! crossing. A hook never runs on pgorm's runtime: the runtime sends it to the
//! JavaScript thread through the instance's channel, the one an operation's
//! result is settled through, and awaits its answer, so the statement it
//! reports on settles only once its collector has been told.

use std::sync::{Arc, Mutex, PoisonError};

use futures_util::future::BoxFuture;
use neon::{event::Channel, prelude::*, types::JsBigInt};
use pgorm::metric::QueryContext;
use pgorm_bindings::{Dialect, Hook, Metrics, Redactions, Report, Reporter};
use tokio::sync::oneshot;

use crate::errors::FailureJs as _;

/// The collector an instrumented JavaScript pool hands pgorm's wrappers: each
/// hook in `Hook::ALL`'s order, or `None` for one never to run, each run by
/// `dispatch` on the JavaScript thread.
// [spec:pgorm:req:bindings.metrics]
pub(crate) fn metrics<'cx>(
    cx: &mut impl Context<'cx>,
    dispatch: Handle<'cx, JsFunction>,
    hooks: [Option<Handle<'cx, JsFunction>>; Hook::ALL.len()],
    secrets: Redactions,
) -> Metrics {
    let mut channel = cx.channel();
    channel.unref(cx);
    let dispatch = dispatch.root(cx);
    let hooks = hooks.map(|hook| hook.map(|hook| hook.root(cx)));
    Metrics::new(
        Collector(Arc::new(Hooks {
            channel,
            dispatch,
            hooks,
        })),
        secrets,
        Dialect::JavaScript,
    )
}

struct Hooks {
    /// The instance's shared channel, unreferenced, so that a pool holding it
    /// keeps no event loop alive.
    channel: Channel,
    /// `dispatch` in `lib/metrics.js`, which runs one hook and calls back
    /// once it has finished.
    dispatch: Root<JsFunction>,
    hooks: [Option<Root<JsFunction>>; Hook::ALL.len()],
}

impl Hooks {
    fn hook(&self, hook: Hook) -> Option<&Root<JsFunction>> {
        let index = Hook::ALL.iter().position(|each| *each == hook)?;
        self.hooks[index].as_ref()
    }
}

struct Collector(Arc<Hooks>);

fn milliseconds<'cx>(cx: &mut Cx<'cx>, duration: std::time::Duration) -> Handle<'cx, JsValue> {
    cx.number(duration.as_secs_f64() * 1000.0).upcast()
}

/// The query a query hook is told of, as `[operation, sql]`, or `null`, and
/// the arguments after it.
fn arguments<'cx>(
    cx: &mut Cx<'cx>,
    report: Report,
) -> NeonResult<(Handle<'cx, JsValue>, Vec<Handle<'cx, JsValue>>)> {
    Ok(match report {
        Report::Query {
            operation,
            sql,
            duration,
            outcome,
        } => {
            let query = cx.empty_array();
            let operation = cx.string(operation);
            query.set(cx, 0, operation)?;
            let sql: Handle<JsValue> = match sql {
                Some(sql) => cx.string(sql).upcast(),
                None => cx.null().upcast(),
            };
            query.set(cx, 1, sql)?;
            let duration = milliseconds(cx, duration);
            let last = match outcome {
                #[allow(clippy::cast_precision_loss)]
                Ok(Some(rows)) => cx.number(rows as f64).upcast(),
                Ok(None) => cx.null().upcast(),
                Err(failure) => failure.into_js(cx)?,
            };
            (query.upcast(), vec![duration, last])
        }
        Report::Duration(duration) => (cx.null().upcast(), vec![milliseconds(cx, duration)]),
        Report::Failed(duration, failure) => {
            let duration = milliseconds(cx, duration);
            let error = failure.into_js(cx)?;
            (cx.null().upcast(), vec![duration, error])
        }
    })
}

impl Reporter for Collector {
    fn reports(&self, hook: Hook) -> bool {
        self.0.hook(hook).is_some()
    }

    /// Send the hook to the JavaScript thread and wait for its answer. A
    /// hook that cannot be sent, the instance torn down, is not waited for.
    fn report(&self, hook: Hook, report: Report) -> BoxFuture<'static, ()> {
        let (answer, answered) = oneshot::channel::<()>();
        let hooks = self.0.clone();
        let sent = self.0.channel.try_send(move |mut cx| {
            let Some(callable) = hooks.hook(hook) else {
                return Ok(());
            };
            let callable = callable.to_inner(&mut cx);
            let dispatch = hooks.dispatch.to_inner(&mut cx);
            let (query, args) = arguments(&mut cx, report)?;
            let list = cx.empty_array();
            for (at, arg) in (0_u32..).zip(args) {
                list.set(&mut cx, at, arg)?;
            }
            let answer = Mutex::new(Some(answer));
            let done = JsFunction::new(&mut cx, move |mut cx| {
                if let Some(answer) = answer.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    let _ = answer.send(());
                }
                Ok(cx.undefined())
            })?;
            let name = cx.string(hook.name(Dialect::JavaScript));
            dispatch
                .bind(&mut cx)
                .arg(name)?
                .arg(callable)?
                .arg(query)?
                .arg(list)?
                .arg(done)?
                .exec()?;
            Ok(())
        });
        Box::pin(async move {
            if sent.is_ok() {
                let _ = answered.await;
            }
        })
    }
}

/// `queryFingerprint(operation, sql)`: pgorm's fingerprint of `sql` as a
/// bigint, or `null` when it has none, parsed the first time a process asks
/// for one of this text.
// [spec:pgorm:req:bindings.metrics]
pub(crate) fn query_fingerprint(mut cx: FunctionContext) -> JsResult<JsValue> {
    let operation = cx.argument::<JsString>(0)?.value(&mut cx);
    let sql = cx.argument::<JsValue>(1)?;
    let sql = match sql.downcast::<JsString, _>(&mut cx) {
        Ok(sql) => Some(sql.value(&mut cx)),
        Err(_) => None,
    };
    match QueryContext::new(&operation, sql.as_deref()).fingerprint() {
        Some(fingerprint) => Ok(JsBigInt::from_u64(&mut cx, fingerprint.value()).upcast()),
        None => Ok(cx.null().upcast()),
    }
}
