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

use std::{fmt, sync::Mutex};

use futures_util::future::BoxFuture;
use pgorm::metric::{QueryContext, QueryFingerprint};
use pgorm_bindings::{Hook, Metrics, Redactions, Report, Reporter};
use pyo3::{IntoPyObjectExt, prelude::*, types::PyTuple};
use tokio::sync::oneshot;

use crate::{config::DIALECT, errors::raised};

/// The collector an instrumented Python pool hands pgorm's wrappers, its
/// hooks run on the pool's event loop.
// [spec:pgorm:req:bindings.metrics]
pub(crate) fn metrics(
    event_loop: Py<PyAny>,
    dispatch: Py<PyAny>,
    hooks: [Option<Py<PyAny>>; Hook::ALL.len()],
    secrets: Redactions,
) -> Metrics {
    Metrics::new(
        Collector {
            event_loop,
            dispatch,
            hooks,
        },
        secrets,
        DIALECT,
    )
}

struct Collector {
    /// The loop the pool belongs to, where every hook runs.
    event_loop: Py<PyAny>,
    /// `pgorm.metrics._dispatch`, which runs one hook and answers once it
    /// has finished.
    dispatch: Py<PyAny>,
    /// Each hook's callable, in `Hook::ALL`'s order, or `None` for one the
    /// collector left as `NoOpMetrics` wrote it.
    hooks: [Option<Py<PyAny>>; Hook::ALL.len()],
}

impl Collector {
    fn hook(&self, hook: Hook) -> Option<&Py<PyAny>> {
        let index = Hook::ALL.iter().position(|each| *each == hook)?;
        self.hooks[index].as_ref()
    }
}

/// What a report is as a hook's arguments: a statement's context, its
/// duration in seconds, and its rows or its exception.
fn arguments<'py>(py: Python<'py>, report: Report) -> PyResult<Bound<'py, PyTuple>> {
    let seconds = |duration: std::time::Duration| duration.as_secs_f64().into_bound_py_any(py);
    match report {
        Report::Query {
            operation,
            sql,
            duration,
            outcome,
        } => {
            let last = match outcome {
                Ok(rows) => rows.into_bound_py_any(py)?,
                Err(failure) => raised(failure).into_value(py).into_bound_py_any(py)?,
            };
            PyTuple::new(
                py,
                [
                    Context { operation, sql }.into_bound_py_any(py)?,
                    seconds(duration)?,
                    last,
                ],
            )
        }
        Report::Duration(duration) => PyTuple::new(py, [seconds(duration)?]),
        Report::Failed(duration, failure) => PyTuple::new(
            py,
            [
                seconds(duration)?,
                raised(failure).into_value(py).into_bound_py_any(py)?,
            ],
        ),
    }
}

impl Reporter for Collector {
    fn reports(&self, hook: Hook) -> bool {
        self.hook(hook).is_some()
    }

    /// Queue the hook on the pool's event loop and wait for its answer. A
    /// hook that cannot be queued, its loop closed, is not waited for.
    fn report(&self, hook: Hook, report: Report) -> BoxFuture<'static, ()> {
        let (answer, answered) = oneshot::channel();
        let queued = Python::attach(|py| -> PyResult<()> {
            let Some(callable) = self.hook(hook) else {
                return Ok(());
            };
            let args = arguments(py, report)?;
            let answer = Answer(Mutex::new(Some(answer)));
            self.event_loop.bind(py).call_method1(
                "call_soon_threadsafe",
                (
                    self.dispatch.bind(py),
                    hook.name(DIALECT),
                    callable.bind(py),
                    args,
                    answer,
                ),
            )?;
            Ok(())
        });
        Box::pin(async move {
            if queued.is_ok() {
                let _ = answered.await;
            }
        })
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
    module.add(
        "METRICS_HOOKS",
        Hook::ALL.map(|hook| hook.name(DIALECT)).to_vec(),
    )?;
    module.add_class::<Context>()?;
    module.add_class::<Fingerprint>()?;
    module.add_class::<Answer>()?;
    Ok(())
}
