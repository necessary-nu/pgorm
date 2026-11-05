mod connection;
mod copy;
mod copy_rows;
mod db_connection;
mod lifecycle;
mod listen;
mod transaction;

pub use connection::*;
pub use copy::{CopyInSink, CopyOutStream, CopyTrait};
pub use copy_rows::{CopyColumn, CopyRowDecoder, CopyRowEncoder, CopyRowSink, CopyRowStream};
pub use db_connection::*;
pub use listen::{Listener, Notifications, connect_listener, notify};
pub use transaction::{TransactionError, TransactionTrait};
pub(crate) use transaction::{drive, rolled_back, sealed};

/// What a [`Notifications`] stream yields: the channel, the payload, and the
/// process id of the session that sent it.
// [spec:pgorm:req:conn.listen]
pub use tokio_postgres::Notification;

/// The chunk type COPY's data travels in, both ways, re-exported so writing a
/// [`CopyInSink`] needs no direct dependency on `bytes`.
// [spec:pgorm:req:conn.copy+2]
pub use bytes::Bytes;

/// The buffer [`CopyRowEncoder`] writes a binary COPY's header, rows and
/// trailer into.
// [spec:pgorm:req:conn.driver-types]
pub use bytes::BytesMut;

/// The traits [`CopyInSink`] and the streams pgorm returns implement, for
/// code that names them in a signature of its own. Reading a stream or
/// writing a sink still takes the extension traits of the `futures` crate.
// [spec:pgorm:req:conn.driver-types]
pub use futures::{Sink, Stream};

pub use tokio_postgres::{Config, IsolationLevel};

/// The row [`query_one`], [`query_opt`] and [`query_all`] return, and the
/// stream of them [`query_raw`] returns.
///
/// [`query_one`]: ConnectionTrait::query_one
/// [`query_opt`]: ConnectionTrait::query_opt
/// [`query_all`]: ConnectionTrait::query_all
/// [`query_raw`]: ConnectionTrait::query_raw
// [spec:pgorm:req:conn.driver-types]
pub use tokio_postgres::{Row, RowStream};

/// The driver's error, which [`Error::Postgres`] carries, renamed so that it
/// does not collide with pgorm's own [`Error`].
// [spec:pgorm:req:conn.driver-types]
pub use tokio_postgres::Error as PostgresError;

/// What the driver's error reports of a refusal the server sent: the
/// server's error, and the SQLSTATE that [`PostgresError::code`] returns.
// [spec:pgorm:req:conn.driver-types]
pub use tokio_postgres::error::{DbError, SqlState};

/// What [`Config`]'s setters take and its getters return.
// [spec:pgorm:req:conn.driver-types]
pub use tokio_postgres::config::{
    ChannelBinding, Host, LoadBalanceHosts, SslMode, SslNegotiation, TargetSessionAttrs,
};

/// The pool and manager knobs [`connect_with`] takes, re-exported so shaping a
/// pool never requires naming `pgorm_pool` itself.
pub use pgorm_pool::{ManagerConfig, PoolBuilder, RecyclingMethod, StatementCacheSize};

/// What [`PoolBuilder`]'s setters take, what [`DatabasePool::status`] returns,
/// and the error [`Error::Pool`] carries, with what its variants hold.
// [spec:pgorm:req:conn.driver-types]
pub use pgorm_pool::{
    Hook, HookError, PoolConfig, PoolError, QueueMode, Runtime, Status, TimeoutType, Timeouts,
};

/// The channel pgorm-migration's runner notifies once a batch that applied a
/// migration commits: the one to give
/// [`ManagerConfig::statement_cache_channel`] for a pool to clear its
/// statement caches after another process's migration.
// [spec:pgorm:req:conn.pool.statement-cache.listen+1]
pub use pgorm_pool::STATEMENT_CACHE_CHANNEL;

/// The TLS connector types [`connect_with`]'s bounds are written in, plus the
/// [`NoTls`] the simple entry points pass, re-exported for the same reason.
pub use tokio_postgres::{
    NoTls, Socket,
    tls::{MakeTlsConnect, TlsConnect},
};

use crate::error::Error;
use pgorm_pool::{BuildError, Manager, Pool};
use std::{collections::BTreeMap, convert::identity};

/// The builder rejected the pool it was asked to build. deadpool's only such
/// failure is a timeout configured without a runtime, so the message is carried
/// through verbatim.
fn pool_build_err(err: BuildError) -> Error {
    Error::Custom(format!("cannot build connection pool: {err}"))
}

