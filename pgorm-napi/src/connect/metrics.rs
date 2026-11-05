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

use std::{
    fmt,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use neon::{event::Channel, prelude::*, types::JsBigInt};
use pgorm::{
    Error,
    metric::{MetricsCollector, QueryContext},
};
use tokio::sync::oneshot;

use crate::errors::{Failure, Redactions, failure};

/// The seven hooks, in `MetricsCollector`'s order, as JavaScript names them.
pub(crate) const HOOKS: [&str; 7] = [
    "recordQuerySuccess",
    "recordQueryError",
    "recordConnectionAcquired",
    "recordConnectionError",
    "recordTransactionBegin",
    "recordTransactionCommit",
    "recordTransactionRollback",
];

const QUERY_SUCCESS: usize = 0;
const QUERY_ERROR: usize = 1;
const CONNECTION_ACQUIRED: usize = 2;
const CONNECTION_ERROR: usize = 3;
const TRANSACTION_BEGIN: usize = 4;
const TRANSACTION_COMMIT: usize = 5;
const TRANSACTION_ROLLBACK: usize = 6;

/// The collector an instrumented JavaScript pool hands pgorm's wrappers:
/// each hook they call is handed on to the JavaScript object's.
// [spec:pgorm:req:bindings.metrics]
#[derive(Clone)]
pub(crate) struct Metrics(Arc<Collector>);

struct Collector {
    /// The instance's shared channel, unreferenced, so that a pool holding it
    /// keeps no event loop alive.
    channel: Channel,
    /// `dispatch` in `lib/metrics.js`, which runs one hook and calls back
    /// once it has finished.
    dispatch: Root<JsFunction>,
    /// Each hook, or `None` for one the collector left as `NoOpMetrics`
    /// wrote it, which is never sent.
    hooks: [Option<Root<JsFunction>>; 7],
    secrets: Redactions,
}

impl fmt::Debug for Metrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reported: Vec<&str> = HOOKS
            .iter()
            .zip(&self.0.hooks)
            .filter_map(|(name, hook)| hook.as_ref().map(|_| *name))
            .collect();
        f.debug_struct("Metrics")
            .field("reported", &reported)
            .finish_non_exhaustive()
    }
}

/// What a hook is told, owned until the JavaScript thread makes it values.
enum Report {
    Query {
        operation: String,
        sql: Option<String>,
        duration: Duration,
        outcome: Result<Option<u64>, Failure>,
    },
    Duration(Duration),
    Failed(Duration, Failure),
}

fn milliseconds<'cx>(cx: &mut Cx<'cx>, duration: Duration) -> Handle<'cx, JsValue> {
    cx.number(duration.as_secs_f64() * 1000.0).upcast()
}

impl Report {
    /// The query a query hook is told of, as `[operation, sql]`, or `null`,
    /// and the arguments after it.
    fn into_js<'cx>(
        self,
        cx: &mut Cx<'cx>,
    ) -> NeonResult<(Handle<'cx, JsValue>, Vec<Handle<'cx, JsValue>>)> {
        Ok(match self {
            Self::Query {
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
            Self::Duration(duration) => (cx.null().upcast(), vec![milliseconds(cx, duration)]),
            Self::Failed(duration, failure) => {
                let duration = milliseconds(cx, duration);
                let error = failure.into_js(cx)?;
                (cx.null().upcast(), vec![duration, error])
            }
        })
    }
}

impl Metrics {
    /// A collector over `hooks`, each run by `dispatch`, both rooted here.
    pub(crate) fn new<'cx>(
        cx: &mut impl Context<'cx>,
        dispatch: Handle<'cx, JsFunction>,
        hooks: [Option<Handle<'cx, JsFunction>>; 7],
        secrets: Redactions,
    ) -> Self {
        let mut channel = cx.channel();
        channel.unref(cx);
        let dispatch = dispatch.root(cx);
        let hooks = hooks.map(|hook| hook.map(|hook| hook.root(cx)));
        Self(Arc::new(Collector {
            channel,
            dispatch,
            hooks,
            secrets,
        }))
    }

    /// Send hook `index` to the JavaScript thread with `report`, and wait
    /// for its answer. A hook that cannot be sent — the instance torn down —
    /// is not waited for, and a hook's own failure is the dispatcher's to
    /// report, so nothing here fails the statement.
    async fn report(&self, index: usize, report: Report) {
        if self.0.hooks[index].is_none() {
            return;
        }
        let (answer, answered) = oneshot::channel::<()>();
        let collector = self.0.clone();
        let sent = self.0.channel.try_send(move |mut cx| {
            let Some(hook) = &collector.hooks[index] else {
                return Ok(());
            };
            let hook = hook.to_inner(&mut cx);
            let dispatch = collector.dispatch.to_inner(&mut cx);
            let (query, args) = report.into_js(&mut cx)?;
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
            let name = cx.string(HOOKS[index]);
            dispatch
                .bind(&mut cx)
                .arg(name)?
                .arg(hook)?
                .arg(query)?
                .arg(list)?
                .arg(done)?
                .exec()?;
            Ok(())
        });
        if sent.is_ok() {
            let _ = answered.await;
        }
    }

    fn query(
        query: QueryContext<'_>,
        duration: Duration,
        outcome: Result<Option<u64>, Failure>,
    ) -> Report {
        Report::Query {
            operation: query.operation().to_owned(),
            sql: query.sql().map(str::to_owned),
            duration,
            outcome,
        }
    }
}

#[pgorm::entity::prelude::async_trait::async_trait]
impl MetricsCollector for Metrics {
    async fn record_query_success(
        &self,
        query: QueryContext<'_>,
        duration: Duration,
        rows: Option<u64>,
    ) {
        let report = Self::query(query, duration, Ok(rows));
        self.report(QUERY_SUCCESS, report).await;
    }

    async fn record_query_error(&self, query: QueryContext<'_>, duration: Duration, error: &Error) {
        let report = Self::query(query, duration, Err(failure(error, &self.0.secrets)));
        self.report(QUERY_ERROR, report).await;
    }

    async fn record_connection_acquired(&self, duration: Duration) {
        self.report(CONNECTION_ACQUIRED, Report::Duration(duration))
            .await;
    }

    async fn record_connection_error(&self, duration: Duration, error: &Error) {
        let report = Report::Failed(duration, failure(error, &self.0.secrets));
        self.report(CONNECTION_ERROR, report).await;
    }

    async fn record_transaction_begin(&self, duration: Duration) {
        self.report(TRANSACTION_BEGIN, Report::Duration(duration))
            .await;
    }

    async fn record_transaction_commit(&self, duration: Duration) {
        self.report(TRANSACTION_COMMIT, Report::Duration(duration))
            .await;
    }

    async fn record_transaction_rollback(&self, duration: Duration) {
        self.report(TRANSACTION_ROLLBACK, Report::Duration(duration))
            .await;
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
