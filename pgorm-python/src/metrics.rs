//! pgorm's metrics layer for a Python pool: an `InstrumentedPool` whose
//! collector is a Python object, each report run on the pool's event loop.
//!
//! The timing and the choice of hook are pgorm's own: an instrumented pool
//! lends `InstrumentedConnection`s, so every statement, acquisition and
//! transaction is reported by `pgorm::metric`. What lives here is only the
//! crossing. A hook never runs on pgorm's runtime: the runtime queues it on
//! the pool's event loop, as it queues an operation's result, and awaits its
//! answer, so the statement it reports on finishes only once its collector
//! has been told.

use std::{
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};

use pgorm::{
    Error,
    metric::{MetricsCollector, QueryContext, QueryFingerprint},
};
use pyo3::{IntoPyObjectExt, prelude::*, types::PyTuple};
use tokio::sync::oneshot;

use crate::errors::{Redactions, database_error};

/// The seven hooks, in `MetricsCollector`'s order, as Python names them.
pub(crate) const HOOKS: [&str; 7] = [
    "record_query_success",
    "record_query_error",
    "record_connection_acquired",
    "record_connection_error",
    "record_transaction_begin",
    "record_transaction_commit",
    "record_transaction_rollback",
];

const QUERY_SUCCESS: usize = 0;
const QUERY_ERROR: usize = 1;
const CONNECTION_ACQUIRED: usize = 2;
const CONNECTION_ERROR: usize = 3;
const TRANSACTION_BEGIN: usize = 4;
const TRANSACTION_COMMIT: usize = 5;
const TRANSACTION_ROLLBACK: usize = 6;

/// The collector an instrumented Python pool hands pgorm's wrappers: each
/// hook they call is handed on to the Python object's.
// [spec:pgorm:req:bindings.metrics]
#[derive(Clone)]
pub(crate) struct Metrics(Arc<Collector>);

struct Collector {
    /// The loop the pool belongs to, where every hook runs.
    event_loop: Py<PyAny>,
    /// `pgorm.metrics._dispatch`, which runs one hook and answers once it
    /// has finished.
    dispatch: Py<PyAny>,
    /// Each hook's callable, or `None` for one the collector left as
    /// `NoOpMetrics` wrote it, which is never queued.
    hooks: [Option<Py<PyAny>>; 7],
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

impl Metrics {
    pub(crate) fn new(
        event_loop: Py<PyAny>,
        dispatch: Py<PyAny>,
        hooks: [Option<Py<PyAny>>; 7],
        secrets: Redactions,
    ) -> Self {
        Self(Arc::new(Collector {
            event_loop,
            dispatch,
            hooks,
            secrets,
        }))
    }

    /// Queue hook `index` on the pool's event loop with what `args` builds,
    /// and wait for its answer. A hook that cannot be queued — its loop
    /// closed — is not waited for, and a hook's own failure is the
    /// dispatcher's to report, so nothing here fails the statement.
    async fn report<A>(&self, index: usize, args: A)
    where
        A: for<'py> FnOnce(Python<'py>) -> PyResult<Bound<'py, PyTuple>> + Send,
    {
        let Some(hook) = &self.0.hooks[index] else {
            return;
        };
        let (answer, answered) = oneshot::channel();
        let queued = Python::attach(|py| -> PyResult<()> {
            let args = args(py)?;
            let answer = Answer(Mutex::new(Some(answer)));
            self.0.event_loop.bind(py).call_method1(
                "call_soon_threadsafe",
                (
                    self.0.dispatch.bind(py),
                    HOOKS[index],
                    hook.bind(py),
                    args,
                    answer,
                ),
            )?;
            Ok(())
        });
        if queued.is_ok() {
            let _ = answered.await;
        }
    }

    async fn duration(&self, index: usize, duration: Duration) {
        self.report(index, move |py| PyTuple::new(py, [duration.as_secs_f64()]))
            .await;
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
        let context = Context::of(query);
        self.report(QUERY_SUCCESS, move |py| {
            PyTuple::new(
                py,
                [
                    context.into_bound_py_any(py)?,
                    duration.as_secs_f64().into_bound_py_any(py)?,
                    rows.into_bound_py_any(py)?,
                ],
            )
        })
        .await;
    }

