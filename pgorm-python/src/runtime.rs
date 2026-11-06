use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex as RegistryMutex, Weak},
    time::Duration,
};

use crate::panics::future_into_py;
use pgorm::{
    ConnectionTrait, DatabaseConnection,
    metric::{InstrumentedConnection, InstrumentedPool},
};
use pyo3::prelude::*;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{self, DIALECT, NativePoolShape, Owner, seconds},
    errors::{
        ConstructionError, InternalError, LifecycleError, Redactions, TimeoutError, database_error,
    },
    execution::Database,
    listen::ListenerState,
    metrics::{HOOKS, Metrics},
};

/// A connection checked out of a pool: a plain one, or one an instrumented
/// pool lent, which reports what runs on it.
// [spec:pgorm:req:bindings.metrics]
#[derive(Debug)]
pub(crate) enum Held {
    Plain(DatabaseConnection),
    Metered(InstrumentedConnection<Metrics>),
}

impl Held {
    pub(crate) fn database(&self) -> Database<'_> {
        match self {
            Self::Plain(connection) => Database::Connection(connection),
            Self::Metered(connection) => Database::InstrumentedConnection(connection),
        }
    }

    fn discard(self) {
        match self {
            Self::Plain(connection) => connection.discard(),
            Self::Metered(connection) => connection.into_inner().discard(),
        }
    }
}

pub(crate) struct PoolState {
    pub(crate) pool: pgorm::DatabasePool,
    pub(crate) secrets: Redactions,
    pub(crate) cancelled: CancellationToken,
    pub(crate) owner: Arc<Py<PyAny>>,
    acquire_timeout: Duration,
    connections: RegistryMutex<Vec<Weak<ConnectionState>>>,
    listeners: RegistryMutex<Vec<Weak<ListenerState>>>,
}

impl fmt::Debug for PoolState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoolState")
            .field("closed", &self.cancelled.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// Refuse to run from any event loop but `owner`, the one the pool or
/// listener `what` names was built on.
pub(crate) fn check_loop(py: Python<'_>, owner: &Py<PyAny>, what: &str) -> PyResult<()> {
    let current = running_loop(py)?;
    if !owner.bind(py).is(&current) {
        return Err(LifecycleError::new_err(format!(
            "{what} belongs to another asyncio event loop"
        )));
    }
    Ok(())
}

impl PoolState {
    fn new(
        pool: pgorm::DatabasePool,
        secrets: Redactions,
        owner: Arc<Py<PyAny>>,
        acquire_timeout: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            secrets,
            owner,
            acquire_timeout,
            cancelled: CancellationToken::new(),
            connections: RegistryMutex::new(Vec::new()),
            listeners: RegistryMutex::new(Vec::new()),
        })
    }

    pub(crate) fn check_owner(&self, py: Python<'_>) -> PyResult<()> {
        check_loop(py, &self.owner, "pool")
    }

    pub(crate) fn ensure_open(&self) -> PyResult<()> {
        if self.cancelled.is_cancelled() {
            Err(LifecycleError::new_err("pool is closed"))
        } else {
            Ok(())
        }
    }

    /// Record `listener` so that closing the pool closes it; a listener
    /// opened while the pool closed is closed here instead.
    // [spec:pgorm:req:bindings.listen+3]
    pub(crate) fn register_listener(&self, listener: &Arc<ListenerState>) -> PyResult<()> {
        let mut registry = self
            .listeners
            .lock()
            .map_err(|_| InternalError::new_err("listener registry lock failed"))?;
        if self.cancelled.is_cancelled() {
            listener.close_now();
            return Err(LifecycleError::new_err("pool is closed"));
        }
        registry.retain(|item| item.strong_count() > 0);
        registry.push(Arc::downgrade(listener));
        Ok(())
    }

    fn close_listeners(&self) {
        if let Ok(mut registry) = self.listeners.lock() {
            crate::listen::close_all(&registry);
            registry.clear();
        }
    }

    /// Check a connection out within the acquire timeout, for its borrower
    /// alone, and record it so that closing the pool closes it. Through
    /// `metered`, an instrumented pool's, the acquisition is reported and the
    /// connection reports what runs on it.
    pub(crate) async fn lend(
        self: Arc<Self>,
        metered: Option<InstrumentedPool<Metrics>>,
    ) -> PyResult<Arc<ConnectionState>> {
        let acquire = async {
            match &metered {
                Some(metered) => metered.get().await.map(Held::Metered),
                None => self.pool.get().await.map(Held::Plain),
            }
        };
        let connection = tokio::select! {
            _ = self.cancelled.cancelled() => return Err(LifecycleError::new_err("pool is closed")),
            result = tokio::time::timeout(self.acquire_timeout, acquire) => {
                self.ensure_open()?;
                result.map_err(|_| TimeoutError::new_err("pool acquisition timed out"))?
                    .map_err(|error| database_error(error, &self.secrets))?
            }
        };
        if self.cancelled.is_cancelled() {
            connection.discard();
            return Err(LifecycleError::new_err("pool is closed"));
        }
        let state = Arc::new(ConnectionState {
            pool: self.clone(),
            cancelled: CancellationToken::new(),
            slot: Arc::new(Mutex::new(Some(connection))),
        });
        let mut registry = self
            .connections
            .lock()
            .map_err(|_| InternalError::new_err("connection registry lock failed"))?;
        if self.cancelled.is_cancelled() {
            state.close_now();
            return Err(LifecycleError::new_err("pool is closed"));
        }
        registry.retain(|item| item.strong_count() > 0);
        registry.push(Arc::downgrade(&state));
        Ok(state)
    }

    fn close_now(&self) -> PyResult<Vec<Arc<ConnectionState>>> {
        self.cancelled.cancel();
        self.pool.close();
        self.close_listeners();
        let mut registry = self
            .connections
            .lock()
            .map_err(|_| InternalError::new_err("connection registry lock failed"))?;
        let connections: Vec<_> = registry.iter().filter_map(Weak::upgrade).collect();
        registry.clear();
        for connection in &connections {
            connection.close_now();
        }
        Ok(connections)
    }
}

