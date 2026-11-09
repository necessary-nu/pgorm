//! A pool's configuration, read from the connection string and the options
//! JavaScript passes into pgorm-bindings' settings, which decide them the
//! same way for both bindings.

use std::{collections::BTreeMap, time::Duration};

use neon::prelude::*;
use pgorm_bindings::{Built, Dialect, PoolOptions, PoolSettings, TlsMode, TlsSettings};

use super::hooks::{self, Callable};
use crate::{
    errors::Failure,
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

/// What one pool's own options say: Rust's `PoolBuilder` closure's, how
/// long acquiring waits, and the functions its hooks call.
pub(crate) type Shape = PoolOptions<Callable>;

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
        self.settings(tag).pool(
            &shape,
            |event, callable, secrets| hooks::hook(event, callable.clone(), secrets.clone()),
            DIALECT,
        )
    }

    /// One pool per entry of `shapes`, each tagged with its key, in the
    /// tags' order.
    // [spec:pgorm:req:bindings.multi-pool]
    pub(crate) fn build_multi(
        self,
        shapes: BTreeMap<String, Shape>,
    ) -> Result<Vec<(String, Built)>, Failure> {
        self.settings(None).pools(
            &shapes,
            |event, callable, secrets| hooks::hook(event, callable.clone(), secrets.clone()),
            DIALECT,
        )
    }

    /// The connection string and TLS options, for a listener of their own.
    pub(crate) fn listener_parts(&self) -> (&str, &TlsSettings, Duration) {
        (&self.dsn, &self.tls, self.connect_timeout)
    }
}

/// One pool's own options, as JavaScript passes them.
// [spec:pgorm:req:bindings.connections+3]
// [spec:pgorm:req:bindings.pool-hooks]
pub(crate) fn shape<'cx>(cx: &mut Cx<'cx>, options: Handle<'cx, JsObject>) -> NeonResult<Shape> {
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
    Ok(Shape {
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
