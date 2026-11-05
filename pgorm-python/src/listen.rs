//! A dedicated listening connection, and `pg_notify` through any executor.
//!
//! The listener owns pgorm's `Listener` and its `Notifications` outright; the
//! connection behind them is not one of the pool's, so nothing here goes
//! through `Operation`. Closing the listener drops the stream, which closes
//! its connection.

use std::{
    fmt,
    num::NonZeroUsize,
    sync::{Arc, Weak},
};

use futures_util::StreamExt;
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
};
use pyo3_async_runtimes::tokio::future_into_py;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    config::{self, DIALECT, seconds},
    errors::{ConstructionError, InternalError, LifecycleError, Redactions, database_error},
    execution::Target,
    runtime::{PoolState, check_loop, running_loop},
    transactions::work::{Output, Work},
};

pub(crate) struct ListenerState {
    /// The event loop the listener belongs to: its pool's, or the one
    /// `connect_listener` ran on.
    owner: Arc<Py<PyAny>>,
    /// Its pool's closing, which closes it; one that never fires for a
    /// listener of no pool.
    pool_closed: CancellationToken,
    secrets: Redactions,
    listener: pgorm::Listener,
    /// `None` once the listener is closed or its stream has ended.
    notifications: Arc<Mutex<Option<pgorm::Notifications>>>,
    closed: CancellationToken,
}

impl fmt::Debug for ListenerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListenerState")
            .field("closed", &self.closed.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl ListenerState {
    /// Close without waiting: a stream no pull holds is dropped here, and one
    /// a pending pull holds is dropped by that pull when it sees the close.
    pub(crate) fn close_now(&self) {
        self.closed.cancel();
        if let Ok(mut notifications) = self.notifications.try_lock() {
            drop(notifications.take());
        }
    }

    fn ensure_pool_open(&self) -> PyResult<()> {
        if self.pool_closed.is_cancelled() {
            Err(LifecycleError::new_err("pool is closed"))
        } else {
            Ok(())
        }
    }

    fn check_owner(&self, py: Python<'_>) -> PyResult<()> {
        check_loop(py, &self.owner, "listener")
    }

    fn ensure_open(&self) -> PyResult<()> {
        self.ensure_pool_open()?;
        if self.closed.is_cancelled() {
            Err(LifecycleError::new_err("listener is closed"))
        } else {
            Ok(())
        }
    }
}

/// Closes the listener if the request it guards is abandoned before its
/// reply: the server may have acted on a `LISTEN` or `UNLISTEN` whose answer
/// never arrived, so which channels the connection listens on is unknown.
struct CloseUnlessAnswered(Option<Arc<ListenerState>>);

impl Drop for CloseUnlessAnswered {
    fn drop(&mut self) {
        if let Some(state) = self.0.take() {
            state.close_now();
        }
    }
}

/// Open a listener on `pool`'s configuration and register it, so closing the
/// pool closes it.
// [spec:pgorm:req:bindings.listen+3]
pub(crate) fn open<'py>(
    py: Python<'py>,
    pool: Arc<PoolState>,
    capacity: usize,
) -> PyResult<Bound<'py, PyAny>> {
    pool.check_owner(py)?;
    pool.ensure_open()?;
    let capacity = NonZeroUsize::new(capacity)
        .ok_or_else(|| PyValueError::new_err("capacity must be a positive integer"))?;
    future_into_py(py, async move {
        let (listener, notifications) = tokio::select! {
            biased;
            _ = pool.cancelled.cancelled() => return Err(LifecycleError::new_err("pool is closed")),
            opened = pool.pool.listener(capacity) => {
                opened.map_err(|error| database_error(error, &pool.secrets))?
            }
        };
        let state = Arc::new(ListenerState {
            owner: pool.owner.clone(),
            pool_closed: pool.cancelled.clone(),
            secrets: pool.secrets.clone(),
            listener,
            notifications: Arc::new(Mutex::new(Some(notifications))),
            closed: CancellationToken::new(),
        });
        pool.register_listener(&state)?;
        Ok(NativeListener { state })
    })
}

