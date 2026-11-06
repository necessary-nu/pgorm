//! A pool's configuration, read from the connection string and the options
//! JavaScript passes into pgorm-bindings' settings, which decide them the
//! same way for both bindings.

use std::{collections::BTreeMap, time::Duration};

use neon::prelude::*;
use pgorm_bindings::{Dialect, HookEvent, PoolSettings, PoolShape, TlsMode, TlsSettings};

use super::hooks::{self, Callable};
use crate::{
    errors::{Failure, Redactions},
    statements::pick,
    values::read::{refuse, string, text},
};

pub(crate) const DIALECT: Dialect = Dialect::JavaScript;

/// What the options every pool built from one connection string shares say.
#[derive(Debug)]
pub(crate) struct Shared {
    dsn: String,
    tls: TlsSettings,
    connect_timeout: Duration,
    recycling_method: pgorm::RecyclingMethod,
    statement_cache_size: usize,
    statement_cache_channel: Option<String>,
}

/// What one pool's own options say: Rust's `PoolBuilder` closure's, and how
/// long acquiring waits.
#[derive(Debug)]
pub(crate) struct Shape {
    max_size: usize,
    acquire_timeout: Duration,
    wait_timeout: Option<Duration>,
    create_timeout: Option<Duration>,
    recycle_timeout: Option<Duration>,
    queue_mode: pgorm::QueueMode,
    post_create: Option<Callable>,
    pre_recycle: Option<Callable>,
    post_recycle: Option<Callable>,
}

/// A built pool, the secrets its errors redact, and how long acquiring a
/// connection may wait.
pub(crate) struct Built {
    pub(crate) pool: pgorm::DatabasePool,
    pub(crate) secrets: Redactions,
    pub(crate) acquire_timeout: Duration,
}

pub(super) fn option<'cx>(
    cx: &mut Cx<'cx>,
    options: Handle<'cx, JsObject>,
    name: &str,
) -> NeonResult<Option<Handle<'cx, JsValue>>> {
    let value: Handle<JsValue> = options.get(cx, name)?;
    Ok(if value.is_a::<JsUndefined, _>(cx) {
        None
    } else {
        Some(value)
    })
}

/// A count option: a non-negative safe integer.
fn count<'cx>(
    cx: &mut Cx<'cx>,
    options: Handle<'cx, JsObject>,
    name: &str,
    default: usize,
) -> NeonResult<usize> {
    let Some(value) = option(cx, options, name)? else {
        return Ok(default);
    };
    let number = match value.downcast::<JsNumber, _>(cx) {
        Ok(number) => number.value(cx),
        Err(_) => return cx.throw_type_error(format!("{name} is a number")),
    };
    if number.fract() != 0.0 || !(0.0..=9_007_199_254_740_991.0).contains(&number) {
        return refuse(cx, format!("{name} is a non-negative integer"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(number as usize)
}

/// A duration option, in milliseconds: positive and finite.
fn milliseconds<'cx>(
    cx: &mut Cx<'cx>,
    options: Handle<'cx, JsObject>,
    name: &str,
) -> NeonResult<Option<Duration>> {
    let Some(value) = option(cx, options, name)? else {
        return Ok(None);
    };
    let number = match value.downcast::<JsNumber, _>(cx) {
        Ok(number) => number.value(cx),
        Err(_) => return cx.throw_type_error(format!("{name} is a number of milliseconds")),
    };
    match Duration::try_from_secs_f64(number / 1000.0) {
        Ok(duration) if number > 0.0 => Ok(Some(duration)),
        _ => refuse(
            cx,
            format!("{name} is a positive, finite number of milliseconds"),
        ),
    }
}

/// PEM text, as a string or a `Uint8Array`.
fn pem<'cx>(
    cx: &mut Cx<'cx>,
    options: Handle<'cx, JsObject>,
    name: &str,
) -> NeonResult<Option<Vec<u8>>> {
    match option(cx, options, name)? {
        None => Ok(None),
        Some(value) if value.is_a::<JsString, _>(cx) => Ok(Some(string(cx, value)?.into_bytes())),
        Some(value) => match value.downcast::<JsUint8Array, _>(cx) {
            Ok(bytes) => {
                use neon::types::buffer::TypedArray;
                Ok(Some(bytes.as_slice(cx).to_vec()))
            }
            Err(_) => {
                cx.throw_type_error(format!("{name} is PEM text, as a string or a Uint8Array"))
            }
        },
    }
}

/// A hook option: a function, which `lib/connections.js` has wrapped.
fn callable<'cx>(
    cx: &mut Cx<'cx>,
    options: Handle<'cx, JsObject>,
    name: &str,
) -> NeonResult<Option<Callable>> {
    match option(cx, options, name)? {
        None => Ok(None),
        Some(value) => match value.downcast::<JsFunction, _>(cx) {
            Ok(function) => Ok(Some(Callable::new(cx, function))),
            Err(_) => cx.throw_type_error(format!("{name} is a function")),
        },
    }
}

