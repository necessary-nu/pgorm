//! A pool's options as Python passes them, read into pgorm-bindings'
//! settings, which decide them the same way for both bindings.

use std::{fmt, sync::Arc, time::Duration};

use pgorm_bindings::{Dialect, HookEvent, PoolShape, TlsMode, TlsSettings};
use pyo3::{exceptions::PyTypeError, prelude::*};

use crate::{
    errors::{ConstructionError, Redactions},
    hooks::{self, Callable},
};

pub(crate) fn seconds(value: f64, name: &str) -> PyResult<Duration> {
    if !value.is_finite() || value <= 0.0 {
        return Err(ConstructionError::new_err(format!(
            "{name} must be positive and finite"
        )));
    }
    Duration::try_from_secs_f64(value)
        .map_err(|_| ConstructionError::new_err(format!("{name} is outside the supported range")))
}

fn optional_seconds(value: Option<f64>, name: &str) -> PyResult<Option<Duration>> {
    value.map(|value| seconds(value, name)).transpose()
}

/// The `tls`, `ca`, `cert` and `key` options, the files Python reads already
/// read into PEM.
// [spec:pgorm:req:bindings.tls]
pub(crate) fn tls(
    mode: Option<&str>,
    ca: Option<Vec<u8>>,
    cert: Option<Vec<u8>>,
    key: Option<Vec<u8>>,
) -> PyResult<TlsSettings> {
    let mode = match mode {
        None => None,
        Some(name) => Some(TlsMode::named(name).ok_or_else(|| {
            PyTypeError::new_err(
                "tls is 'disable', 'prefer', 'require', 'verify-ca' or 'verify-full'",
            )
        })?),
    };
    Ok(TlsSettings {
        mode,
        ca,
        cert,
        key,
    })
}

/// The `recycle` and `recycle_sql` options: one of the named methods, or
/// SQL of the program's own.
pub(crate) fn recycling(
    recycle: &str,
    recycle_sql: Option<String>,
) -> PyResult<pgorm::RecyclingMethod> {
    let named = match recycle {
        "verified" => pgorm::RecyclingMethod::Verified,
        "fast" => pgorm::RecyclingMethod::Fast,
        "clean" => pgorm::RecyclingMethod::Clean,
        _ => {
            return Err(PyTypeError::new_err(
                "recycle is 'verified', 'fast' or 'clean'",
            ));
        }
    };
    Ok(match recycle_sql {
        Some(sql) => pgorm::RecyclingMethod::Custom(sql),
        None => named,
    })
}

/// What one pool's shape says, as Python gave it, read into Rust as the
/// pool is built: its size and timeouts, the order it lends in, and the
/// runtime module's wrappers of its hooks.
// [spec:pgorm:req:bindings.connections+3]
#[pyclass(name = "_PoolShape", module = "pgorm._native", frozen)]
pub(crate) struct NativePoolShape {
    max_size: usize,
    acquire_timeout: Duration,
    wait_timeout: Option<Duration>,
    create_timeout: Option<Duration>,
    recycle_timeout: Option<Duration>,
    queue_mode: pgorm::QueueMode,
    post_create: Option<Arc<Py<PyAny>>>,
    pre_recycle: Option<Arc<Py<PyAny>>>,
    post_recycle: Option<Arc<Py<PyAny>>>,
}

impl fmt::Debug for NativePoolShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativePoolShape")
            .field("max_size", &self.max_size)
            .finish_non_exhaustive()
    }
}

#[pymethods]
impl NativePoolShape {
    #[new]
    #[pyo3(signature = (*, max_size=10, acquire_timeout=30.0, wait_timeout=None, create_timeout=None, recycle_timeout=None, queue_mode="fifo", post_create=None, pre_recycle=None, post_recycle=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        max_size: usize,
        acquire_timeout: f64,
        wait_timeout: Option<f64>,
        create_timeout: Option<f64>,
        recycle_timeout: Option<f64>,
        queue_mode: &str,
        post_create: Option<Py<PyAny>>,
        pre_recycle: Option<Py<PyAny>>,
        post_recycle: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let queue_mode = match queue_mode {
            "fifo" => pgorm::QueueMode::Fifo,
            "lifo" => pgorm::QueueMode::Lifo,
            _ => return Err(PyTypeError::new_err("queue_mode is 'fifo' or 'lifo'")),
        };
        Ok(Self {
            max_size,
            acquire_timeout: seconds(acquire_timeout, "acquire_timeout")?,
            wait_timeout: optional_seconds(wait_timeout, "wait_timeout")?,
            create_timeout: optional_seconds(create_timeout, "create_timeout")?,
            recycle_timeout: optional_seconds(recycle_timeout, "recycle_timeout")?,
            queue_mode,
            post_create: post_create.map(Arc::new),
            pre_recycle: pre_recycle.map(Arc::new),
            post_recycle: post_recycle.map(Arc::new),
        })
    }
}

/// Where a pool's hooks run: the loop the pool belongs to.
pub(crate) struct Owner {
    pub(crate) locals: Arc<pyo3_async_runtimes::TaskLocals>,
    pub(crate) owner: Arc<Py<PyAny>>,
}

impl Owner {
    /// The running loop, which must be one.
    pub(crate) fn current(py: Python<'_>) -> PyResult<Self> {
        let owner = Arc::new(crate::runtime::running_loop(py)?.unbind());
        let locals = Arc::new(pyo3_async_runtimes::tokio::get_current_locals(py)?);
        Ok(Self { locals, owner })
    }
}

impl NativePoolShape {
    pub(crate) fn acquire_timeout(&self) -> Duration {
        self.acquire_timeout
    }

    /// The shape pgorm-bindings builds the pool with, each hook calling its
    /// wrapper on `owner`'s loop.
    pub(crate) fn shape(&self, owner: &Owner, secrets: &Redactions) -> PoolShape {
        let hook = |event, call: &Option<Arc<Py<PyAny>>>| {
            call.as_ref().map(|call| {
                hooks::hook(
                    event,
                    Callable {
                        call: call.clone(),
                        locals: owner.locals.clone(),
                        owner: owner.owner.clone(),
                    },
                    secrets.clone(),
                )
            })
        };
        PoolShape {
            max_size: self.max_size,
            wait_timeout: self.wait_timeout,
            create_timeout: self.create_timeout,
            recycle_timeout: self.recycle_timeout,
            queue_mode: self.queue_mode,
            post_create: hook(HookEvent::PostCreate, &self.post_create),
            pre_recycle: hook(HookEvent::PreRecycle, &self.pre_recycle),
            post_recycle: hook(HookEvent::PostRecycle, &self.post_recycle),
        }
    }
}

pub(crate) const DIALECT: Dialect = Dialect::Python;
