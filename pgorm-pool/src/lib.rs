#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(
    nonstandard_style,
    rust_2018_idioms,
    rustdoc::broken_intra_doc_links,
    rustdoc::private_intra_doc_links
)]
#![forbid(non_ascii_idents, unsafe_code)]
#![warn(
    deprecated_in_future,
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    unreachable_pub,
    unused_import_braces,
    unused_labels,
    unused_lifetimes,
    unused_qualifications,
    unused_results
)]

mod cache_listener;
mod config;
mod copy;
mod generic_client;
mod raw;
mod transaction;

use std::{
    borrow::Cow,
    collections::HashMap,
    fmt,
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use bytes::Buf;
use cache_listener::CacheListener;
use config::Tag;
use deadpool::managed;
#[cfg(not(target_arch = "wasm32"))]
use tokio::spawn;
use tokio::task::JoinHandle;
use tokio_postgres::{
    Client as PgClient, Config as PgConfig, CopyInSink, Error, Statement, ToStatement, types::Type,
};

#[cfg(not(target_arch = "wasm32"))]
use tokio_postgres::{
    Socket,
    tls::{MakeTlsConnect, TlsConnect},
};

pub use tokio_postgres;

pub use self::config::{
    ChannelBinding, Config, ConfigError, LoadBalanceHosts, ManagerConfig, RecyclingMethod, SslMode,
    StatementCacheSize, TargetSessionAttrs,
};

pub use self::cache_listener::STATEMENT_CACHE_CHANNEL;
pub use self::generic_client::GenericClient;
pub use self::raw::{RawConnection, Unpooled};
pub use self::transaction::{CommitError, Transaction, TransactionBuilder};

pub use deadpool::managed::reexports::*;
deadpool::managed_reexports!(
    "tokio_postgres",
    Manager,
    managed::Object<Manager>,
    Error,
    ConfigError
);

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Type alias for [`Object`]
pub type Client = Object;

type RecycleResult = managed::RecycleResult<Error>;
type RecycleError = managed::RecycleError<Error>;

// #[derive(Debug, Clone, Default)]
// pub struct Status {
//     pub active_connections: Arc<AtomicUsize>,
// }

/// [`Manager`] for creating and recycling PostgreSQL connections.
///
/// [`Manager`]: managed::Manager
pub struct Manager {
    tag: Tag,
    // status: Arc<Status>,
    config: ManagerConfig,
    pg_config: PgConfig,
    connect: Arc<dyn Connect>,
    /// [`StatementCaches`] of [`Client`]s handed out by the [`Pool`].
    pub statement_caches: StatementCaches,
    listener: CacheListener,
}

impl Manager {
    #[cfg(not(target_arch = "wasm32"))]
    /// Creates a new [`Manager`] using the given [`tokio_postgres::Config`] and
    /// `tls` connector.
    pub fn new<T>(pg_config: tokio_postgres::Config, tls: T) -> Self
    where
        T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
        T::Stream: Sync + Send,
        T::TlsConnect: Sync + Send,
        <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
    {
        Self::from_config(pg_config, tls, ManagerConfig::default())
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Create a new [`Manager`] using the given [`tokio_postgres::Config`], and
    /// `tls` connector and [`ManagerConfig`].
    pub fn from_config<T>(pg_config: tokio_postgres::Config, tls: T, config: ManagerConfig) -> Self
    where
        T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
        T::Stream: Sync + Send,
        T::TlsConnect: Sync + Send,
        <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
    {
        Self::from_connect(pg_config, ConfigConnectImpl { tls }, config)
    }

    /// Create a new [`Manager`] using the given [`tokio_postgres::Config`], and
    /// `connect` impl and [`ManagerConfig`].
    pub fn from_connect(
        pg_config: tokio_postgres::Config,
        connect: impl Connect + 'static,
        config: ManagerConfig,
    ) -> Self {
        Self {
            tag: config
                .tag
                .as_deref()
                .map(|x| Tag(Arc::new(x.to_string())))
                .unwrap_or_default(),
            config,
            pg_config,
            // status: Arc::new(Status::default()),
            connect: Arc::new(connect),
            statement_caches: StatementCaches::default(),
            listener: CacheListener::default(),
        }
    }

    /// The tag identifying this manager's pool, either the one given in
    /// [`ManagerConfig`] or a generated `default-N`.
    pub fn tag(&self) -> Arc<String> {
        self.tag.0.clone()
    }

    /// Open a connection with this manager's configuration and connector,
    /// outside the pool: it is not counted against the pool's size, gets no
    /// statement cache and is never recycled, and its I/O half is returned
    /// undriven (see [`Connect::connect_raw`]).
    // [spec:pgorm:req:conn.listen]    a pool's listener connects as its connections do
    pub async fn connect_raw(&self) -> Result<Unpooled, Error> {
        self.connect.connect_raw(&self.pg_config).await
    }

    /// Close the connection listening on
    /// [`ManagerConfig::statement_cache_channel`] and end its task, or keep
    /// one from opening if the pool has not opened a connection yet.
    ///
    /// [`Pool::close`] does not reach the manager, so a pool closed without
    /// this keeps listening until it is dropped; pgorm's
    /// `DatabasePool::close` calls both.
    // [spec:pgorm:req:conn.pool.statement-cache.listen+1]    closing the pool ends the listener
    pub fn stop_statement_cache_listener(&self) {
        self.listener.stop();
    }

    /// Start listening on [`ManagerConfig::statement_cache_channel`], once.
    fn start_statement_cache_listener(&self) {
        let Some(channel) = &self.config.statement_cache_channel else {
            return;
        };
        self.listener.start(|| {
            cache_listener::listen_for_clears(
                self.connect.clone(),
                self.pg_config.clone(),
                channel.clone(),
                self.statement_caches.generation.clone(),
            )
        });
    }
}

// [spec:pgorm:req:conn.pool.statement-cache.listen+1]    a dropped pool does not leak the listener
impl Drop for Manager {
    fn drop(&mut self) {
        self.listener.stop();
    }
}

impl fmt::Debug for Manager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Manager")
            .field("config", &self.config)
            .field("pg_config", &self.pg_config)
            // .field("status", &self.status)
            .field("tag", &self.tag)
            //.field("connect", &self.connect)
            .field("statement_caches", &self.statement_caches)
            .finish()
    }
}

