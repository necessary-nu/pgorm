//! Configuration used for [`Pool`] creation.

use std::{
    borrow::Cow,
    env, fmt,
    net::IpAddr,
    num::NonZeroUsize,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use pgorm_driver::config::{
    ChannelBinding as PgChannelBinding, LoadBalanceHosts as PgLoadBalanceHosts,
    SslMode as PgSslMode, TargetSessionAttrs as PgTargetSessionAttrs,
};

#[cfg(not(target_arch = "wasm32"))]
use super::Pool;
#[cfg(not(target_arch = "wasm32"))]
use crate::{CreatePoolError, PoolBuilder};
#[cfg(not(target_arch = "wasm32"))]
use pgorm_driver::{
    Socket,
    tls::{MakeTlsConnect, TlsConnect},
};

use super::PoolConfig;

/// Configuration object.
///
/// # Example (from environment)
///
/// By enabling the `serde` feature you can read the configuration using the
/// [`config`](https://crates.io/crates/config) crate as following:
/// ```env
/// PG__HOST=pg.example.com
/// PG__USER=john_doe
/// PG__PASSWORD=topsecret
/// PG__DBNAME=example
/// PG__POOL__MAX_SIZE=16
/// PG__POOL__TIMEOUTS__WAIT__SECS=5
/// PG__POOL__TIMEOUTS__WAIT__NANOS=0
/// ```
/// ```rust
/// #[derive(serde::Deserialize, serde::Serialize)]
/// struct Config {
///     pg: pgorm_pool::Config,
/// }
/// impl Config {
///     pub fn from_env() -> Result<Self, config::ConfigError> {
///         let mut cfg = config::Config::builder()
///            .add_source(config::Environment::default().separator("__"))
///            .build()?;
///            cfg.try_deserialize()
///     }
/// }
/// ```
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Config {
    /// Initialize the configuration by parsing the URL first.
    ///
    /// Most of the other options then override what the URL set, because
    /// their `pgorm_driver::Config` setters replace. Four do not: `host`,
    /// `hosts`, `hostaddr`, `hostaddrs`, `port` and `ports` all reach setters
    /// that *push*, so they are added to whatever the URL already supplied.
    /// That is the point for hosts — a multi-host configuration is written by
    /// adding to it — but it means a URL that already names a port plus an
    /// explicit `port` yields two ports for one host, which tokio-postgres
    /// refuses at connect time with *invalid number of ports* rather than
    /// choosing between them. Write the port in the URL or in `port`, not in
    /// both.
    pub url: Option<String>,
    /// See [`pgorm_driver::Config::user`].
    ///
    /// Left unset — here and in `url` — the connection falls back to the
    /// `USER` environment variable, so the user a connection authenticates as
    /// can come from the process environment rather than from this struct.
    /// See [`Config::get_pg_config`].
    pub user: Option<String>,
    /// See [`pgorm_driver::Config::password`].
    pub password: Option<String>,
    /// See [`pgorm_driver::Config::dbname`].
    pub dbname: Option<String>,
    /// See [`pgorm_driver::Config::options`].
    pub options: Option<String>,
    /// See [`pgorm_driver::Config::application_name`].
    pub application_name: Option<String>,
    /// See [`pgorm_driver::Config::ssl_mode`].
    pub ssl_mode: Option<SslMode>,
    /// This is similar to [`Config::hosts`] but only allows one host to be
    /// specified.
    ///
    /// Unlike [`pgorm_driver::Config`] this structure differentiates between
    /// one host and more than one host. This makes it possible to store this
    /// configuration in an environment variable.
    ///
    /// See [`pgorm_driver::Config::host`].
    pub host: Option<String>,
    /// See [`pgorm_driver::Config::host`].
    pub hosts: Option<Vec<String>>,
    /// See [`pgorm_driver::Config::hostaddr`].
    pub hostaddr: Option<IpAddr>,
    /// See [`pgorm_driver::Config::hostaddr`].
    pub hostaddrs: Option<Vec<IpAddr>>,
    /// This is similar to [`Config::ports`] but only allows one port to be
    /// specified.
    ///
    /// Unlike [`pgorm_driver::Config`] this structure differentiates between
    /// one port and more than one port. This makes it possible to store this
    /// configuration in an environment variable.
    ///
    /// See [`pgorm_driver::Config::port`].
    pub port: Option<u16>,
    /// See [`pgorm_driver::Config::port`].
    pub ports: Option<Vec<u16>>,
    /// See [`pgorm_driver::Config::connect_timeout`].
    pub connect_timeout: Option<Duration>,
    /// See [`pgorm_driver::Config::keepalives`].
    pub keepalives: Option<bool>,
    #[cfg(not(target_arch = "wasm32"))]
    /// See [`pgorm_driver::Config::keepalives_idle`].
    pub keepalives_idle: Option<Duration>,
    /// See [`pgorm_driver::Config::target_session_attrs`].
    pub target_session_attrs: Option<TargetSessionAttrs>,
    /// See [`pgorm_driver::Config::channel_binding`].
    pub channel_binding: Option<ChannelBinding>,
    /// See [`pgorm_driver::Config::load_balance_hosts`].
    pub load_balance_hosts: Option<LoadBalanceHosts>,

    /// [`Manager`] configuration.
    ///
    /// [`Manager`]: super::Manager
    pub manager: Option<ManagerConfig>,

    /// [`Pool`] configuration.
    pub pool: Option<PoolConfig>,
}

/// A value the [`fmt::Debug`] impl refuses to print, spelled `_` as
/// [`pgorm_driver::Config`] spells it.
struct Redaction;

impl fmt::Debug for Redaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("_")
    }
}