pub(crate) fn running_loop(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("asyncio")?
        .call_method0("get_running_loop")
        .map_err(|_| {
            LifecycleError::new_err("pgorm resources require a running asyncio event loop")
        })
}

/// Closes its pool once the last native pool over it is gone: a pool and
/// every instrumented pool wrapping it share one.
#[derive(Debug)]
struct Closer(Arc<PoolState>);

impl Drop for Closer {
    fn drop(&mut self) {
        // Cleanup may run after the Python event loop has stopped.
        let state = &self.0;
        state.cancelled.cancel();
        state.pool.close();
        state.close_listeners();
        if let Ok(registry) = state.connections.lock() {
            for state in registry.iter().filter_map(Weak::upgrade) {
                state.close_now();
            }
        }
    }
}

// [spec:pgorm:req:python.runtime+2]
// [spec:pgorm:req:bindings.connections+3]
#[derive(Debug)]
#[pyclass(name = "_Pool", module = "pgorm._native", frozen)]
pub(crate) struct NativePool {
    pub(crate) state: Arc<PoolState>,
    /// An instrumented pool's pgorm wrapper, which every connection this
    /// pool lends is checked out through.
    pub(crate) metered: Option<InstrumentedPool<Metrics>>,
    closer: Arc<Closer>,
}

impl NativePool {
    /// The native pool over `state`, which closes once the last native pool
    /// over it is gone.
    fn over(state: Arc<PoolState>) -> Self {
        Self {
            closer: Arc::new(Closer(state.clone())),
            state,
            metered: None,
        }
    }
}

/// The options every pool built from one connection string shares, read
/// into pgorm-bindings' settings.
#[allow(clippy::too_many_arguments)]
fn settings(
    dsn: String,
    tag: Option<String>,
    tls: Option<&str>,
    ca_pem: Option<Vec<u8>>,
    cert_pem: Option<Vec<u8>>,
    key_pem: Option<Vec<u8>>,
    connect_timeout: f64,
    statement_cache_size: usize,
    recycle: &str,
    recycle_sql: Option<String>,
    statement_cache_channel: Option<String>,
) -> PyResult<pgorm_bindings::PoolSettings> {
    Ok(pgorm_bindings::PoolSettings {
        dsn,
        tls: config::tls(tls, ca_pem, cert_pem, key_pem)?,
        connect_timeout: seconds(connect_timeout, "connect_timeout")?,
        recycling_method: config::recycling(recycle, recycle_sql)?,
        statement_cache_size,
        statement_cache_channel,
        tag,
    })
}