// [spec:pgorm:sem:conn.pool.lifecycle+3]    create/recycle/detach
impl managed::Manager for Manager {
    type Type = ClientWrapper;
    type Error = Error;

    async fn create(&self) -> Result<ClientWrapper, Error> {
        // Started ahead of the connection, so that its first `LISTEN`, which
        // clears, tends to precede the first statement prepared.
        self.start_statement_cache_listener();
        let (client, conn_task) = self.connect.connect(&self.pg_config).await?;
        let mut client_wrapper =
            ClientWrapper::with_cache_size(client, conn_task, self.config.statement_cache);
        self.statement_caches.attach(&mut client_wrapper);
        Ok(client_wrapper)
    }

    async fn recycle(&self, client: &mut ClientWrapper, _: &Metrics) -> RecycleResult {
        if client.is_closed() {
            tracing::warn!(target: "deadpool.postgres", "Connection could not be recycled: Connection closed");
            return Err(RecycleError::message("Connection closed"));
        }
        self.statement_caches.bring_up_to_date(client);
        match self.config.recycling_method.query() {
            Some(sql) => match client.simple_query(sql).await {
                Ok(_) => Ok(()),
                Err(e) => {
                    tracing::warn!(target: "deadpool.postgres", "Connection could not be recycled: {}", e);
                    Err(e.into())
                }
            },
            None => Ok(()),
        }
    }

    fn detach(&self, object: &mut ClientWrapper) {
        // let _ = self.status.active_connections.fetch_sub(1, Ordering::Relaxed);
        self.statement_caches.detach(&object.statement_cache);
    }
}