/// [`Config::url`] with its credentials removed.
///
/// The field holds whatever [`pgorm_driver::Config::from_str`] accepts, which
/// is two spellings. In a URL the credentials are the `userinfo` before the
/// authority's `@`, and everything after it — host, port, database, options —
/// is what makes the printed value worth having, so only the `userinfo` is
/// replaced. A libpq keyword/value string has no such delimiter and its quoting
/// and escaping are not worth re-implementing to find one, so a string that
/// mentions `password` at all is withheld whole.
// [spec:pgorm:sem:conn.pool.config-redaction+1]    url credentials
fn redact_url(url: &str) -> Cow<'_, str> {
    let Some(scheme_end) = url.find("://") else {
        return if url.contains("password") {
            Cow::Borrowed("_")
        } else {
            Cow::Borrowed(url)
        };
    };

    let authority_start = scheme_end + "://".len();
    let authority_end = url[authority_start..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |offset| authority_start + offset);

    match url[authority_start..authority_end].rfind('@') {
        Some(at) => Cow::Owned(format!(
            "{}_{}",
            &url[..authority_start],
            &url[authority_start + at..]
        )),
        None => Cow::Borrowed(url),
    }
}

/// Hand-written rather than derived because two fields carry credentials, and
/// the type is built to be deserialized from the environment — the crate's own
/// example fills it from `PG__PASSWORD` — so a derived impl would put the
/// password in whatever log a `?cfg` reaches.
// [spec:pgorm:sem:conn.pool.config-redaction+1]
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut config_dbg = &mut f.debug_struct("Config");
        config_dbg = config_dbg
            .field("url", &self.url.as_deref().map(redact_url))
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| Redaction))
            .field("dbname", &self.dbname)
            .field("options", &self.options)
            .field("application_name", &self.application_name)
            .field("ssl_mode", &self.ssl_mode)
            .field("host", &self.host)
            .field("hosts", &self.hosts)
            .field("hostaddr", &self.hostaddr)
            .field("hostaddrs", &self.hostaddrs)
            .field("port", &self.port)
            .field("ports", &self.ports)
            .field("connect_timeout", &self.connect_timeout)
            .field("keepalives", &self.keepalives);

        #[cfg(not(target_arch = "wasm32"))]
        {
            config_dbg = config_dbg.field("keepalives_idle", &self.keepalives_idle);
        }

        config_dbg
            .field("target_session_attrs", &self.target_session_attrs)
            .field("channel_binding", &self.channel_binding)
            .field("load_balance_hosts", &self.load_balance_hosts)
            .field("manager", &self.manager)
            .field("pool", &self.pool)
            .finish()
    }
}