    async fn record_query_error(&self, query: QueryContext<'_>, duration: Duration, error: &Error) {
        let context = Context::of(query);
        let error = database_error(error, &self.0.secrets);
        self.report(QUERY_ERROR, move |py| {
            PyTuple::new(
                py,
                [
                    context.into_bound_py_any(py)?,
                    duration.as_secs_f64().into_bound_py_any(py)?,
                    error.into_value(py).into_bound_py_any(py)?,
                ],
            )
        })
        .await;
    }

    async fn record_connection_acquired(&self, duration: Duration) {
        self.duration(CONNECTION_ACQUIRED, duration).await;
    }

    async fn record_connection_error(&self, duration: Duration, error: &Error) {
        let error = database_error(error, &self.0.secrets);
        self.report(CONNECTION_ERROR, move |py| {
            PyTuple::new(
                py,
                [
                    duration.as_secs_f64().into_bound_py_any(py)?,
                    error.into_value(py).into_bound_py_any(py)?,
                ],
            )
        })
        .await;
    }

    async fn record_transaction_begin(&self, duration: Duration) {
        self.duration(TRANSACTION_BEGIN, duration).await;
    }

    async fn record_transaction_commit(&self, duration: Duration) {
        self.duration(TRANSACTION_COMMIT, duration).await;
    }

    async fn record_transaction_rollback(&self, duration: Duration) {
        self.duration(TRANSACTION_ROLLBACK, duration).await;
    }
}

/// What a dispatched hook calls once it has finished, letting the statement
/// it reported on go on. Dropped uncalled, it lets it go on too.
#[pyclass(name = "_MetricsAnswer", module = "pgorm._native", frozen)]
struct Answer(Mutex<Option<oneshot::Sender<()>>>);

impl fmt::Debug for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Answer").finish_non_exhaustive()
    }
}

#[pymethods]
impl Answer {
    fn __call__(&self) {
        if let Some(answer) = self.0.lock().ok().and_then(|mut answer| answer.take()) {
            let _ = answer.send(());
        }
    }
}

/// `pgorm::metric::QueryContext`, owned, so that it outlives the hook call it
/// is made for and its fingerprint is computed only when Python asks.
// [spec:pgorm:req:bindings.metrics]
#[pyclass(name = "QueryContext", module = "pgorm", frozen, skip_from_py_object)]
#[derive(Debug, Clone)]
pub(crate) struct Context {
    operation: String,
    sql: Option<String>,
}

impl Context {
    fn of(query: QueryContext<'_>) -> Self {
        Self {
            operation: query.operation().to_owned(),
            sql: query.sql().map(str::to_owned),
        }
    }
}

#[pymethods]
impl Context {
    #[new]
    #[pyo3(signature = (operation, sql=None))]
    fn new(operation: String, sql: Option<String>) -> Self {
        Self { operation, sql }
    }

    #[getter]
    fn operation(&self) -> &str {
        &self.operation
    }

    #[getter]
    fn sql(&self) -> Option<&str> {
        self.sql.as_deref()
    }

    /// pgorm's fingerprint of the statement, parsed the first time a process
    /// asks for one of this text.
    #[getter]
    fn fingerprint(&self) -> Option<Fingerprint> {
        QueryContext::new(&self.operation, self.sql.as_deref())
            .fingerprint()
            .map(Fingerprint)
    }

    fn __repr__(&self) -> String {
        match &self.sql {
            Some(sql) => format!("QueryContext({:?}, {sql:?})", self.operation),
            None => format!("QueryContext({:?})", self.operation),
        }
    }
}

/// `pgorm::metric::QueryFingerprint`: its 16 hex digits as its `str`, the
/// same number as `value`.
// [spec:pgorm:req:bindings.metrics]
#[pyclass(
    name = "QueryFingerprint",
    module = "pgorm",
    frozen,
    eq,
    hash,
    skip_from_py_object
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Fingerprint(QueryFingerprint);

#[pymethods]
impl Fingerprint {
    #[getter]
    fn value(&self) -> u64 {
        self.0.value()
    }

    fn __str__(&self) -> String {
        self.0.to_string()
    }

    fn __repr__(&self) -> String {
        format!("QueryFingerprint('{}')", self.0)
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("METRICS_HOOKS", HOOKS)?;
    module.add_class::<Context>()?;
    module.add_class::<Fingerprint>()?;
    module.add_class::<Answer>()?;
    Ok(())
}