#[pymethods]
impl NativePool {
    #[new]
    #[pyo3(signature = (dsn, shape, *, tag=None, tls=None, ca_pem=None, cert_pem=None, key_pem=None, connect_timeout=10.0, statement_cache_size=128, recycle="verified", recycle_sql=None, statement_cache_channel=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        dsn: String,
        shape: PyRef<'_, NativePoolShape>,
        tag: Option<String>,
        tls: Option<String>,
        ca_pem: Option<Vec<u8>>,
        cert_pem: Option<Vec<u8>>,
        key_pem: Option<Vec<u8>>,
        connect_timeout: f64,
        statement_cache_size: usize,
        recycle: &str,
        recycle_sql: Option<String>,
        statement_cache_channel: Option<String>,
    ) -> PyResult<Self> {
        let owner = Owner::current(py)?;
        let settings = settings(
            dsn,
            tag,
            tls.as_deref(),
            ca_pem,
            cert_pem,
            key_pem,
            connect_timeout,
            statement_cache_size,
            recycle,
            recycle_sql,
            statement_cache_channel,
        )?;
        let mut secrets = None;
        let pool = settings
            .build(
                |config| {
                    let redactions = Redactions::from_config(config);
                    let shaped = shape.shape(&owner, &redactions);
                    secrets = Some(redactions);
                    shaped
                },
                DIALECT,
            )
            .map_err(|refusal| ConstructionError::new_err(refusal.into_message()))?;
        let secrets =
            secrets.ok_or_else(|| InternalError::new_err("the pool was built unshaped"))?;
        Ok(Self::over(PoolState::new(
            pool,
            secrets,
            owner.owner,
            shape.acquire_timeout(),
        )))
    }

    /// One pool per entry of `shapes`, each tagged with its key, sharing the
    /// connection, TLS and manager configuration, in the tags' order.
    // [spec:pgorm:req:bindings.multi-pool]
    #[staticmethod]
    #[pyo3(signature = (dsn, shapes, *, tls=None, ca_pem=None, cert_pem=None, key_pem=None, connect_timeout=10.0, statement_cache_size=128, recycle="verified", recycle_sql=None, statement_cache_channel=None))]
    #[allow(clippy::too_many_arguments)]
    fn multi(
        py: Python<'_>,
        dsn: String,
        shapes: BTreeMap<String, PyRef<'_, NativePoolShape>>,
        tls: Option<String>,
        ca_pem: Option<Vec<u8>>,
        cert_pem: Option<Vec<u8>>,
        key_pem: Option<Vec<u8>>,
        connect_timeout: f64,
        statement_cache_size: usize,
        recycle: &str,
        recycle_sql: Option<String>,
        statement_cache_channel: Option<String>,
    ) -> PyResult<Vec<(String, Self)>> {
        let owner = Owner::current(py)?;
        let settings = settings(
            dsn,
            None,
            tls.as_deref(),
            ca_pem,
            cert_pem,
            key_pem,
            connect_timeout,
            statement_cache_size,
            recycle,
            recycle_sql,
            statement_cache_channel,
        )?;
        let mut secrets = None;
        let pools = pgorm_bindings::multi_pool(
            settings,
            |config| {
                let redactions = Redactions::from_config(config);
                let shaped = shapes
                    .iter()
                    .map(|(tag, shape)| (tag.clone(), shape.shape(&owner, &redactions)))
                    .collect();
                secrets = Some(redactions);
                shaped
            },
            DIALECT,
        )
        .map_err(|refusal| ConstructionError::new_err(refusal.into_message()))?;
        let secrets =
            secrets.ok_or_else(|| InternalError::new_err("the pools were built unshaped"))?;
        pools
            .iter()
            .map(|(tag, pool)| {
                let shape = shapes
                    .get(tag)
                    .ok_or_else(|| InternalError::new_err("a pool has no shape"))?;
                Ok((
                    tag.to_owned(),
                    Self::over(PoolState::new(
                        pool.clone(),
                        secrets.clone(),
                        owner.owner.clone(),
                        shape.acquire_timeout(),
                    )),
                ))
            })
            .collect()
    }

    /// The pool's tag, or the `default-N` pgorm named it.
    fn tag(&self) -> String {
        self.state.pool.tag().to_string()
    }

    /// This pool, sharing its connections and its closing, reporting to the
    /// collector whose seven hooks `hooks` holds in `METRICS_HOOKS`' order —
    /// `None` for one never to run — each run on this pool's event loop by
    /// `dispatch`.
    // [spec:pgorm:req:bindings.metrics]
    fn instrumented(
        &self,
        py: Python<'_>,
        hooks: Vec<Option<Py<PyAny>>>,
        dispatch: Py<PyAny>,
    ) -> PyResult<Self> {
        self.state.check_owner(py)?;
        if self.metered.is_some() {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "an InstrumentedPool wraps a pool that is not instrumented",
            ));
        }
        let hooks: [Option<Py<PyAny>>; HOOKS.len()] = hooks
            .try_into()
            .map_err(|_| pyo3::exceptions::PyTypeError::new_err("a collector has seven hooks"))?;
        let metrics = Metrics::new(
            self.state.owner.clone_ref(py),
            dispatch,
            hooks,
            self.state.secrets.clone(),
        );
        Ok(Self {
            state: self.state.clone(),
            metered: Some(InstrumentedPool::new(self.state.pool.clone(), metrics)),
            closer: self.closer.clone(),
        })
    }

    fn acquire<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.check_owner(py)?;
        self.state.ensure_open()?;
        let pool = self.state.clone();
        let metered = self.metered.clone();
        future_into_py(py, async move {
            Ok(NativeConnection {
                state: pool.lend(metered).await?,
            })
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.check_owner(py)?;
        let connections = self.state.close_now()?;
        future_into_py(py, async move {
            for connection in connections {
                drop(connection.slot.lock().await.take());
            }
            Ok(())
        })
    }

    fn closed(&self) -> bool {
        self.state.cancelled.is_cancelled()
    }

    /// Open a dedicated listening connection with this pool's configuration.
    #[pyo3(signature = (capacity=1024))]
    fn listener<'py>(&self, py: Python<'py>, capacity: usize) -> PyResult<Bound<'py, PyAny>> {
        crate::listen::open(py, self.state.clone(), capacity)
    }

    /// Mark every connection's cached prepared statements stale, as
    /// `DatabasePool::clear_statement_caches` does. Nothing is sent, so a
    /// closed pool or another event loop is no reason to refuse it.
    // [spec:pgorm:req:bindings.connections+3]
    // [spec:pgorm:req:conn.pool.statement-cache.clear+1]
    fn clear_statement_caches(&self) {
        self.state.pool.clear_statement_caches();
    }

    fn status(&self, py: Python<'_>) -> PyResult<(usize, usize, usize, usize)> {
        self.state.check_owner(py)?;
        let status = self.state.pool.status();
        Ok((
            status.max_size,
            status.size,
            status.available,
            status.waiting,
        ))
    }

    fn __repr__(&self) -> String {
        format!("<pgorm native pool closed={}>", self.closed())
    }
}