/// This error is returned if there is something wrong with the configuration
#[derive(Debug)]
#[non_exhaustive]
pub enum ConfigError {
    /// This variant is returned if the `url` is invalid
    InvalidUrl(pgorm_driver::Error),
    /// This variant is returned if the `dbname` is missing from the config
    DbnameMissing,
    /// This variant is returned if the `dbname` contains an empty string
    DbnameEmpty,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(e) => write!(f, "configuration property \"url\" is invalid: {}", e),
            Self::DbnameMissing => write!(f, "configuration property \"dbname\" not found"),
            Self::DbnameEmpty => write!(
                f,
                "configuration property \"dbname\" contains an empty string",
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Create a new [`Config`] instance with default values. This function is
    /// identical to [`Config::default()`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Creates a new [`Pool`] using this [`Config`].
    ///
    /// # Errors
    ///
    /// See [`CreatePoolError`] for details.
    pub fn create_pool<T>(&self, tls: T) -> Result<Pool, CreatePoolError>
    where
        T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
        T::Stream: Sync + Send,
        T::TlsConnect: Sync + Send,
        <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
    {
        use deadpool::Runtime;

        let builder = self.builder(tls).map_err(CreatePoolError::Config)?;
        builder
            .runtime(Runtime::Tokio1)
            .build()
            .map_err(CreatePoolError::Build)
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Creates a new [`PoolBuilder`] using this [`Config`].
    ///
    /// # Errors
    ///
    /// See [`ConfigError`] and [`pgorm_driver::Error`] for details.
    pub fn builder<T>(&self, tls: T) -> Result<PoolBuilder, ConfigError>
    where
        T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
        T::Stream: Sync + Send,
        T::TlsConnect: Sync + Send,
        <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
    {
        let pg_config = self.get_pg_config()?;
        let manager_config = self.get_manager_config();
        let manager = crate::Manager::from_config(pg_config, tls, manager_config);
        let pool_config = self.get_pool_config();
        Ok(Pool::builder(manager).config(pool_config))
    }

    /// Returns [`pgorm_driver::Config`] which can be used to connect to
    /// the database.
    ///
    /// Every field this struct declares is applied here. A field carried but
    /// never forwarded would be a setting the caller wrote and the connection
    /// never saw, which for `target_session_attrs` and `channel_binding` means
    /// silently connecting to a standby or without the binding that was asked
    /// for.
    ///
    /// Two things this reads are not fields of this struct, and both are
    /// inherited behaviour that callers may be relying on. When neither
    /// [`user`](Self::user) nor `url` supplies a non-empty user, the `USER`
    /// environment variable is read and used — so the connecting user can
    /// come from the process environment, and a deployment that expects one
    /// user gets another by changing nothing but the environment. When the
    /// configuration names no host at all, Unix targets add the three
    /// conventional socket directories (`/run/postgresql`,
    /// `/var/run/postgresql`, `/tmp`) and others add `127.0.0.1`.
    ///
    /// How `url` and the individual fields combine is documented on
    /// [`url`](Self::url): most fields replace what the URL set, but the host,
    /// hostaddr and port families are added to it.
    // [spec:pgorm:req:conn.pool.config-forwarding+2]
    #[allow(unused_results)]
    pub fn get_pg_config(&self) -> Result<pgorm_driver::Config, ConfigError> {
        let mut cfg = if let Some(url) = &self.url {
            pgorm_driver::Config::from_str(url).map_err(ConfigError::InvalidUrl)?
        } else {
            pgorm_driver::Config::new()
        };
        if let Some(user) = self.user.as_ref().filter(|s| !s.is_empty()) {
            cfg.user(user.as_str());
        }
        if !cfg.get_user().is_some_and(|u| !u.is_empty())
            && let Ok(user) = env::var("USER")
        {
            cfg.user(&user);
        }
        if let Some(password) = &self.password {
            cfg.password(password);
        }
        if let Some(dbname) = self.dbname.as_ref().filter(|s| !s.is_empty()) {
            cfg.dbname(dbname);
        }
        match cfg.get_dbname() {
            None => {
                return Err(ConfigError::DbnameMissing);
            }
            Some("") => {
                return Err(ConfigError::DbnameEmpty);
            }
            _ => {}
        }
        if let Some(options) = &self.options {
            cfg.options(options.as_str());
        }
        if let Some(application_name) = &self.application_name {
            cfg.application_name(application_name.as_str());
        }
        if let Some(host) = &self.host {
            cfg.host(host.as_str());
        }
        if let Some(hosts) = &self.hosts {
            for host in hosts.iter() {
                cfg.host(host.as_str());
            }
        }
        if cfg.get_hosts().is_empty() {
            // Systems that support it default to unix domain sockets.
            #[cfg(unix)]
            {
                cfg.host_path("/run/postgresql");
                cfg.host_path("/var/run/postgresql");
                cfg.host_path("/tmp");
            }
            // Windows and other systems use 127.0.0.1 instead.
            #[cfg(not(unix))]
            cfg.host("127.0.0.1");
        }
        if let Some(hostaddr) = self.hostaddr {
            cfg.hostaddr(hostaddr);
        }
        if let Some(hostaddrs) = &self.hostaddrs {
            for hostaddr in hostaddrs {
                cfg.hostaddr(*hostaddr);
            }
        }
        if let Some(port) = self.port {
            cfg.port(port);
        }
        if let Some(ports) = &self.ports {
            for port in ports.iter() {
                cfg.port(*port);
            }
        }
        if let Some(connect_timeout) = self.connect_timeout {
            cfg.connect_timeout(connect_timeout);
        }
        if let Some(keepalives) = self.keepalives {
            cfg.keepalives(keepalives);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(keepalives_idle) = self.keepalives_idle {
            cfg.keepalives_idle(keepalives_idle);
        }
        if let Some(mode) = self.ssl_mode {
            cfg.ssl_mode(mode.into());
        }
        if let Some(target_session_attrs) = self.target_session_attrs {
            cfg.target_session_attrs(target_session_attrs.into());
        }
        if let Some(channel_binding) = self.channel_binding {
            cfg.channel_binding(channel_binding.into());
        }
        if let Some(load_balance_hosts) = self.load_balance_hosts {
            cfg.load_balance_hosts(load_balance_hosts.into());
        }
        Ok(cfg)
    }

    /// Returns [`ManagerConfig`] which can be used to construct a
    /// [`deadpool::managed::Pool`] instance.
    #[must_use]
    pub fn get_manager_config(&self) -> ManagerConfig {
        self.manager.clone().unwrap_or_default()
    }

    /// Returns [`deadpool::managed::PoolConfig`] which can be used to construct
    /// a [`deadpool::managed::Pool`] instance.
    #[must_use]
    pub fn get_pool_config(&self) -> PoolConfig {
        self.pool.unwrap_or_default()
    }
}

/// Possible methods of how a connection is recycled.
///
/// The default is [`Fast`], which performs no health or cleanup query.
/// Native Turso recycling also requires completed work and an idle
/// transaction state, regardless of the selected method.
///
/// [`Fast`]: RecyclingMethod::Fast
/// [`Verified`]: RecyclingMethod::Verified
// [spec:pgorm:sem:conn.pool.recycle+2]
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum RecyclingMethod {
    /// Check [`Client::is_closed()`][1] without executing a query.
    ///
    /// Native Turso also rejects unfinished work and open transactions.
    ///
    /// [1]: pgorm_driver::Client::is_closed
    #[default]
    Fast,

    /// Run [`Client::is_closed()`][1] and execute a test query.
    ///
    /// This is slower, but guarantees that the database connection is ready to
    /// be used. Normally, [`Client::is_closed()`][1] should be enough to filter
    /// out bad connections, but under some circumstances (i.e. hard-closed
    /// network connections) it's possible that [`Client::is_closed()`][1]
    /// returns `false` while the connection is dead. You will receive an error
    /// on your first query then.
    ///
    /// Native Turso uses its native health operation and checks that cleanup
    /// is complete and the transaction state is idle before and after it.
    ///
    /// [1]: pgorm_driver::Client::is_closed
    Verified,

    /// Like [`Verified`] query method, but instead use the following sequence
    /// of statements which guarantees a pristine connection:
    /// ```sql
    /// CLOSE ALL;
    /// SET SESSION AUTHORIZATION DEFAULT;
    /// RESET ALL;
    /// UNLISTEN *;
    /// SELECT pg_advisory_unlock_all();
    /// DISCARD TEMP;
    /// DISCARD SEQUENCES;
    /// ```
    ///
    /// This is similar to calling `DISCARD ALL`. but doesn't call
    /// `DEALLOCATE ALL` and `DISCARD PLAN`, so that the statement cache is not
    /// rendered ineffective.
    ///
    /// Native Turso refuses this method because these PostgreSQL session
    /// operations are unsupported.
    ///
    /// [`Verified`]: RecyclingMethod::Verified
    Clean,

    /// Like [`Verified`] but allows to specify a custom SQL to be executed.
    ///
    /// Native Turso executes the SQL as a batch and rejects the connection
    /// if it leaves unfinished work or an open transaction.
    ///
    /// [`Verified`]: RecyclingMethod::Verified
    Custom(String),
}

impl RecyclingMethod {
    const DISCARD_SQL: &'static str = "\
        CLOSE ALL; \
        SET SESSION AUTHORIZATION DEFAULT; \
        RESET ALL; \
        UNLISTEN *; \
        SELECT pg_advisory_unlock_all(); \
        DISCARD TEMP; \
        DISCARD SEQUENCES;\
    ";

    /// SQL for PostgreSQL connection recycling.
    pub fn query(&self) -> Option<&str> {
        match self {
            Self::Fast => None,
            Self::Verified => Some(""),
            Self::Clean => Some(Self::DISCARD_SQL),
            Self::Custom(sql) => Some(sql),
        }
    }
}

/// How many prepared statements one connection's
/// [`StatementCache`](super::StatementCache) may hold.
///
/// The key space is the SQL text, and one logical query can produce many texts:
/// an `IN` list rendered with a placeholder per element is a different text at
/// every arity. A cache that grew with it would hold server-side prepared
/// statements the connection never uses again, so the size is capped and
/// insertion into a full cache evicts to make room.
///
/// There is deliberately no unbounded variant — that is the growth this type
/// exists to stop, and a caller who wants an effectively unlimited cache can
/// say so with a large [`Bounded`](StatementCacheSize::Bounded). A zero bound
/// is unrepresentable for the same reason: it would be
/// [`Disabled`](StatementCacheSize::Disabled) spelled a second way.
// [spec:pgorm:req:conn.pool.statement-cache.bound+2]
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum StatementCacheSize {
    /// Hold at most this many statements per connection.
    Bounded(NonZeroUsize),

    /// Cache nothing: every statement is prepared afresh and closed when the
    /// last handle to it drops, as if no cache existed.
    Disabled,
}

/// The default bound, an order of magnitude above the worst measured spread of
/// one logical query (25 texts, from 25 `IN`-list arities), and small enough
/// that even a large pool keeps its server-side statement count in the low
/// thousands.
const DEFAULT_STATEMENT_CACHE_SIZE: NonZeroUsize = match NonZeroUsize::new(256) {
    Some(size) => size,
    None => NonZeroUsize::MIN,
};

// [spec:pgorm:req:conn.pool.statement-cache.bound+2]    the default bound
impl Default for StatementCacheSize {
    fn default() -> Self {
        Self::Bounded(DEFAULT_STATEMENT_CACHE_SIZE)
    }
}

impl StatementCacheSize {
    /// The cap, or `None` when nothing is to be cached.
    pub(crate) fn limit(self) -> Option<NonZeroUsize> {
        match self {
            Self::Bounded(limit) => Some(limit),
            Self::Disabled => None,
        }
    }
}

/// Configuration object for a [`Manager`].
///
/// [`Manager`]: super::Manager
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ManagerConfig {
    /// Method of how a connection is recycled. See [`RecyclingMethod`].
    pub recycling_method: RecyclingMethod,

    /// Tag
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,

    /// How many prepared statements each connection caches, and whether it
    /// caches at all. See [`StatementCacheSize`].
    #[serde(default)]
    pub statement_cache: StatementCacheSize,

    /// A channel whose notifications clear every connection's statement cache,
    /// as [`StatementCaches::clear_before_reuse`] does; `None`, the default,
    /// listens on nothing.
    ///
    /// When set, the pool keeps one connection of its own listening on the
    /// channel, opened when the pool opens its first connection. It is not
    /// lent, counts against no size, and is reopened if it drops, the caches
    /// being cleared once each time its `LISTEN` takes effect, since a
    /// notification sent while it was down reached nothing. It sets its own
    /// session's `idle_session_timeout` to 0, so a server that ends idle
    /// sessions does not end it. A notification's payload is ignored.
    /// [`STATEMENT_CACHE_CHANNEL`] is the channel
    /// pgorm-migration's runner notifies once a batch that applied a migration
    /// commits.
    ///
    /// Nothing travels on the listening connection between notifications, so
    /// whatever ends idle connections between the pool and the server ends it
    /// too, and each reopening clears the caches again. A network middlebox
    /// that drops idle TCP connections, such as an AWS Network Load Balancer
    /// (after 350 seconds by default), is kept from it by TCP keepalives sent
    /// more often than its cutoff: set
    /// [`keepalives_idle`](pgorm_driver::Config::keepalives_idle) below it
    /// on the `pgorm_driver::Config` the pool is built from (its default is
    /// two hours), which reaches this connection as it reaches the pool's own. A
    /// keepalive carries no data, so it does not reset a timeout that waits
    /// for the client to send something, as a pooler's can.
    ///
    /// [`StatementCaches::clear_before_reuse`]: super::StatementCaches::clear_before_reuse
    /// [`STATEMENT_CACHE_CHANNEL`]: super::STATEMENT_CACHE_CHANNEL
    // [spec:pgorm:req:conn.pool.statement-cache.listen+1]    the setting, off by default
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement_cache_channel: Option<String>,
}

static DEFAULT_TAG_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[repr(transparent)]
pub(crate) struct Tag(pub Arc<String>);

// [spec:pgorm:sem:conn.pool.get+1]    generated default tag
impl Default for Tag {
    fn default() -> Self {
        Self(Arc::new(format!(
            "default-{}",
            DEFAULT_TAG_COUNT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Properties required of a session.
///
/// This is a 1:1 copy of the [`PgTargetSessionAttrs`] enumeration.
/// This is duplicated here in order to add support for the
/// [`serde::Deserialize`] trait which is required for the [`serde`] support.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum TargetSessionAttrs {
    /// No special properties are required.
    Any,

    /// The session must allow writes.
    ReadWrite,
}

// [spec:pgorm:req:conn.pool.config-forwarding+2]    the enum the field is forwarded as
impl From<TargetSessionAttrs> for PgTargetSessionAttrs {
    fn from(attrs: TargetSessionAttrs) -> Self {
        match attrs {
            TargetSessionAttrs::Any => Self::Any,
            TargetSessionAttrs::ReadWrite => Self::ReadWrite,
        }
    }
}

/// TLS configuration.
///
/// This is a 1:1 copy of the [`PgSslMode`] enumeration.
/// This is duplicated here in order to add support for the
/// [`serde::Deserialize`] trait which is required for the [`serde`] support.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum SslMode {
    /// Do not use TLS.
    Disable,

    /// Attempt to connect with TLS but allow sessions without.
    Prefer,

    /// Require the use of TLS.
    Require,
}

impl From<SslMode> for PgSslMode {
    fn from(mode: SslMode) -> Self {
        match mode {
            SslMode::Disable => Self::Disable,
            SslMode::Prefer => Self::Prefer,
            SslMode::Require => Self::Require,
        }
    }
}

/// Channel binding configuration.
///
/// This is a 1:1 copy of the [`PgChannelBinding`] enumeration.
/// This is duplicated here in order to add support for the
/// [`serde::Deserialize`] trait which is required for the [`serde`] support.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum ChannelBinding {
    /// Do not use channel binding.
    Disable,

    /// Attempt to use channel binding but allow sessions without.
    Prefer,

    /// Require the use of channel binding.
    Require,
}

// [spec:pgorm:req:conn.pool.config-forwarding+2]    the enum the field is forwarded as
impl From<ChannelBinding> for PgChannelBinding {
    fn from(cb: ChannelBinding) -> Self {
        match cb {
            ChannelBinding::Disable => Self::Disable,
            ChannelBinding::Prefer => Self::Prefer,
            ChannelBinding::Require => Self::Require,
        }
    }
}

/// Load balancing configuration.
///
/// This is a 1:1 copy of the [`PgLoadBalanceHosts`] enumeration.
/// This is duplicated here in order to add support for the
/// [`serde::Deserialize`] trait which is required for the [`serde`] support.
#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub enum LoadBalanceHosts {
    /// Make connection attempts to hosts in the order provided.
    Disable,
    /// Make connection attempts to hosts in a random order.
    Random,
}

// [spec:pgorm:req:conn.pool.config-forwarding+2]    the enum the field is forwarded as
impl From<LoadBalanceHosts> for PgLoadBalanceHosts {
    fn from(cb: LoadBalanceHosts) -> Self {
        match cb {
            LoadBalanceHosts::Disable => Self::Disable,
            LoadBalanceHosts::Random => Self::Random,
        }
    }
}