/// Open a listener on a connection of its own, belonging to no pool, as
/// pgorm's `connect_listener` does: the connection string and TLS options
/// read as a pool's are, every refusal raised before anything is sent.
// [spec:pgorm:req:bindings.listen+3]
// [spec:pgorm:req:bindings.tls]
#[pyfunction]
#[pyo3(name = "_connect_listener", signature = (dsn, capacity, *, tls=None, ca_pem=None, cert_pem=None, key_pem=None, connect_timeout=10.0))]
#[allow(clippy::too_many_arguments)]
fn connect_listener<'py>(
    py: Python<'py>,
    dsn: String,
    capacity: usize,
    tls: Option<String>,
    ca_pem: Option<Vec<u8>>,
    cert_pem: Option<Vec<u8>>,
    key_pem: Option<Vec<u8>>,
    connect_timeout: f64,
) -> PyResult<Bound<'py, PyAny>> {
    let owner = Arc::new(running_loop(py)?.unbind());
    let capacity = NonZeroUsize::new(capacity)
        .ok_or_else(|| PyValueError::new_err("capacity must be a positive integer"))?;
    let tls = config::tls(tls.as_deref(), ca_pem, cert_pem, key_pem)?;
    let (configured, connector) = pgorm_bindings::listener_config(
        &dsn,
        &tls,
        seconds(connect_timeout, "connect_timeout")?,
        DIALECT,
    )
    .map_err(|refusal| ConstructionError::new_err(refusal.into_message()))?;
    let secrets = Redactions::from_config(&configured);
    future_into_py(py, async move {
        let (listener, notifications) =
            pgorm_bindings::connect_listener(configured, connector, capacity)
                .await
                .map_err(|error| database_error(error, &secrets))?;
        Ok(NativeListener {
            state: Arc::new(ListenerState {
                owner,
                pool_closed: CancellationToken::new(),
                secrets,
                listener,
                notifications: Arc::new(Mutex::new(Some(notifications))),
                closed: CancellationToken::new(),
            }),
        })
    })
}

/// What a request on the listener asks for.
enum Request {
    Listen(String),
    Unlisten(String),
    UnlistenAll,
}

// [spec:pgorm:req:bindings.argument-errors+1]
fn channel(value: &Bound<'_, PyAny>) -> PyResult<String> {
    value
        .extract::<String>()
        .map_err(|_| PyTypeError::new_err("a channel name must be a str"))
}

// [spec:pgorm:req:bindings.listen+3]
// [spec:pgorm:req:bindings.cancellation]
/// A dedicated connection's `LISTEN` set and the notifications it receives.
#[pyclass(name = "_Listener", module = "pgorm._native", frozen)]
pub(crate) struct NativeListener {
    state: Arc<ListenerState>,
}

impl fmt::Debug for NativeListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeListener")
            .field("closed", &self.closed())
            .finish()
    }
}

impl NativeListener {
    fn request<'py>(&self, py: Python<'py>, request: Request) -> PyResult<Bound<'py, PyAny>> {
        self.state.check_owner(py)?;
        self.state.ensure_open()?;
        let state = self.state.clone();
        future_into_py(py, async move {
            state.ensure_open()?;
            let mut guard = CloseUnlessAnswered(Some(state.clone()));
            let listener = &state.listener;
            let sent = async {
                match &request {
                    Request::Listen(channel) => listener.listen(channel).await,
                    Request::Unlisten(channel) => listener.unlisten(channel).await,
                    Request::UnlistenAll => listener.unlisten_all().await,
                }
            };
            let result = tokio::select! {
                biased;
                _ = state.pool_closed.cancelled() => return Err(LifecycleError::new_err("pool is closed")),
                _ = state.closed.cancelled() => return Err(LifecycleError::new_err("listener is closed")),
                result = sent => result,
            };
            guard.0 = None;
            result.map_err(|error| database_error(error, &state.secrets))
        })
    }
}