impl Shared {
    /// `dsn` and the shared options, as JavaScript passes them.
    // [spec:pgorm:req:bindings.connections+3]
    // [spec:pgorm:req:bindings.tls]
    // [spec:pgorm:req:napi.lifecycle+3]
    pub(crate) fn read<'cx>(
        cx: &mut Cx<'cx>,
        dsn: Handle<'cx, JsValue>,
        options: Handle<'cx, JsObject>,
    ) -> NeonResult<Self> {
        let dsn = text(cx, dsn, "a connection string")?;
        let tls = match option(cx, options, "tls")? {
            Some(tls) => Some(pick(cx, tls, "tls", &TlsMode::NAMES)?),
            None => None,
        };
        let tls = TlsSettings {
            mode: tls,
            ca: pem(cx, options, "ca")?,
            cert: pem(cx, options, "cert")?,
            key: pem(cx, options, "key")?,
        };
        let recycle = match option(cx, options, "recycle")? {
            Some(recycle) => Some(pick(
                cx,
                recycle,
                "recycle",
                &[
                    ("verified", pgorm::RecyclingMethod::Verified),
                    ("fast", pgorm::RecyclingMethod::Fast),
                    ("clean", pgorm::RecyclingMethod::Clean),
                ],
            )?),
            None => None,
        };
        let recycle_sql = match option(cx, options, "recycleSql")? {
            Some(sql) => Some(text(cx, sql, "recycleSql")?),
            None => None,
        };
        let recycling_method = match (recycle, recycle_sql) {
            (Some(_), Some(_)) => {
                return cx
                    .throw_type_error("recycle and recycleSql both say how to recycle: give one");
            }
            (_, Some(sql)) => pgorm::RecyclingMethod::Custom(sql),
            (Some(method), None) => method,
            (None, None) => pgorm::RecyclingMethod::Verified,
        };
        let statement_cache_channel = match option(cx, options, "statementCacheChannel")? {
            Some(channel) => Some(text(cx, channel, "statementCacheChannel")?),
            None => None,
        };
        Ok(Self {
            dsn,
            tls,
            connect_timeout: milliseconds(cx, options, "connectTimeout")?
                .unwrap_or(Duration::from_secs(10)),
            recycling_method,
            statement_cache_size: count(cx, options, "statementCacheSize", 128)?,
            statement_cache_channel,
        })
    }

    fn settings(self, tag: Option<String>) -> PoolSettings {
        PoolSettings {
            dsn: self.dsn,
            tls: self.tls,
            connect_timeout: self.connect_timeout,
            recycling_method: self.recycling_method,
            statement_cache_size: self.statement_cache_size,
            statement_cache_channel: self.statement_cache_channel,
            tag,
        }
    }

    /// A pool shaped as `shape` says, tagged `tag`. TLS verifies the server's
    /// certificate and host name unless the connection string says
    /// `sslmode=disable` or the `tls` option names a weaker mode; there is no
    /// silent fallback to plaintext.
    // [spec:pgorm:req:bindings.connections+3]
    pub(crate) fn build(self, tag: Option<String>, shape: Shape) -> Result<Built, Failure> {
        let mut secrets = None;
        let acquire_timeout = shape.acquire_timeout;
        let pool = self
            .settings(tag)
            .build(
                |config| {
                    let redactions = Redactions::from_config(config);
                    let shaped = shape.shape(&redactions);
                    secrets = Some(redactions);
                    shaped
                },
                DIALECT,
            )
            .map_err(|refusal| Failure::Construction(refusal.into_message()))?;
        Ok(Built {
            pool,
            secrets: secrets
                .ok_or_else(|| Failure::Internal("the pool was built unshaped".to_owned()))?,
            acquire_timeout,
        })
    }

    /// One pool per entry of `shapes`, each tagged with its key, in the
    /// tags' order.
    // [spec:pgorm:req:bindings.multi-pool]
    pub(crate) fn build_multi(
        self,
        shapes: BTreeMap<String, Shape>,
    ) -> Result<Vec<(String, Built)>, Failure> {
        let mut secrets = None;
        let timeouts: BTreeMap<String, Duration> = shapes
            .iter()
            .map(|(tag, shape)| (tag.clone(), shape.acquire_timeout))
            .collect();
        let pools = pgorm_bindings::multi_pool(
            self.settings(None),
            |config| {
                let redactions = Redactions::from_config(config);
                let shaped = shapes
                    .iter()
                    .map(|(tag, shape)| (tag.clone(), shape.shape(&redactions)))
                    .collect();
                secrets = Some(redactions);
                shaped
            },
            DIALECT,
        )
        .map_err(|refusal| Failure::Construction(refusal.into_message()))?;
        let secrets =
            secrets.ok_or_else(|| Failure::Internal("the pools were built unshaped".to_owned()))?;
        Ok(pools
            .iter()
            .map(|(tag, pool)| {
                (
                    tag.to_owned(),
                    Built {
                        pool: pool.clone(),
                        secrets: secrets.clone(),
                        acquire_timeout: timeouts
                            .get(tag)
                            .copied()
                            .unwrap_or(Duration::from_secs(30)),
                    },
                )
            })
            .collect())
    }

    /// The connection string and TLS options, for a listener of their own.
    pub(crate) fn listener_parts(&self) -> (&str, &TlsSettings, Duration) {
        (&self.dsn, &self.tls, self.connect_timeout)
    }
}