pub(crate) struct ConnectionState {
    pub(crate) pool: Arc<PoolState>,
    pub(crate) cancelled: CancellationToken,
    pub(crate) slot: Arc<Mutex<Option<Held>>>,
}

impl fmt::Debug for ConnectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionState")
            .field("closed", &self.cancelled.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl ConnectionState {
    fn close_now(&self) {
        self.cancelled.cancel();
        if let Ok(mut slot) = self.slot.try_lock() {
            drop(slot.take());
        }
    }

    /// Close, discarding the connection rather than returning it: one an
    /// operation holds is discarded by that operation once it sees the close.
    // [spec:pgorm:req:bindings.connections+3]
    fn discard_now(&self) {
        self.cancelled.cancel();
        if let Ok(mut slot) = self.slot.try_lock()
            && let Some(connection) = slot.take()
        {
            connection.discard();
        }
    }

    pub(crate) fn ensure_open(&self) -> PyResult<()> {
        self.pool.ensure_open()?;
        if self.cancelled.is_cancelled() {
            Err(LifecycleError::new_err("connection is closed"))
        } else {
            Ok(())
        }
    }
}

/// Cancellation drops the active connection instead of returning unknown state.
pub(crate) struct Operation {
    slot: OwnedMutexGuard<Option<Held>>,
    connection: Option<Held>,
    state: Arc<ConnectionState>,
}

impl Operation {
    pub(crate) fn begin(state: Arc<ConnectionState>) -> PyResult<Self> {
        state.ensure_open()?;
        let mut slot =
            state.slot.clone().try_lock_owned().map_err(|_| {
                LifecycleError::new_err("connection already has an active operation")
            })?;
        let connection = slot
            .take()
            .ok_or_else(|| LifecycleError::new_err("connection is closed"))?;
        Ok(Self {
            slot,
            connection: Some(connection),
            state,
        })
    }

    pub(crate) fn connection(&self) -> PyResult<Database<'_>> {
        self.connection
            .as_ref()
            .map(Held::database)
            .ok_or_else(|| InternalError::new_err("operation has no connection"))
    }

    pub(crate) fn connection_mut(&mut self) -> PyResult<&mut Held> {
        self.connection
            .as_mut()
            .ok_or_else(|| InternalError::new_err("operation has no connection"))
    }

    pub(crate) fn restore(mut self) {
        if self.state.ensure_open().is_ok() {
            *self.slot = self.connection.take();
        }
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            self.state.cancelled.cancel();
            connection.discard();
        }
    }
}

#[derive(Debug)]
#[pyclass(name = "_Connection", module = "pgorm._native", frozen)]
pub(crate) struct NativeConnection {
    pub(crate) state: Arc<ConnectionState>,
}