/// Describes a mechanism for establishing a connection to a PostgreSQL
/// server via `tokio_postgres`.
pub trait Connect: Sync + Send {
    /// Establishes a new `tokio_postgres` connection, returning
    /// the associated `Client` and a `JoinHandle` to a tokio task
    /// for processing the connection.
    fn connect(
        &self,
        pg_config: &PgConfig,
    ) -> BoxFuture<'_, Result<(PgClient, JoinHandle<()>), Error>>;

    /// Establishes a new `tokio_postgres` connection and returns its I/O half
    /// undriven, for an owner that polls it itself to see the asynchronous
    /// messages a spawned connection task discards.
    fn connect_raw(&self, pg_config: &PgConfig) -> BoxFuture<'_, Result<Unpooled, Error>>;
}

#[cfg(not(target_arch = "wasm32"))]
/// Provides an implementation of [`Connect`] that establishes the connection
/// using the `tokio_postgres` configuration itself.
#[derive(Debug)]
pub struct ConfigConnectImpl<T>
where
    T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
    T::Stream: Sync + Send,
    T::TlsConnect: Sync + Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    /// The TLS connector to use for the connection.
    pub tls: T,
}

#[cfg(not(target_arch = "wasm32"))]
impl<T> Connect for ConfigConnectImpl<T>
where
    T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
    T::Stream: Sync + Send,
    T::TlsConnect: Sync + Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    fn connect(
        &self,
        pg_config: &PgConfig,
    ) -> BoxFuture<'_, Result<(PgClient, JoinHandle<()>), Error>> {
        let tls = self.tls.clone();
        let pg_config = pg_config.clone();
        Box::pin(async move {
            let fut = pg_config.connect(tls);
            let (client, connection) = fut.await?;
            let conn_task = spawn(async move {
                if let Err(e) = connection.await {
                    tracing::warn!(target: "deadpool.postgres", "Connection error: {}", e);
                }
            });
            Ok((client, conn_task))
        })
    }

    fn connect_raw(&self, pg_config: &PgConfig) -> BoxFuture<'_, Result<Unpooled, Error>> {
        let tls = self.tls.clone();
        let pg_config = pg_config.clone();
        Box::pin(async move {
            let (client, connection) = pg_config.connect(tls).await?;
            Ok((client, Box::new(connection) as Box<dyn RawConnection>))
        })
    }
}

/// Structure holding a reference to all [`StatementCache`]s and providing
/// access for clearing all caches and removing single statements from them.
// [spec:pgorm:sem:conn.pool.statement-cache+3]    manager-level registry
#[derive(Default, Debug)]
pub struct StatementCaches {
    caches: Mutex<Vec<Weak<StatementCache>>>,
    /// Counts [`StatementCaches::clear_before_reuse`] calls and the clears
    /// the statement-cache listener hears. A connection whose cache was last
    /// cleared at an older count is cleared before it is lent. Shared with the
    /// listener's task, which must not hold the manager.
    generation: Arc<AtomicU64>,
}

impl StatementCaches {
    /// Registers a newly created connection's cache. The cache is empty, so it
    /// is current with every clear asked for so far.
    fn attach(&self, client: &mut ClientWrapper) {
        client.cache_generation = self.generation.load(Ordering::Acquire);
        let cache = Arc::downgrade(&client.statement_cache);
        self.caches.lock().unwrap().push(cache);
    }

    fn detach(&self, cache: &Arc<StatementCache>) {
        let cache = Arc::downgrade(cache);
        self.caches.lock().unwrap().retain(|sc| !sc.ptr_eq(&cache));
    }

    /// Clears [`StatementCache`] of all connections which were handed out by a
    /// [`Manager`].
    ///
    /// Every cache is cleared at once, including those of connections that are
    /// checked out and running statements. A statement whose preparation was
    /// already in flight can land in a cache after the clear has passed it, so
    /// after DDL prefer [`StatementCaches::clear_before_reuse`].
    pub fn clear(&self) {
        let caches = self.caches.lock().unwrap();
        for cache in caches.iter() {
            if let Some(cache) = cache.upgrade() {
                cache.clear();
            }
        }
    }