/// The manager configuration the simple entry points delegate with: no
/// recycling query, the default statement-cache bound, no statement-cache
/// channel, and `tag` — `None` everywhere but [`connect_multi_with_builder`],
/// which names each pool.
///
/// Spelled out rather than taken from `ManagerConfig::default()` because these
/// are the defaults `conn.pool` fixes for those entry points, not whatever the
/// pool crate's own default happens to be.
fn simple_manager_config(tag: Option<String>) -> ManagerConfig {
    ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
        tag,
        statement_cache: StatementCacheSize::default(),
        statement_cache_channel: None,
    }
}

/// The longest channel name `LISTEN` keeps whole and `pg_notify` accepts.
const CHANNEL_BYTES: usize = 63;

/// Refuse a [`ManagerConfig::statement_cache_channel`] no notification could
/// reach: an empty name, which `LISTEN` refuses, or one longer than an
/// identifier, which it truncates while `pg_notify` refuses it whole.
// [spec:pgorm:req:conn.pool.statement-cache.listen+1]    an unusable channel is refused at construction
fn check_statement_cache_channel(manager: &ManagerConfig) -> Result<(), Error> {
    match &manager.statement_cache_channel {
        Some(channel) if channel.is_empty() || channel.len() > CHANNEL_BYTES => {
            Err(Error::Custom(format!(
                "statement_cache_channel {channel:?} is not a channel name of 1 to {CHANNEL_BYTES} bytes"
            )))
        }
        _ => Ok(()),
    }
}

/// Method to create a [DatabasePool] over every knob the pool has: the TLS
/// connector, the [`ManagerConfig`], and the [`PoolBuilder`].
///
/// This is the general entry point the other three are fixed-argument
/// shorthands for, and the only route to a pool that speaks TLS, that recycles
/// with anything but [`RecyclingMethod::Fast`], or that caches a number of
/// statements other than the default.
///
/// `tls` is a `tokio_postgres` TLS connector — [`NoTls`], or one of
/// `tokio-postgres-rustls` / `tokio-postgres-openssl`, which is where a managed
/// PostgreSQL requiring TLS is reached from.
///
/// ```no_run
/// use pgorm::{ManagerConfig, NoTls, RecyclingMethod, StatementCacheSize, connect_with};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let pool = connect_with(
///     "postgres://postgres@localhost/example".parse()?,
///     NoTls,
///     ManagerConfig {
///         recycling_method: RecyclingMethod::Verified,
///         tag: Some("reader".to_owned()),
///         statement_cache: StatementCacheSize::Disabled,
///         statement_cache_channel: Some(pgorm::STATEMENT_CACHE_CHANNEL.to_owned()),
///     },
///     |builder| builder.max_size(8),
/// )?;
/// # let _ = pool;
/// # Ok(())
/// # }
/// ```
///
/// [`ManagerConfig::statement_cache_channel`] is the one setting here that
/// costs a connection: a pool given one keeps a connection of its own
/// listening there from its first checkout, and clears its statement caches
/// whenever a notification arrives, as another process's migration sends.
///
/// # Errors
///
/// Returns [`Error::Custom`] if the builder produced by `build` cannot build a
/// pool — deadpool rejects a pool configured with timeouts but no runtime —
/// or if `statement_cache_channel` is empty or longer than the 63 bytes of an
/// identifier.
// [spec:pgorm:req:conn.pool+4]    the general entry point
#[doc(alias = "ConnectOptions")]
pub fn connect_with<T>(
    config: Config,
    tls: T,
    manager: ManagerConfig,
    build: impl FnOnce(PoolBuilder) -> PoolBuilder,
) -> Result<DatabasePool, Error>
where
    T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
    T::Stream: Sync + Send,
    T::TlsConnect: Sync + Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    check_statement_cache_channel(&manager)?;
    let mgr = Manager::from_config(config, tls, manager);
    build(Pool::builder(mgr))
        .build()
        .map(DatabasePool)
        .map_err(pool_build_err)
}