#[pymethods]
impl NativeConnection {
    #[pyo3(signature=(*, mode="default", isolation=None))]
    fn begin<'py>(
        &self,
        py: Python<'py>,
        mode: &str,
        isolation: Option<&str>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.state.pool.check_owner(py)?;
        self.state.ensure_open()?;
        let mode = crate::transactions::mode(mode, isolation)?;
        let state = self.state.clone();
        future_into_py(py, async move {
            Ok(crate::transactions::NativeTransaction {
                state: crate::transactions::begin(state, mode).await?,
            })
        })
    }

    fn execute<'py>(
        &self,
        py: Python<'py>,
        query: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        crate::results::execute(
            py,
            crate::execution::Target::Connection(self.state.clone()),
            query,
        )
    }

    fn fetch<'py>(
        &self,
        py: Python<'py>,
        query: &Bound<'_, PyAny>,
        mode: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        crate::results::fetch(
            py,
            crate::execution::Executor::Held(crate::execution::Target::Connection(
                self.state.clone(),
            )),
            query,
            mode,
        )
    }

    /// A stream of `query`'s rows, each a `Record`, or what `decoder`, a
    /// disjoint read's or a registered read's, decodes it as.
    #[pyo3(signature = (query, decoder=None))]
    fn stream<'py>(
        &self,
        py: Python<'py>,
        query: &Bound<'_, PyAny>,
        decoder: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let decoder = match decoder.filter(|decoder| !decoder.is_none()) {
            None => None,
            Some(decoder) => Some(
                if let Ok(disjoint) =
                    decoder.extract::<PyRef<'_, crate::disjoint::NativeDisjoint>>()
                {
                    disjoint.row_decode()
                } else {
                    decoder
                        .extract::<PyRef<'_, crate::entities::reads::NativeRowDecoder>>()?
                        .row_decode()
                },
            ),
        };
        crate::results::open(py, self.state.clone(), query, decoder)
    }

    fn notify<'py>(
        &self,
        py: Python<'py>,
        channel: &Bound<'_, PyAny>,
        payload: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        crate::listen::notify(
            py,
            crate::execution::Target::Connection(self.state.clone()),
            channel,
            payload,
        )
    }

    fn ping<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.pool.check_owner(py)?;
        let state = self.state.clone();
        future_into_py(py, async move {
            let operation = Operation::begin(state.clone())?;
            let db = operation.connection()?;
            let result = tokio::select! {
                _ = state.cancelled.cancelled() => return Err(LifecycleError::new_err("connection is closed")),
                _ = state.pool.cancelled.cancelled() => return Err(LifecycleError::new_err("pool is closed")),
                result = db.query_one("SELECT TRUE", &[]) => result,
            };
            operation.restore();
            let row = result.map_err(|error| database_error(error, &state.pool.secrets))?;
            row.try_get::<_, bool>(0)
                .map_err(|error| database_error(pgorm::Error::from(error), &state.pool.secrets))
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.pool.check_owner(py)?;
        self.state.close_now();
        let state = self.state.clone();
        future_into_py(py, async move {
            drop(state.slot.lock().await.take());
            Ok(())
        })
    }

    /// Close the connection and discard it, as `DatabaseConnection::discard`
    /// does, so the pool opens a fresh one in its place; an instrumented
    /// pool's is discarded through the connection it wraps.
    // [spec:pgorm:req:bindings.connections+3]
    fn discard<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.pool.check_owner(py)?;
        self.state.discard_now();
        let state = self.state.clone();
        future_into_py(py, async move {
            if let Some(connection) = state.slot.lock().await.take() {
                connection.discard();
            }
            Ok(())
        })
    }

    fn closed(&self) -> bool {
        self.state.cancelled.is_cancelled() || self.state.pool.cancelled.is_cancelled()
    }

    fn __repr__(&self) -> String {
        format!("<pgorm native connection closed={}>", self.closed())
    }
}

impl Drop for NativeConnection {
    fn drop(&mut self) {
        self.state.close_now();
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    // [spec:pgorm:req:bindings.connections+3]    the channel pgorm-migration's runner notifies
    module.add("STATEMENT_CACHE_CHANNEL", pgorm::STATEMENT_CACHE_CHANNEL)?;
    module.add_class::<NativePool>()?;
    module.add_class::<NativePoolShape>()?;
    module.add_class::<crate::hooks::NativeHookConnection>()?;
    module.add_class::<NativeConnection>()?;
    module.add_class::<crate::transactions::NativeTransaction>()?;
    Ok(())
}