    /// Marks the [`StatementCache`] of every connection handed out so far as
    /// stale: an idle connection's is cleared before the pool next lends it,
    /// and one checked out at the time keeps its statements until it is
    /// returned and is cleared before it is lent again.
    ///
    /// A cleared statement is closed on the server when its last handle drops,
    /// which for a statement no `Row` still holds is the clear itself, so the
    /// server-side prepared statements are deallocated rather than left until
    /// the connection closes.
    ///
    /// Call it after DDL that changes what a cached plan depends on: PostgreSQL
    /// re-plans most such changes itself, but not a domain's default.
    // [spec:pgorm:req:conn.pool.statement-cache.clear+1]    bump the generation
    pub fn clear_before_reuse(&self) {
        cache_listener::advance(&self.generation);
    }

    /// Clears `client`'s cache if a [`StatementCaches::clear_before_reuse`]
    /// has been asked for since it was last cleared. Called only while the
    /// pool holds the connection idle, so no statement of its owner's can be
    /// mid-preparation.
    // [spec:pgorm:req:conn.pool.statement-cache.clear+1]    an idle connection catches up before it is lent
    fn bring_up_to_date(&self, client: &mut ClientWrapper) {
        let current = self.generation.load(Ordering::Acquire);
        if client.cache_generation != current {
            client.statement_cache.clear();
            client.cache_generation = current;
        }
    }

    /// Removes statement from all caches which were handed out by a
    /// [`Manager`].
    pub fn remove(&self, query: &str, types: &[Type]) {
        let caches = self.caches.lock().unwrap();
        for cache in caches.iter() {
            if let Some(cache) = cache.upgrade() {
                drop(cache.remove(query, types));
            }
        }
    }
}

impl fmt::Debug for StatementCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientWrapper")
            //.field("map", &self.map)
            .field("size", &self.size)
            .finish()
    }
}

// Allows us to use owned keys in a `HashMap`, but still be able to call `get`
// with borrowed keys instead of allocating them each time.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct StatementCacheKey<'a> {
    query: Cow<'a, str>,
    types: Cow<'a, [Type]>,
}

/// Representation of a cache of [`Statement`]s.
///
/// [`StatementCache`] is bound to one [`Client`], and [`Statement`]s generated
/// by that [`Client`] must not be used with other [`Client`]s.
///
/// It can be used like that:
/// ```rust,ignore
/// let client = pool.get().await?;
/// let stmt = client
///     .statement_cache
///     .prepare(&client, "SELECT 1")
///     .await;
/// let rows = client.query(stmt, &[]).await?;
/// ...
/// ```
///
/// Normally, you probably want to use the [`ClientWrapper::prepare_cached()`]
/// and [`ClientWrapper::prepare_typed_cached()`] methods instead (or the
/// similar ones on [`Transaction`]).
// [spec:pgorm:sem:conn.pool.statement-cache+3]
pub struct StatementCache {
    map: RwLock<HashMap<StatementCacheKey<'static>, Statement>>,
    size: AtomicUsize,
    capacity: StatementCacheSize,
}

impl StatementCache {
    fn new(capacity: StatementCacheSize) -> Self {
        Self {
            map: RwLock::new(HashMap::new()),
            size: AtomicUsize::new(0),
            capacity,
        }
    }

    /// Returns current size of this [`StatementCache`].
    pub fn size(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }

    /// Clears this [`StatementCache`].
    ///
    /// **Important:** This only clears the [`StatementCache`] of one [`Client`]
    /// instance. If you want to clear the [`StatementCache`] of all [`Client`]s
    /// you should be calling `pool.manager().statement_caches.clear()` instead.
    pub fn clear(&self) {
        let mut map = self.map.write().unwrap();
        map.clear();
        self.size.store(0, Ordering::Relaxed);
    }

