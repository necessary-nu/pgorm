//! A binding's pool options, built into pgorm pools.

use std::{collections::BTreeMap, num::NonZeroUsize, time::Duration};

use pgorm::{
    Config, DatabaseMultiPool, DatabasePool, Hook, HookError, Listener, ManagerConfig, NoTls,
    Notifications, PoolBuilder, PoolError, QueueMode, RecyclingMethod, Runtime, StatementCacheSize,
    TimeoutType,
};

use crate::{Connector, Dialect, Failure, HookEvent, Redactions, Refusal, TlsSettings};

/// One pool's own options as a binding read them: Rust's `PoolBuilder`
/// closure's, how long acquiring a connection waits, and the language's
/// callables `C` its three hooks call.
// [spec:pgorm:req:bindings.connections+4]
// [spec:pgorm:req:bindings.pool-hooks]
#[derive(Debug, Clone)]
pub struct PoolOptions<C> {
    /// The most connections the pool holds.
    pub max_size: usize,
    /// How long the binding's acquire waits for a connection in all, which
    /// [`PoolState`](crate::PoolState) times itself.
    pub acquire_timeout: Duration,
    /// How long a `get` waits for a free connection.
    pub wait_timeout: Option<Duration>,
    /// How long opening a connection may take.
    pub create_timeout: Option<Duration>,
    /// How long checking a returned connection may take.
    pub recycle_timeout: Option<Duration>,
    /// Whether the pool lends the connection returned first or last.
    pub queue_mode: QueueMode,
    /// The callable run on each connection as it opens.
    pub post_create: Option<C>,
    /// The callable run on a returned connection before it is checked.
    pub pre_recycle: Option<C>,
    /// The callable run on a returned connection once it passed its check.
    pub post_recycle: Option<C>,
}

impl<C> PoolOptions<C> {
    /// Validate pool capacity without constructing or invoking hooks.
    fn check(&self, dialect: Dialect) -> Result<(), Refusal> {
        if self.max_size == 0 || self.max_size > tokio::sync::Semaphore::MAX_PERMITS {
            return Err(Refusal(format!(
                "{} is outside the supported range",
                dialect.option("max_size")
            )));
        }
        Ok(())
    }