/// Method to create a [DatabasePool] on a database
///
/// Nothing is connected to here. This builds a pool and returns it; the pool
/// opens connections lazily, so a wrong host, a wrong password or a database
/// that does not exist is not reported by this call but by the first
/// [`DatabasePool::get`] that has to open a connection — and by every later
/// one, since nothing has been proven in between. Treat a returned
/// `DatabasePool` as a configuration that has not been tested yet: a caller
/// that wants to fail at startup rather than at first use should `get()` a
/// connection there and drop it.
///
/// # Panics
///
/// Panics if pgorm's own default pool configuration is rejected by the builder.
/// `config` shapes the [`Manager`], not the pool, so nothing a caller supplies
/// can reach that check: a panic here is a pgorm bug, not caller error. Use
/// [`connect_with_builder`] for a fallible pool whose shape the caller chooses,
/// or [`connect_with`] for one whose TLS and manager configuration it chooses
/// too.
// [spec:pgorm:req:conn.pool+4]
#[doc(alias = "Database")]
pub fn connect(config: Config) -> DatabasePool {
    connect_with(config, NoTls, simple_manager_config(None), identity)
        .expect("pgorm's default pool configuration builds")
}

/// Method to create a [DatabasePool], with the pool's sizing and timeouts
/// customised by `build`.
///
/// # Errors
///
/// Returns [`Error::Custom`] if the builder produced by `build` cannot build a
/// pool — deadpool rejects a pool configured with timeouts but no runtime.
// [spec:pgorm:req:conn.pool+4]    builder customization
#[doc(alias = "ConnectOptions")]
pub fn connect_with_builder(
    config: Config,
    build: impl Fn(PoolBuilder) -> PoolBuilder,
) -> Result<DatabasePool, Error> {
    connect_with(config, NoTls, simple_manager_config(None), build)
}

/// Method to create one [DatabasePool] per entry in `build`, each tagged with
/// that entry's key and customised by its builder closure, gathered into a
/// [`DatabaseMultiPool`] keyed by those tags.
///
/// # Errors
///
/// Returns [`Error::Custom`] for the first entry whose builder cannot produce a
/// pool; none of the pools is returned in that case.
// [spec:pgorm:sem:conn.pool.multi+4]    per-tag pool construction
pub fn connect_multi_with_builder(
    config: Config,
    build: BTreeMap<String, Box<dyn Fn(PoolBuilder) -> PoolBuilder>>,
) -> Result<DatabaseMultiPool, Error> {
    connect_multi_with(config, NoTls, simple_manager_config(None), build)
}

/// Method to create one [DatabasePool] per entry in `build` over every knob
/// [`connect_with`] takes: each pool connects through `tls` with `manager`'s
/// recycling method, statement cache and statement-cache channel, is tagged
/// with its entry's key in place of `manager`'s own tag, and is shaped by its
/// entry's closure. The pools are gathered into a [`DatabaseMultiPool`]
/// keyed by those tags.
///
/// This is the general entry point [`connect_multi_with_builder`] is the
/// plaintext shorthand for, and the only route to several pools that speak
/// TLS.
///
/// ```no_run
/// use std::collections::BTreeMap;
///
/// use pgorm::{ManagerConfig, NoTls, PoolBuilder, RecyclingMethod, connect_multi_with};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut shapes: BTreeMap<String, fn(PoolBuilder) -> PoolBuilder> = BTreeMap::new();
/// shapes.insert("web".to_owned(), |builder| builder.max_size(8));
/// shapes.insert("jobs".to_owned(), |builder| builder.max_size(2));
/// let manager = ManagerConfig {
///     recycling_method: RecyclingMethod::Clean,
///     ..ManagerConfig::default()
/// };
/// let pools = connect_multi_with(
///     "postgres://postgres@localhost/example".parse()?,
///     NoTls,
///     manager,
///     shapes,
/// )?;
/// assert_eq!(pools.get("jobs").map(|pool| pool.status().max_size), Some(2));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns [`Error::Custom`] for the first entry whose builder cannot produce a
/// pool, or if `manager`'s `statement_cache_channel` is no channel name; none
/// of the pools is returned in that case.
// [spec:pgorm:sem:conn.pool.multi+4]    the general multi constructor
pub fn connect_multi_with<T, F>(
    config: Config,
    tls: T,
    manager: ManagerConfig,
    build: BTreeMap<String, F>,
) -> Result<DatabaseMultiPool, Error>
where
    T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
    T::Stream: Sync + Send,
    T::TlsConnect: Sync + Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
    F: FnOnce(PoolBuilder) -> PoolBuilder,
{
    build
        .into_iter()
        .map(|(tag, build)| {
            let tagged = ManagerConfig {
                tag: Some(tag.clone()),
                ..manager.clone()
            };
            let pool = connect_with(config.clone(), tls.clone(), tagged, build)?;
            Ok((tag, pool))
        })
        .collect::<Result<_, Error>>()
        .map(DatabaseMultiPool)
}