#[pymethods]
impl NativeListener {
    fn listen<'py>(&self, py: Python<'py>, name: &Bound<'_, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let name = channel(name)?;
        self.request(py, Request::Listen(name))
    }

    fn unlisten<'py>(
        &self,
        py: Python<'py>,
        name: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let name = channel(name)?;
        self.request(py, Request::Unlisten(name))
    }

    fn unlisten_all<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.request(py, Request::UnlistenAll)
    }

    /// The next notification, or `None` once the listener is closed. An
    /// error ends the stream and closes the listener; a cancelled pull loses
    /// nothing, the notification it awaited staying queued.
    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.check_owner(py)?;
        let state = self.state.clone();
        future_into_py(py, async move {
            state.ensure_pool_open()?;
            let mut slot = state.notifications.clone().try_lock_owned().map_err(|_| {
                LifecycleError::new_err("listener already has a pending next operation")
            })?;
            if state.closed.is_cancelled() {
                drop(slot.take());
                return Ok(None);
            }
            let Some(notifications) = slot.as_mut() else {
                return Ok(None);
            };
            let item = tokio::select! {
                biased;
                _ = state.pool_closed.cancelled() => {
                    drop(slot.take());
                    return Err(LifecycleError::new_err("pool is closed"));
                }
                _ = state.closed.cancelled() => {
                    drop(slot.take());
                    return Ok(None);
                }
                item = notifications.next() => item,
            };
            match item {
                Some(Ok(notification)) => Ok(Some(PyNotification {
                    channel: notification.channel().to_owned(),
                    payload: notification.payload().to_owned(),
                    process_id: notification.process_id(),
                })),
                Some(Err(error)) => {
                    state.closed.cancel();
                    drop(slot.take());
                    Err(database_error(error, &state.secrets))
                }
                None => {
                    state.closed.cancel();
                    drop(slot.take());
                    Ok(None)
                }
            }
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.check_owner(py)?;
        self.state.close_now();
        let state = self.state.clone();
        future_into_py(py, async move {
            drop(state.notifications.lock().await.take());
            Ok(())
        })
    }

    fn closed(&self) -> bool {
        self.state.closed.is_cancelled() || self.state.pool_closed.is_cancelled()
    }

    fn __repr__(&self) -> String {
        format!("<pgorm native listener closed={}>", self.closed())
    }
}

impl Drop for NativeListener {
    fn drop(&mut self) {
        self.state.close_now();
    }
}

// [spec:pgorm:req:bindings.listen+3]
/// One notification: its channel, its payload and the process id of the
/// session that sent it.
#[pyclass(
    name = "Notification",
    module = "pgorm",
    frozen,
    eq,
    hash,
    skip_from_py_object
)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PyNotification {
    channel: String,
    payload: String,
    process_id: i32,
}

#[pymethods]
impl PyNotification {
    #[getter]
    fn channel(&self) -> &str {
        &self.channel
    }

    #[getter]
    fn payload(&self) -> &str {
        &self.payload
    }

    #[getter]
    fn process_id(&self) -> i32 {
        self.process_id
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "notifications cannot be tested as Python booleans",
        ))
    }

    fn __repr__(&self) -> String {
        format!(
            "Notification(channel={:?}, payload={:?}, process_id={})",
            self.channel, self.payload, self.process_id
        )
    }
}

/// Send `payload` on `channel` through pgorm's `notify` on a connection or
/// transaction: bound, and delivered when the transaction commits.
// [spec:pgorm:req:bindings.listen+3]
pub(crate) fn notify<'py>(
    py: Python<'py>,
    target: Target,
    channel: &Bound<'_, PyAny>,
    payload: &Bound<'_, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    target.check(py)?;
    let (Ok(channel), Ok(payload)) = (channel.extract::<String>(), payload.extract::<String>())
    else {
        return Err(PyTypeError::new_err(
            "notify takes a str channel and a str payload",
        ));
    };
    future_into_py(py, async move {
        match target.run(Work::Notify(channel, payload)).await? {
            Output::Finished => Ok(()),
            _ => Err(InternalError::new_err("unexpected notify result")),
        }
    })
}

/// Drop every listener `registry` still holds a handle to.
pub(crate) fn close_all(registry: &[Weak<ListenerState>]) {
    for state in registry.iter().filter_map(Weak::upgrade) {
        state.close_now();
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeListener>()?;
    module.add_class::<PyNotification>()?;
    module.add_function(wrap_pyfunction!(connect_listener, module)?)?;
    Ok(())
}

// [spec:pgorm:req:python.capabilities+2]
// [spec:pgorm:req:bindings.listen+3]
pub(crate) fn policy() -> Value {
    json!({
        "connection": "a dedicated connection opened with the pool's configuration and TLS, outside its size, which closing the pool closes; or connect_listener's, belonging to no pool, which only its own close closes",
        "channels": "exact names sent as quoted identifiers; a name over 63 bytes raises ConstructionError before anything is sent",
        "buffering": "at most `capacity` notifications wait unread; one more closes the listener and raises ConnectionError after those waiting",
        "end": "a failed or terminated connection raises its error after the notifications already received; nothing reconnects",
        "cancellation": "a cancelled next loses nothing; a cancelled listen, unlisten or unlisten_all closes the listener",
        "notify": "SELECT pg_notify($1, $2) with both values bound, delivered when the transaction commits"
    })
}