    /// Removes a [`Statement`] from this [`StatementCache`].
    ///
    /// **Important:** This only removes a [`Statement`] from one [`Client`]
    /// cache. If you want to remove a [`Statement`] from all
    /// [`StatementCaches`] you should be calling
    /// `pool.manager().statement_caches.remove()` instead.
    pub fn remove(&self, query: &str, types: &[Type]) -> Option<Statement> {
        let key = StatementCacheKey {
            query: Cow::Owned(query.to_owned()),
            types: Cow::Owned(types.to_owned()),
        };
        let mut map = self.map.write().unwrap();
        let removed = map.remove(&key);
        if removed.is_some() {
            let _ = self.size.fetch_sub(1, Ordering::Relaxed);
        }
        removed
    }

    /// Returns a [`Statement`] from this [`StatementCache`].
    fn get(&self, query: &str, types: &[Type]) -> Option<Statement> {
        let key = StatementCacheKey {
            query: Cow::Borrowed(query),
            types: Cow::Borrowed(types),
        };
        self.map.read().unwrap().get(&key).map(ToOwned::to_owned)
    }

    /// Inserts a [`Statement`] into this [`StatementCache`], evicting an
    /// existing entry first if the cache is already at its capacity.
    ///
    /// The victim is whichever entry the map yields first, not the least
    /// recently used: recency would have to be written on every lookup, turning
    /// the read lock a cache hit takes today into an exclusive one. What the
    /// bound is for is stopping unbounded growth, and any victim achieves that.
    // [spec:pgorm:req:conn.pool.statement-cache.bound+1]    evict one to make room
    fn insert(&self, query: &str, types: &[Type], stmt: Statement) {
        let Some(capacity) = self.capacity.limit() else {
            return;
        };
        let key = StatementCacheKey {
            query: Cow::Owned(query.to_owned()),
            types: Cow::Owned(types.to_owned()),
        };

        // The evicted statement is dropped after the guard, because dropping the
        // last handle to a `Statement` queues a `Close` on the connection.
        let evicted = {
            let mut map = self.map.write().unwrap();
            let victim = if map.len() >= capacity.get() && !map.contains_key(&key) {
                map.keys().next().cloned()
            } else {
                None
            };
            let evicted = victim.and_then(|victim| map.remove(&victim));
            let _ = map.insert(key, stmt);
            self.size.store(map.len(), Ordering::Relaxed);
            evicted
        };
        drop(evicted);
    }

    /// Creates a new prepared [`Statement`] using this [`StatementCache`], if
    /// possible.
    ///
    /// See [`tokio_postgres::Client::prepare()`].
    pub async fn prepare(&self, client: &PgClient, query: &str) -> Result<Statement, Error> {
        self.prepare_typed(client, query, &[]).await
    }

    /// Creates a new prepared [`Statement`] with specifying its [`Type`]s
    /// explicitly using this [`StatementCache`], if possible.
    ///
    /// Under [`StatementCacheSize::Disabled`] nothing is stored or looked up,
    /// so this is [`tokio_postgres::Client::prepare_typed()`] itself.
    ///
    /// See [`tokio_postgres::Client::prepare_typed()`].
    // [spec:pgorm:req:conn.pool.statement-cache.bound+1]    Disabled prepares afresh
    pub async fn prepare_typed(
        &self,
        client: &PgClient,
        query: &str,
        types: &[Type],
    ) -> Result<Statement, Error> {
        if self.capacity.limit().is_none() {
            return client.prepare_typed(query, types).await;
        }

        match self.get(query, types) {
            Some(statement) => Ok(statement),
            None => {
                let stmt = client.prepare_typed(query, types).await?;
                self.insert(query, types, stmt.clone());
                Ok(stmt)
            }
        }
    }
}

/// Wrapper around [`tokio_postgres::Client`] with a [`StatementCache`].
#[derive(Debug)]
pub struct ClientWrapper {
    /// Original [`PgClient`].
    client: PgClient,

    /// A handle to the connection task that should be aborted when the client
    /// wrapper is dropped.
    conn_task: JoinHandle<()>,

