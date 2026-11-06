//! A pool's hooks, each a Python callable the pool's event loop runs while
//! pgorm holds the connection the hook is about.
//!
//! The hook itself runs on the runtime, through pgorm-bindings' `hook`,
//! which serves the statements the callable sends until it returns. The
//! callable runs on the pool's own loop, scheduled there from the runtime
//! as `asyncio.run_coroutine_threadsafe` would, so no Python code of the
//! program runs on a runtime thread.

use std::{fmt, sync::Arc, time::Duration};

use pgorm_bindings::{HookCall, HookEvent, HookJobs};
use pyo3::prelude::*;
use pyo3_async_runtimes::TaskLocals;
use tokio::sync::oneshot;

use crate::{
    errors::{LifecycleError, Redactions, database_error},
    execution::{Database, Executor, Target},
    runtime::check_loop,
    transactions::work::{Output, Work},
};

/// The connection a hook runs on, while it runs.
pub(crate) struct HookState {
    jobs: HookJobs,
    owner: Arc<Py<PyAny>>,
    secrets: Redactions,
    age: Duration,
    recycle_count: usize,
}

impl fmt::Debug for HookState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HookState")
            .field("recycle_count", &self.recycle_count)
            .finish_non_exhaustive()
    }
}

impl HookState {
    pub(crate) fn check(&self, py: Python<'_>) -> PyResult<()> {
        check_loop(py, &self.owner, "pool")?;
        if self.jobs.0.is_closed() {
            return Err(LifecycleError::new_err(
                "the hook has returned, so its connection is the pool's",
            ));
        }
        Ok(())
    }

    /// Run `work` on the hook's connection, which serves it before the hook
    /// returns.
    // [spec:pgorm:req:bindings.pool-hooks]
    pub(crate) async fn run(&self, work: Work) -> PyResult<Output> {
        let (reply, answer) = oneshot::channel();
        let closed =
            || LifecycleError::new_err("the hook has returned, so its connection is the pool's");
        if !self.jobs.submit(move |db| {
            Box::pin(async move {
                drop(reply.send(work.run(Database::Hook(*db)).await));
            })
        }) {
            return Err(closed());
        }
        answer
            .await
            .map_err(|_| closed())?
            .map_err(|error| database_error(error, &self.secrets))
    }
}

/// The connection a hook's callable is given: statements on it run on the
/// connection the hook is about, until the hook returns.
// [spec:pgorm:req:bindings.pool-hooks]
#[derive(Debug)]
#[pyclass(name = "_HookConnection", module = "pgorm._native", frozen)]
pub(crate) struct NativeHookConnection {
    pub(crate) state: Arc<HookState>,
}

#[pymethods]
impl NativeHookConnection {
    fn execute<'py>(
        &self,
        py: Python<'py>,
        query: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        crate::results::execute(py, Target::Hook(self.state.clone()), query)
    }

    fn fetch<'py>(
        &self,
        py: Python<'py>,
        query: &Bound<'_, PyAny>,
        mode: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        crate::results::fetch(
            py,
            Executor::Held(Target::Hook(self.state.clone())),
            query,
            mode,
        )
    }

    /// Seconds since the connection opened.
    #[getter]
    fn age(&self) -> f64 {
        self.state.age.as_secs_f64()
    }

    /// How many times the pool has recycled the connection.
    #[getter]
    fn recycle_count(&self) -> usize {
        self.state.recycle_count
    }

    fn closed(&self) -> bool {
        self.state.jobs.0.is_closed()
    }

    fn __repr__(&self) -> String {
        format!("<pgorm native hook connection closed={}>", self.closed())
    }
}

/// What a pool's hook calls: the runtime module's wrapper of the program's
/// callable, which awaits what it returns, and the loop it runs on.
#[derive(Clone)]
pub(crate) struct Callable {
    pub(crate) call: Arc<Py<PyAny>>,
    pub(crate) locals: Arc<TaskLocals>,
    pub(crate) owner: Arc<Py<PyAny>>,
}

impl fmt::Debug for Callable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Callable").finish_non_exhaustive()
    }
}

/// The pgorm hook that runs `callable` for `event`. What the callable
/// raises refuses the connection, its class and message the refusal's text.
// [spec:pgorm:req:bindings.pool-hooks]
pub(crate) fn hook(event: HookEvent, callable: Callable, secrets: Redactions) -> pgorm::Hook {
    pgorm_bindings::hook(event, move |call: HookCall| {
        let callable = callable.clone();
        let secrets = secrets.clone();
        async move {
            let state = Arc::new(HookState {
                jobs: call.jobs,
                owner: callable.owner.clone(),
                secrets,
                age: call.age,
                recycle_count: call.recycle_count,
            });
            let pending = Python::attach(|py| {
                let connection = Py::new(py, NativeHookConnection { state })?;
                let awaitable = callable.call.bind(py).call1((connection,))?;
                pyo3_async_runtimes::into_future_with_locals(&callable.locals, awaitable)
            })
            .map_err(describe)?;
            pending.await.map(drop).map_err(describe)
        }
    })
}

/// A Python exception as a refusal's text: its class's name and message.
fn describe(error: PyErr) -> String {
    Python::attach(|py| {
        let class = error
            .get_type(py)
            .name()
            .map_or_else(|_| "Exception".to_owned(), |name| name.to_string());
        let message = error.value(py).to_string();
        if message.is_empty() {
            class
        } else {
            format!("{class}: {message}")
        }
    })
}