    /// The shape the pool is built with, each hook the one `hook` makes of
    /// its callable, reporting under `secrets`.
    fn shape(
        &self,
        hook: &impl Fn(HookEvent, &C, &Redactions) -> Hook,
        secrets: &Redactions,
    ) -> PoolShape {
        let hook = |event, callable: &Option<C>| {
            callable
                .as_ref()
                .map(|callable| hook(event, callable, secrets))
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

/// A pool built from a binding's options, the secrets its errors redact, and
/// how long acquiring one of its connections waits.
#[derive(Debug)]
pub struct Built {
    /// The pool.
    pub pool: DatabasePool,
    /// What the pool's errors redact: its configuration's password.
    pub secrets: Redactions,
    /// [`PoolOptions::acquire_timeout`], as the options gave it.
    pub acquire_timeout: Duration,
    /// Whether the pool opens a native database rather than PostgreSQL.
    pub native: bool,
}

/// What Rust's `PoolBuilder` closure says of one pool: its size, its three
/// timeouts, the order it lends idle connections in, and its hooks.
#[derive(Debug)]
pub(crate) struct PoolShape {
    /// `max_size`: the most connections the pool holds.
    pub max_size: usize,
    /// `wait_timeout`: how long a `get` waits for a free connection.
    pub wait_timeout: Option<Duration>,
    /// `create_timeout`: how long opening a connection may take.
    pub create_timeout: Option<Duration>,
    /// `recycle_timeout`: how long checking a returned connection may take.
    pub recycle_timeout: Option<Duration>,
    /// `queue_mode`: lend the connection returned first, or last.
    pub queue_mode: QueueMode,
    /// `post_create`, run on each connection as it opens.
    pub post_create: Option<Hook>,
    /// `pre_recycle`, run on a returned connection before it is checked.
    pub pre_recycle: Option<Hook>,
    /// `post_recycle`, run on a returned connection once it passed its
    /// check, before the pool lends it again.
    pub post_recycle: Option<Hook>,
}

impl PoolShape {
    /// `max_size` connections, no timeouts, first in first out, no hooks.
    #[cfg(test)]
    fn sized(max_size: usize) -> Self {
        Self {
            max_size,
            wait_timeout: None,
            create_timeout: None,
            recycle_timeout: None,
            queue_mode: QueueMode::Fifo,
            post_create: None,
            pre_recycle: None,
            post_recycle: None,
        }
    }

    fn check(&self, dialect: Dialect) -> Result<(), Refusal> {
        if self.max_size == 0 || self.max_size > tokio::sync::Semaphore::MAX_PERMITS {
            return Err(Refusal(format!(
                "{} is outside the supported range",
                dialect.option("max_size")
            )));
        }
        Ok(())
    }

    /// The builder shaped as this says. The pool always times on tokio, the
    /// runtime the binding runs pgorm on, so a timeout never leaves a pool
    /// unbuildable.
    fn apply(self, builder: PoolBuilder) -> PoolBuilder {
        let mut builder = builder
            .max_size(self.max_size)
            .runtime(Runtime::Tokio1)
            .wait_timeout(self.wait_timeout)
            .create_timeout(self.create_timeout)
            .recycle_timeout(self.recycle_timeout)
            .queue_mode(self.queue_mode);
        if let Some(hook) = self.post_create {
            builder = builder.post_create(hook);
        }
        if let Some(hook) = self.pre_recycle {
            builder = builder.pre_recycle(hook);
        }
        if let Some(hook) = self.post_recycle {
            builder = builder.post_recycle(hook);
        }
        builder
    }
}

/// What a binding's pool is built from: Rust's `connect_with` arguments, the
/// connection string in place of the parsed `Config` and TLS options in
/// place of a connector.
#[derive(Debug)]
pub struct PoolSettings {
    /// A PostgreSQL URL, libpq `key=value` string, or native database path.
    pub dsn: String,
    /// The TLS options.
    pub tls: TlsSettings,
    /// PostgreSQL connection timeout, replacing the string's setting and
    /// defaulting to ten seconds. Native paths refuse an explicit timeout.
    pub connect_timeout: Option<Duration>,
    /// How a returned connection is checked before it is lent again.
    pub recycling_method: RecyclingMethod,
    /// How many statements each connection keeps prepared, 0 for none.
    pub statement_cache_size: usize,
    /// A channel whose notifications clear the statement caches.
    pub statement_cache_channel: Option<String>,
    /// The pool's tag; absent, pgorm names it `default-N`.
    pub tag: Option<String>,
}

impl PoolSettings {
    /// The parsed configuration, the connector it connects through and the
    /// manager configuration, every refusal decided before anything is
    /// built.
    // [spec:pgorm:req:bindings.connections+4]
    // [spec:pgorm:req:turso.python+1]
    // [spec:pgorm:req:turso.javascript+1]
    fn prepare(self, dialect: Dialect) -> Result<(Config, Connector, ManagerConfig), Failure> {
        let mut config = parse(&self.dsn, self.connect_timeout)?;
        let manager = manager_config(
            self.recycling_method,
            self.statement_cache_size,
            self.statement_cache_channel,
            self.tag,
            dialect,
        )?;
        let connector = if config.is_turso() {
            if self.tls.mode.is_some()
                || self.tls.ca.is_some()
                || self.tls.cert.is_some()
                || self.tls.key.is_some()
            {
                return Err(Failure::Construction(
                    "native database paths do not accept TLS options".into(),
                ));
            }
            if self.connect_timeout.is_some() {
                return Err(Failure::Construction(format!(
                    "native database paths do not accept {}",
                    dialect.option("connect_timeout")
                )));
            }
            if matches!(manager.recycling_method, RecyclingMethod::Clean)
                || manager.statement_cache_channel.is_some()
            {
                return Err(Failure::Construction("native database paths do not support Clean recycling or notification-based statement cache invalidation".into()));
            }
            #[cfg(not(feature = "turso"))]
            return Err(Failure::UnsupportedCapability(
                "this pgorm build does not include the experimental turso feature".into(),
            ));
            #[cfg(feature = "turso")]
            {
                Connector::Plain
            }
        } else {
            self.tls.connector(&mut config, dialect)?
        };
        Ok((config, connector, manager))
    }

    /// The pool `connect_with` builds, shaped as `options` say, each hook
    /// the one `hook` makes of its callable once the parsed configuration
    /// has said what its errors redact.
    // [spec:pgorm:req:bindings.connections+4]
    pub fn pool<C>(
        self,
        options: &PoolOptions<C>,
        hook: impl Fn(HookEvent, &C, &Redactions) -> Hook,
        dialect: Dialect,
    ) -> Result<Built, Failure> {
        options.check(dialect)?;
        let mut secrets = None;
        let mut native = false;
        let pool = self.build(
            |config| {
                native = config.is_turso();
                let redactions = Redactions::from_config(config);
                let shape = options.shape(&hook, &redactions);
                secrets = Some(redactions);
                shape
            },
            dialect,
        )?;
        Ok(Built {
            pool,
            secrets: secrets.ok_or_else(unshaped)?,
            acquire_timeout: options.acquire_timeout,
            native,
        })
    }

    /// One pool per tag, each shaped as its entry of `options` says, sharing
    /// these settings, in the tags' order, through pgorm's `connect_multi_with`.
    // [spec:pgorm:req:bindings.multi-pool]
    pub fn pools<C>(
        self,
        options: &BTreeMap<String, PoolOptions<C>>,
        hook: impl Fn(HookEvent, &C, &Redactions) -> Hook,
        dialect: Dialect,
    ) -> Result<Vec<(String, Built)>, Failure> {
        if options.is_empty() {
            return Err(Failure::Construction(
                "a multi-pool needs at least one pool".into(),
            ));
        }
        for (tag, options) in options {
            if tag.is_empty() {
                return Err(Failure::Construction(format!(
                    "{} must not be empty",
                    dialect.option("tag")
                )));
            }
            options.check(dialect)?;
        }
        let mut secrets = None;
        let mut native = false;
        let pools = multi_pool(
            self,
            |config| {
                native = config.is_turso();
                let redactions = Redactions::from_config(config);
                let shapes = options
                    .iter()
                    .map(|(tag, options)| (tag.clone(), options.shape(&hook, &redactions)))
                    .collect();
                secrets = Some(redactions);
                shapes
            },
            dialect,
        )?;
        let secrets = secrets.ok_or_else(unshaped)?;
        pools
            .iter()
            .map(|(tag, pool)| {
                let options = options
                    .get(tag)
                    .ok_or_else(|| Failure::Internal("a pool has no shape".to_owned()))?;
                Ok((
                    tag.to_owned(),
                    Built {
                        pool: pool.clone(),
                        secrets: secrets.clone(),
                        acquire_timeout: options.acquire_timeout,
                        native,
                    },
                ))
            })
            .collect()
    }

    /// The pool `connect_with` builds, shaped as `shape` says once it has
    /// seen the parsed configuration, from which a binding learns what its
    /// errors redact before it makes the hooks that report them.
    fn build(
        self,
        shape: impl FnOnce(&Config) -> PoolShape,
        dialect: Dialect,
    ) -> Result<DatabasePool, Failure> {
        let (config, connector, manager) = self.prepare(dialect)?;
        let shape = shape(&config);
        shape.check(dialect)?;
        let build = |builder| shape.apply(builder);
        match connector {
            Connector::Plain => pgorm::connect_with(config, NoTls, manager, build),
            Connector::Rustls(tls) => pgorm::connect_with(config, tls, manager, build),
        }
        .map_err(|_| Failure::Construction("invalid database pool configuration".to_owned()))
    }
}

/// Shared option errors take precedence over backend availability failures.
// [spec:pgorm:req:conn.pool.statement-cache.listen+1]
fn manager_config(
    recycling_method: RecyclingMethod,
    statement_cache_size: usize,
    statement_cache_channel: Option<String>,
    tag: Option<String>,
    dialect: Dialect,
) -> Result<ManagerConfig, Refusal> {
    if statement_cache_channel
        .as_ref()
        .is_some_and(|channel| channel.is_empty() || channel.len() > 63)
    {
        return Err(Refusal(format!(
            "{} must be a channel name of 1 to 63 bytes",
            dialect.option("statement_cache_channel")
        )));
    }
    if tag.as_ref().is_some_and(String::is_empty) {
        return Err(Refusal(format!(
            "{} must not be empty",
            dialect.option("tag")
        )));
    }
    Ok(ManagerConfig {
        recycling_method,
        tag,
        statement_cache_channel,
        statement_cache: match NonZeroUsize::new(statement_cache_size) {
            None => StatementCacheSize::Disabled,
            Some(size) => StatementCacheSize::Bounded(size),
        },
    })
}

/// What a pool built without its shape being asked for is, which no input
/// should cause.
fn unshaped() -> Failure {
    Failure::Internal("the pool was built unshaped".to_owned())
}

/// Parse a driver target, requiring a host for PostgreSQL and applying its
/// connection timeout. Native targets retain their file configuration.
/// The refusal never repeats the string, which may hold a password.
fn parse(dsn: &str, connect_timeout: Option<Duration>) -> Result<Config, Refusal> {
    let mut config: Config = dsn
        .parse()
        .map_err(|_| Refusal("invalid PostgreSQL connection string".to_owned()))?;
    if !config.is_turso() && config.get_hosts().is_empty() {
        return Err(Refusal("the connection string names no host".to_owned()));
    }
    if !config.is_turso() {
        config.connect_timeout(connect_timeout.unwrap_or(Duration::from_secs(10)));
    }
    Ok(config)
}

/// What a listener of its own connects with: its connection string and TLS
/// options read as a pool's are, every refusal decided before anything is
/// sent. The binding learns what its errors redact from the configuration,
/// then opens the listener with [`connect_listener`].
// [spec:pgorm:req:bindings.listen+3]
pub fn listener_config(
    dsn: &str,
    tls: &TlsSettings,
    connect_timeout: Duration,
    dialect: Dialect,
) -> Result<(Config, Connector), Refusal> {
    let mut config = parse(dsn, Some(connect_timeout))?;
    if config.is_turso() {
        return Err(Refusal(
            "native database paths do not support notification listeners".into(),
        ));
    }
    let connector = tls.connector(&mut config, dialect)?;
    Ok((config, connector))
}

/// A listener on a connection of its own, belonging to no pool, through
/// pgorm's `connect_listener`.
// [spec:pgorm:req:bindings.listen+3]
pub async fn connect_listener(
    config: Config,
    connector: Connector,
    capacity: NonZeroUsize,
) -> Result<(Listener, Notifications), pgorm::Error> {
    match connector {
        Connector::Plain => pgorm::connect_listener(config, NoTls, capacity).await,
        Connector::Rustls(tls) => pgorm::connect_listener(config, tls, capacity).await,
    }
}

/// One pool per tag, each shaped as its entry says, sharing `settings`'
/// connection, TLS and manager configuration, through pgorm's
/// `connect_multi_with`. `settings` names no tag of its own: each pool's is
/// its key.
// [spec:pgorm:req:bindings.multi-pool]
fn multi_pool(
    settings: PoolSettings,
    shapes: impl FnOnce(&Config) -> BTreeMap<String, PoolShape>,
    dialect: Dialect,
) -> Result<DatabaseMultiPool, Failure> {
    if settings.tag.is_some() {
        return Err(Failure::Construction(format!(
            "each pool's {} is its key, so the shared options name none",
            dialect.option("tag")
        )));
    }
    let (config, connector, manager) = settings.prepare(dialect)?;
    let shapes = shapes(&config);
    if shapes.is_empty() {
        return Err(Failure::Construction(
            "a multi-pool needs at least one pool".to_owned(),
        ));
    }
    for (tag, shape) in &shapes {
        if tag.is_empty() {
            return Err(Failure::Construction(format!(
                "{} must not be empty",
                dialect.option("tag")
            )));
        }
        shape.check(dialect)?;
    }
    let builds = shapes
        .into_iter()
        .map(|(tag, shape)| (tag, move |builder| shape.apply(builder)))
        .collect::<BTreeMap<_, _>>();
    match connector {
        Connector::Plain => pgorm::connect_multi_with(config, NoTls, manager, builds),
        Connector::Rustls(tls) => pgorm::connect_multi_with(config, tls, manager, builds),
    }
    .map_err(|_| Failure::Construction("invalid PostgreSQL pool configuration".to_owned()))
}

/// What a failed `get` was, when the pool rather than the server refused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PoolFailure {
    /// One of the three timeouts ran out; raised as the binding's
    /// `TimeoutError`.
    Timeout(String),
    /// The connection could not be lent; raised as its `ConnectionError`.
    Connection(String),
}

/// `error` as a binding reports it, worded for `dialect`. A server's or the
/// driver's refusal is not among these: the binding finds that in the
/// error's causes first, and reports it with its SQLSTATE.
// [spec:pgorm:req:bindings.pool-hooks]
pub(crate) fn pool_failure(error: &PoolError, dialect: Dialect) -> PoolFailure {
    match error {
        PoolError::Timeout(TimeoutType::Wait) => PoolFailure::Timeout(format!(
            "no connection came free within the pool's {}",
            dialect.option("wait_timeout")
        )),
        PoolError::Timeout(TimeoutType::Create) => PoolFailure::Timeout(format!(
            "opening a connection outlasted the pool's {}",
            dialect.option("create_timeout")
        )),
        PoolError::PostCreateHook(HookError::Message(message)) => PoolFailure::Connection(format!(
            "the {} hook refused the connection: {message}",
            dialect.option("post_create")
        )),
        PoolError::Closed => PoolFailure::Connection("the pool is closed".to_owned()),
        _ => PoolFailure::Connection("PostgreSQL pool acquisition failed".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(dsn: &str) -> PoolSettings {
        PoolSettings {
            dsn: dsn.to_owned(),
            tls: TlsSettings::default(),
            connect_timeout: Some(Duration::from_secs(10)),
            recycling_method: RecyclingMethod::Fast,
            statement_cache_size: 0,
            statement_cache_channel: None,
            tag: None,
        }
    }

    fn refusal<T, E: std::fmt::Display>(built: Result<T, E>) -> String {
        match built {
            Err(refusal) => refusal.to_string(),
            Ok(_) => panic!("the pool was built"),
        }
    }

    #[tokio::test]
    async fn a_pool_is_refused_what_it_cannot_use() {
        let plain = "postgres://postgres@db.example.com/bakery?sslmode=disable";
        assert_eq!(
            refusal(settings(plain).build(|_| PoolShape::sized(0), Dialect::JavaScript)),
            "maxSize is outside the supported range"
        );
        assert_eq!(
            refusal(settings("host='unterminated").build(|_| PoolShape::sized(1), Dialect::Python)),
            "invalid PostgreSQL connection string"
        );
        assert_eq!(
            refusal(settings("dbname=bakery").build(|_| PoolShape::sized(1), Dialect::Python)),
            "the connection string names no host"
        );
        let named = PoolSettings {
            tag: Some(String::new()),
            ..settings(plain)
        };
        assert_eq!(
            refusal(named.build(|_| PoolShape::sized(1), Dialect::Python)),
            "tag must not be empty"
        );
    }

    #[tokio::test]
    async fn a_pool_is_tagged_and_shaped_as_asked() {
        let plain = "postgres://postgres@db.example.com/bakery?sslmode=disable";
        let tagged = PoolSettings {
            tag: Some("kitchen".to_owned()),
            ..settings(plain)
        };
        let shape = PoolShape {
            wait_timeout: Some(Duration::from_millis(5)),
            queue_mode: QueueMode::Lifo,
            ..PoolShape::sized(3)
        };
        let pool = tagged
            .build(|_| shape, Dialect::Python)
            .expect("the pool builds");
        assert_eq!(pool.tag().as_str(), "kitchen");
        assert_eq!(pool.status().max_size, 3);
    }

    #[tokio::test]
    async fn each_pool_is_tagged_with_its_key() {
        let plain = "postgres://postgres@db.example.com/bakery?sslmode=disable";
        let shapes = BTreeMap::from([
            ("web".to_owned(), PoolShape::sized(8)),
            ("jobs".to_owned(), PoolShape::sized(2)),
        ]);
        let pools =
            multi_pool(settings(plain), |_| shapes, Dialect::Python).expect("the pools build");
        let tags: Vec<_> = pools
            .iter()
            .map(|(tag, pool)| {
                (
                    tag.to_owned(),
                    pool.tag().to_string(),
                    pool.status().max_size,
                )
            })
            .collect();
        assert_eq!(
            tags,
            [
                ("jobs".to_owned(), "jobs".to_owned(), 2),
                ("web".to_owned(), "web".to_owned(), 8)
            ]
        );
        let named = PoolSettings {
            tag: Some("kitchen".to_owned()),
            ..settings(plain)
        };
        assert_eq!(
            refusal(multi_pool(
                named,
                |_| BTreeMap::from([("web".to_owned(), PoolShape::sized(1))]),
                Dialect::JavaScript
            )),
            "each pool's tag is its key, so the shared options name none"
        );
    }

    fn options(max_size: usize, acquire: u64) -> PoolOptions<()> {
        PoolOptions {
            max_size,
            acquire_timeout: Duration::from_secs(acquire),
            wait_timeout: None,
            create_timeout: None,
            recycle_timeout: None,
            queue_mode: QueueMode::Fifo,
            post_create: None,
            pre_recycle: None,
            post_recycle: None,
        }
    }

    fn no_hook(_: HookEvent, (): &(), _: &Redactions) -> Hook {
        unreachable!("no hook is asked for")
    }

    // [spec:pgorm:req:turso.python+1/test]
    // [spec:pgorm:req:turso.javascript+1/test]
    #[tokio::test]
    async fn native_targets_validate_before_building() {
        let native = || PoolSettings {
            connect_timeout: None,
            ..settings(":memory:")
        };
        let invalid = native().pool(&options(0, 1), no_hook, Dialect::Python);
        assert!(matches!(invalid, Err(Failure::Construction(_))));
        let timed = PoolSettings {
            connect_timeout: Some(Duration::from_secs(10)),
            ..native()
        };
        assert!(matches!(
            timed.prepare(Dialect::Python),
            Err(Failure::Construction(_))
        ));
        let clean = PoolSettings {
            recycling_method: RecyclingMethod::Clean,
            ..native()
        };
        assert!(matches!(
            clean.prepare(Dialect::Python),
            Err(Failure::Construction(_))
        ));
        let built = native().pool(&options(2, 1), no_hook, Dialect::Python);
        #[cfg(feature = "turso")]
        {
            let built = built.expect("native pool is lazy");
            assert_eq!(built.pool.status().size, 0);
            assert_eq!(built.pool.status().max_size, 2);
        }
        #[cfg(not(feature = "turso"))]
        assert!(matches!(built, Err(Failure::UnsupportedCapability(_))));
    }

    // [spec:pgorm:req:bindings.connections+4/test]    each pool keeps its own acquire timeout
    #[tokio::test]
    async fn each_pool_keeps_its_acquire_timeout() {
        let plain = "postgres://postgres@db.example.com/bakery?sslmode=disable";
        let built = settings(plain)
            .pool(&options(3, 7), no_hook, Dialect::Python)
            .expect("the pool builds");
        assert_eq!(built.pool.status().max_size, 3);
        assert_eq!(built.acquire_timeout, Duration::from_secs(7));
        let shapes = BTreeMap::from([
            ("web".to_owned(), options(8, 2)),
            ("jobs".to_owned(), options(2, 9)),
        ]);
        let pools = settings(plain)
            .pools(&shapes, no_hook, Dialect::JavaScript)
            .expect("the pools build");
        let timeouts: Vec<_> = pools
            .iter()
            .map(|(tag, built)| (tag.as_str(), built.acquire_timeout.as_secs()))
            .collect();
        assert_eq!(timeouts, [("jobs", 9), ("web", 2)]);
        let refused = settings("dbname=bakery").pool(&options(1, 1), no_hook, Dialect::Python);
        assert_eq!(
            refused
                .map(|built| built.acquire_timeout)
                .map_err(|failure| failure.message().to_owned()),
            Err("the connection string names no host".to_owned())
        );
    }

    #[test]
    fn pool_failures_name_what_ran_out() {
        assert_eq!(
            pool_failure(&PoolError::Timeout(TimeoutType::Wait), Dialect::JavaScript),
            PoolFailure::Timeout(
                "no connection came free within the pool's waitTimeout".to_owned()
            )
        );
        assert_eq!(
            pool_failure(
                &PoolError::PostCreateHook(HookError::message("too old")),
                Dialect::Python
            ),
            PoolFailure::Connection(
                "the post_create hook refused the connection: too old".to_owned()
            )
        );
    }
}