    /// [`StatementCache`] of this client.
    pub statement_cache: Arc<StatementCache>,

    /// The [`StatementCaches`] generation this client's cache was last
    /// cleared at.
    cache_generation: u64,
}

impl ClientWrapper {
    /// Create a new [`ClientWrapper`] instance using the given
    /// [`tokio_postgres::Client`] and handle to the connection task, with a
    /// statement cache of the default size.
    #[must_use]
    pub fn new(client: PgClient, conn_task: JoinHandle<()>) -> Self {
        Self::with_cache_size(client, conn_task, StatementCacheSize::default())
    }

    /// [`ClientWrapper::new`] with the size of the connection's
    /// [`StatementCache`] chosen explicitly, as [`ManagerConfig`] does.
    // [spec:pgorm:req:conn.pool.statement-cache.bound+1]    per-connection capacity
    #[must_use]
    pub fn with_cache_size(
        client: PgClient,
        conn_task: JoinHandle<()>,
        cache_size: StatementCacheSize,
    ) -> Self {
        Self {
            client,
            conn_task,
            statement_cache: Arc::new(StatementCache::new(cache_size)),
            cache_generation: 0,
        }
    }

    /// Like [`tokio_postgres::Client::prepare()`], but uses an existing
    /// [`Statement`] from the [`StatementCache`] if possible.
    pub async fn prepare_cached(&self, query: &str) -> Result<Statement, Error> {
        self.statement_cache.prepare(&self.client, query).await
    }

    /// Like [`tokio_postgres::Client::prepare_typed()`], but uses an
    /// existing [`Statement`] from the [`StatementCache`] if possible.
    pub async fn prepare_typed_cached(
        &self,
        query: &str,
        types: &[Type],
    ) -> Result<Statement, Error> {
        self.statement_cache
            .prepare_typed(&self.client, query, types)
            .await
    }

    /// Like [`tokio_postgres::Client::transaction()`], but returns a wrapped
    /// [`Transaction`] with a [`StatementCache`].
    #[allow(unused_lifetimes)] // false positive
    pub async fn transaction(&mut self) -> Result<Transaction<'_>, Error> {
        Ok(Transaction {
            txn: PgClient::transaction(&mut self.client).await?,
            statement_cache: self.statement_cache.clone(),
            connection: self.conn_task.abort_handle(),
            savepoint: false,
        })
    }

    /// Like [`tokio_postgres::Client::build_transaction()`], but creates a
    /// wrapped [`Transaction`] with a [`StatementCache`].
    pub fn build_transaction(&mut self) -> TransactionBuilder<'_> {
        TransactionBuilder {
            builder: self.client.build_transaction(),
            statement_cache: self.statement_cache.clone(),
            connection: self.conn_task.abort_handle(),
        }
    }

    /// Like [`tokio_postgres::Client::copy_in()`], but a COPY the server
    /// refuses before its data flow starts closes the connection, because
    /// tokio-postgres leaves an answer behind that the next request would read
    /// as its own. Pass a prepared [`Statement`]: preparing is a request of its
    /// own, and a statement refused there leaves the connection intact.
    // [spec:pgorm:req:conn.copy+2]
    pub async fn copy_in<T, U>(&self, statement: &T) -> Result<CopyInSink<U>, Error>
    where
        T: ?Sized + ToStatement,
        U: Buf + 'static + Send,
    {
        copy::guarded(
            &self.conn_task.abort_handle(),
            self.client.copy_in(statement),
        )
        .await
    }
}

impl Deref for ClientWrapper {
    type Target = PgClient;

    fn deref(&self) -> &PgClient {
        &self.client
    }
}

impl DerefMut for ClientWrapper {
    fn deref_mut(&mut self) -> &mut PgClient {
        &mut self.client
    }
}

// [spec:pgorm:sem:conn.pool.lifecycle+3]    connection task abort on drop
impl Drop for ClientWrapper {
    fn drop(&mut self) {
        self.conn_task.abort()
    }
}