impl Shape {
    /// One pool's own options, as JavaScript passes them.
    // [spec:pgorm:req:bindings.connections+3]
    // [spec:pgorm:req:bindings.pool-hooks]
    pub(crate) fn read<'cx>(cx: &mut Cx<'cx>, options: Handle<'cx, JsObject>) -> NeonResult<Self> {
        let queue_mode = match option(cx, options, "queueMode")? {
            Some(mode) => pick(
                cx,
                mode,
                "queueMode",
                &[
                    ("fifo", pgorm::QueueMode::Fifo),
                    ("lifo", pgorm::QueueMode::Lifo),
                ],
            )?,
            None => pgorm::QueueMode::Fifo,
        };
        Ok(Self {
            max_size: count(cx, options, "maxSize", 10)?,
            acquire_timeout: milliseconds(cx, options, "acquireTimeout")?
                .unwrap_or(Duration::from_secs(30)),
            wait_timeout: milliseconds(cx, options, "waitTimeout")?,
            create_timeout: milliseconds(cx, options, "createTimeout")?,
            recycle_timeout: milliseconds(cx, options, "recycleTimeout")?,
            queue_mode,
            post_create: callable(cx, options, "postCreate")?,
            pre_recycle: callable(cx, options, "preRecycle")?,
            post_recycle: callable(cx, options, "postRecycle")?,
        })
    }

    fn shape(&self, secrets: &Redactions) -> PoolShape {
        let hook = |event, callable: &Option<Callable>| {
            callable
                .clone()
                .map(|callable| hooks::hook(event, callable, secrets.clone()))
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
