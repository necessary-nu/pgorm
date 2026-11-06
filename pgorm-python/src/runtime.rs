use crate::errors::Raise as _;
use std::{collections::BTreeMap, sync::Arc};

use crate::panics::future_into_py;
use pgorm::{ConnectionTrait, metric::InstrumentedPool};
use pgorm_bindings::Metrics;
use pyo3::prelude::*;
use tokio_util::sync::CancellationToken;

use crate::{
    config::{self, DIALECT, NativePoolShape, Owner, seconds},
    errors::{ConstructionError, InternalError, LifecycleError, Redactions},
};

/// The event loop a pool, and everything it lends, belongs to.
pub(crate) type Loop = Arc<Py<PyAny>>;
pub(crate) type PoolState = pgorm_bindings::PoolState<Loop>;
pub(crate) type ConnectionState = pgorm_bindings::ConnectionState<Loop>;
pub(crate) type Operation = pgorm_bindings::Operation<Loop>;
pub(crate) use pgorm_bindings::Held;

/// What is used from its pool's event loop alone.
pub(crate) trait Owned {
    /// Refuse to run from any event loop but the pool's.
    fn check_owner(&self, py: Python<'_>) -> PyResult<()>;
}

impl Owned for PoolState {
    fn check_owner(&self, py: Python<'_>) -> PyResult<()> {
        check_loop(py, &self.owner, "pool")
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
        drop(self.0.close_now());
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
            DIALECT,
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
                        DIALECT,
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
        let hooks: [Option<Py<PyAny>>; pgorm_bindings::Hook::ALL.len()] = hooks
            .try_into()
            .map_err(|_| pyo3::exceptions::PyTypeError::new_err("a collector has seven hooks"))?;
        let metrics = crate::metrics::metrics(
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
        self.state.ensure_open().py()?;
        let pool = self.state.clone();
        let metered = self.metered.clone();
        future_into_py(py, async move {
            Ok(NativeConnection {
                state: pool.lend(metered.as_ref()).await.py()?,
            })
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.check_owner(py)?;
        let closed = self.state.close_now();
        future_into_py(py, async move {
            closed.released().await;
            Ok(())
        })
    }

    fn closed(&self) -> bool {
        self.state.closed.is_cancelled()
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
        self.state.ensure_open().py()?;
        let mode = crate::transactions::mode(mode, isolation)?;
        let state = self.state.clone();
        future_into_py(py, async move {
            Ok(crate::transactions::NativeTransaction {
                state: crate::transactions::begin(state, mode).await?,
            })
        })
    }

    /// Open a transaction pgorm runs again from `BEGIN`, up to `retries`
    /// times, while an attempt fails with a serialization failure or a
    /// deadlock.
    #[pyo3(signature=(retries, *, mode="default", isolation=None))]
    fn begin_retrying<'py>(
        &self,
        py: Python<'py>,
        retries: u32,
        mode: &str,
        isolation: Option<&str>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.state.pool.check_owner(py)?;
        self.state.ensure_open().py()?;
        let mode = crate::transactions::mode(mode, isolation)?;
        let state = self.state.clone();
        future_into_py(py, async move {
            Ok(crate::transactions::NativeRetrying {
                inner: crate::transactions::begin_retrying(state, mode, retries)?,
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
        crate::results::open(
            py,
            crate::results::Source::Connection(self.state.clone()),
            query,
            crate::results::row_decoder(decoder)?,
        )
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
            let operation = Operation::begin(state.clone()).py()?;
            ConnectionState::run(operation, &CancellationToken::new(), async |db| {
                let row = db
                    .database()
                    .query_one("SELECT TRUE", &[])
                    .await
                    .map_err(|error| state.pool.failure(&error))?;
                row.try_get::<_, bool>(0)
                    .map_err(|error| state.pool.failure(&pgorm::Error::from(error)))
            })
            .await
            .py()
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.pool.check_owner(py)?;
        self.state.close_now();
        let state = self.state.clone();
        future_into_py(py, async move {
            state.released().await;
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
            state.discarded().await;
            Ok(())
        })
    }

    fn closed(&self) -> bool {
        self.state.is_closed()
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
    module.add_class::<crate::transactions::NativeRetrying>()?;
    Ok(())
}
