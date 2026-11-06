# Connection configuration and TLS

[Getting started](crate::guide::getting_started) built a pool with
[`pgorm::connect`](crate::connect), which takes a connection
[`Config`](crate::Config) and uses defaults for everything else. This chapter
covers the rest: what the `Config` can say, how big the pool is and how long it
waits, how connections are checked when they are reused, TLS, and pools kept
side by side.

There are five constructors. `connect` and `connect_with_builder` are
shorthands for `connect_with`, and `connect_multi_with_builder` for
`connect_multi_with`, which builds each of its pools with `connect_with`:

| Function | Takes |
| --- | --- |
| [`connect`](crate::connect) | a `Config` |
| [`connect_with_builder`](crate::connect_with_builder) | a `Config`, and a closure shaping the pool |
| [`connect_with`](crate::connect_with) | a `Config`, a TLS connector, a [`ManagerConfig`](crate::ManagerConfig), and a closure shaping the pool |
| [`connect_multi_with_builder`](crate::connect_multi_with_builder) | a `Config`, and a closure per pool, by name |
| [`connect_multi_with`](crate::connect_multi_with) | a `Config`, a TLS connector, a `ManagerConfig`, and a closure per pool, by name |

None of them connects. A pool opens connections when they are first asked
for, so a wrong host, password or database name shows up at the first
[`get`](crate::DatabasePool::get), as an error there.

## The connection's settings

[`Config`](crate::Config) is tokio-postgres's connection configuration. It
parses from a URL or from libpq's `key=value` form, and its builder methods
set each part: `host`, `port`, `user`, `password`, `dbname`,
`application_name`, `connect_timeout`, `keepalives_idle`, and `options`,
which passes server settings for every session, such as a `search_path` or
a `statement_timeout`. The setters that choose between modes take
tokio-postgres's enums, which pgorm re-exports: `ssl_mode` an
[`SslMode`](crate::SslMode), `ssl_negotiation` an
[`SslNegotiation`](crate::SslNegotiation), `target_session_attrs` a
[`TargetSessionAttrs`](crate::TargetSessionAttrs), `channel_binding` a
[`ChannelBinding`](crate::ChannelBinding), and `load_balance_hosts` a
[`LoadBalanceHosts`](crate::LoadBalanceHosts).

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("configuration_config").await;
# let url = scratch.url();
use std::time::Duration;

use pgorm::{ConnectionTrait, TargetSessionAttrs};

let mut config: pgorm::Config = url.parse()?;
config
    .application_name("bakery-web")
    .connect_timeout(Duration::from_secs(5))
    // Refuse a server that is a read-only standby.
    .target_session_attrs(TargetSessionAttrs::ReadWrite)
    .options("-c search_path=kitchen,public -c statement_timeout=5s");

let pool = pgorm::connect(config);
let db = pool.get().await?;
let setting = |name: &'static str| {
    let db = &db;
    async move {
        let row = db.query_one("SELECT current_setting($1)", &[&name]).await?;
        Ok::<String, pgorm::Error>(row.get(0))
    }
};
assert_eq!(setting("application_name").await?, "bakery-web");
assert_eq!(setting("search_path").await?, "kitchen,public");
assert_eq!(setting("statement_timeout").await?, "5s");
# drop(db);
# pool.close();
# scratch.finish().await;
# Ok(())
# }
```

An `application_name` shows in `pg_stat_activity` and the server's log, which
makes the program's sessions easy to find. A `search_path` is how a program
whose tables live in a schema other than `public` reaches them without
qualifying every name.

## The pool's size and timeouts

[`connect_with_builder`](crate::connect_with_builder) passes the pool's
[`PoolBuilder`](crate::PoolBuilder), deadpool's, to a closure of yours:

- `max_size(n)` caps how many connections the pool opens. The default is
  twice the number of CPUs the machine reports.
- `wait_timeout`, `create_timeout` and `recycle_timeout` bound how long a
  `get` waits for a free connection, for a new connection to open, and for a
  returned one to be checked. Each is off by default, so a `get` on an
  exhausted pool waits until a connection is returned. A timeout needs the
  runtime it is timed on: `runtime(Runtime::Tokio1)`. `timeouts` sets all
  three at once, from a [`Timeouts`](crate::Timeouts).
- `queue_mode` chooses whether the pool lends the connection returned first
  or last, a [`QueueMode`](crate::QueueMode), and `config` sets the size,
  the timeouts and the queue mode together, from a
  [`PoolConfig`](crate::PoolConfig).
- `post_create`, `pre_recycle` and `post_recycle` each take a
  [`Hook`](crate::Hook), run on a connection when it opens, before the pool
  checks it for reuse, and after.

These types are deadpool's, re-exported by pgorm with every other type that
shaping a pool takes, so the closure needs no dependency but pgorm.

A closure shaping a pool that cannot be built, such as one setting a timeout
with no runtime, is an [`Error::Custom`](crate::Error::Custom) from the
constructor. A `get` that fails is an [`Error::Pool`](crate::Error::Pool),
holding a [`PoolError`](crate::PoolError) that says why: a timeout, and
which of the three; the pool closed; a hook refused the connection; or the
connection could not be opened.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("configuration_builder").await;
# let config = scratch.config();
use std::time::Duration;

use pgorm::{Error, PoolError, Runtime, Status, TimeoutType, connect_with_builder};

let pool = connect_with_builder(config.clone(), |builder| {
    builder
        .max_size(2)
        .runtime(Runtime::Tokio1)
        .wait_timeout(Some(Duration::from_millis(200)))
        .create_timeout(Some(Duration::from_secs(5)))
})?;

let first = pool.get().await?;
let second = pool.get().await?;
let status: Status = pool.status();
assert_eq!((status.max_size, status.size, status.available), (2, 2, 0));

// Both connections are lent, so a third `get` waits, then gives up.
assert!(matches!(
    pool.get().await,
    Err(Error::Pool(PoolError::Timeout(TimeoutType::Wait)))
));
drop((first, second));
assert_eq!(pool.status().available, 2);

// A timeout with no runtime to time it on cannot be built.
let unbuildable = connect_with_builder(config, |builder| {
    builder.wait_timeout(Some(Duration::from_secs(1)))
});
assert!(matches!(unbuildable, Err(Error::Custom(_))));
# pool.close();
# scratch.finish().await;
# Ok(())
# }
```

[`DatabasePool::status`](crate::DatabasePool::status) is a snapshot of the
pool: its `max_size`, how many connections it holds (`size`), how many of
those are idle (`available`), and how many callers are `waiting`. It is worth
exporting as a gauge.

A hook runs code of yours on a connection the pool holds. A `post_create`
hook runs once on each new connection before the pool lends it, which makes
it the place for a check or a setup the `Config` cannot express. A hook that
returns a [`HookError`](crate::HookError) refuses the connection, and the
`get` that opened it fails with `PoolError::PostCreateHook`:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("configuration_hooks").await;
# let config = scratch.config();
use pgorm::{ConnectionTrait, Error, Hook, HookError, PoolError, connect_with_builder};

/// A hook refusing a connection to a server older than `minimum`, a
/// `server_version_num` such as 180000 for PostgreSQL 18.
fn requiring(minimum: i32) -> Hook {
    Hook::async_fn(move |client, _| {
        Box::pin(async move {
            let row = client
                .query_one("SELECT current_setting('server_version_num')::int", &[])
                .await
                .map_err(HookError::Backend)?;
            let version: i32 = row.get(0);
            if version < minimum {
                return Err(HookError::message(format!(
                    "server version {version} is older than {minimum}"
                )));
            }
            Ok(())
        })
    })
}

let pool = connect_with_builder(config.clone(), |builder| builder.post_create(requiring(180000)))?;
let db = pool.get().await?;
assert_eq!(db.query_one("SELECT 1", &[]).await?.get::<_, i32>(0), 1);

let refusing = connect_with_builder(config, |builder| builder.post_create(requiring(990000)))?;
assert!(matches!(
    refusing.get().await,
    Err(Error::Pool(PoolError::PostCreateHook(HookError::Message(_))))
));
# drop(db);
# pool.close();
# refusing.close();
# scratch.finish().await;
# Ok(())
# }
```

The client a hook is given is tokio-postgres's.
[`HookConnection::new(client)`](crate::HookConnection) borrows it for the
hook's length as a [`ConnectionTrait`](crate::ConnectionTrait), so pgorm's
own statements run in a hook, an entity's `find` among them, through the
connection's statement cache: a statement a hook prepares is the one the
connection's borrowers then find cached. Here a `post_create` hook names
each new session after the bakery it serves, the first by id:

```rust
# use pgorm_guide_support::bakery;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("configuration_hook_connection").await;
# let config = scratch.config();
use pgorm::entity::prelude::*;
use pgorm::{ConnectionTrait, Hook, HookConnection, HookError, connect_with_builder};

fn naming() -> Hook {
    Hook::async_fn(|client, _| {
        Box::pin(async move {
            let db = HookConnection::new(client);
            let refuse = |error: pgorm::Error| HookError::message(error.to_string());
            let first = bakery::Entity::find()
                .order_by_asc(bakery::Column::Id)
                .one(&db)
                .await
                .map_err(refuse)?;
            db.execute("SELECT set_config('application_name', $1, false)", &[&first.name])
                .await
                .map_err(refuse)?;
            Ok(())
        })
    })
}

let pool = connect_with_builder(config, |builder| builder.post_create(naming()))?;
let db = pool.get().await?;
let name: String = db
    .query_one("SELECT current_setting('application_name')", &[])
    .await?
    .get(0);
assert_eq!(name, "Sunrise");
# drop(db);
# pool.close();
# scratch.finish().await;
# Ok(())
# }
```

## Reusing connections

A connection returned to the pool keeps its session: a setting changed with
`SET`, a temporary table, an advisory lock, a `LISTEN`. The next caller to
take it finds them. Before lending a connection again, the pool *recycles*
it, checking it in the way the
[`RecyclingMethod`](crate::RecyclingMethod) says:

| Method | Before each reuse |
| --- | --- |
| `Fast`, the default | checks that the connection is still open, and sends nothing |
| `Verified` | also runs an empty query, catching a connection the network dropped without telling it |
| `Clean` | also resets the session: `CLOSE ALL; SET SESSION AUTHORIZATION DEFAULT; RESET ALL; UNLISTEN *; SELECT pg_advisory_unlock_all(); DISCARD TEMP; DISCARD SEQUENCES;` |
| `Custom(sql)` | runs your SQL |

`Clean` leaves the prepared statements alone, so the [statement
cache](crate::guide::statement_caches) keeps working. A `Custom` method that
deallocates them, such as `DISCARD ALL`, leaves the cache naming statements
the server no longer has; pair one with `StatementCacheSize::Disabled`.

The method is part of the [`ManagerConfig`](crate::ManagerConfig), which only
[`connect_with`](crate::connect_with) takes. Beside it the `ManagerConfig`
holds the pool's `tag`, its name; the `statement_cache` bound; and the
`statement_cache_channel` that [Statement caches and
DDL](crate::guide::statement_caches) describes.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("configuration_recycling").await;
# let config = scratch.config();
use pgorm::{ConnectionTrait, ManagerConfig, NoTls, RecyclingMethod, connect_with};

// What the next caller sees of a setting the last one changed.
for (recycling_method, seen) in [
    (RecyclingMethod::Fast, "off"),
    (RecyclingMethod::Clean, "on"),
] {
    let manager = ManagerConfig {
        recycling_method,
        tag: Some("kitchen".to_owned()),
        ..ManagerConfig::default()
    };
    let pool = connect_with(config.clone(), NoTls, manager, |builder| builder.max_size(1))?;
    assert_eq!(pool.tag().as_str(), "kitchen");

    let db = pool.get().await?;
    db.batch_execute("SET enable_seqscan = off").await?;
    drop(db);

    // The pool has one connection, so this is the same session.
    let db = pool.get().await?;
    let row = db.query_one("SELECT current_setting('enable_seqscan')", &[]).await?;
    let setting: String = row.get(0);
    assert_eq!(setting, seen);
#   drop(db);
#   pool.close();
}
# scratch.finish().await;
# Ok(())
# }
```

Under `Fast`, the setting one caller made is still there for the next; under
`Clean`, the pool reset it. Code that changes session state on a pooled
connection either recycles with `Clean` or puts the state back itself; a
setting meant for one transaction is better made with `SET LOCAL`, which
ends with the transaction.

A connection whose state is not known at all, such as one whose statement
was cancelled half way, can be thrown away rather than returned:
[`DatabaseConnection::discard`](crate::DatabaseConnection::discard) closes it,
and the pool opens a fresh one when it next needs one.

## TLS

A server that requires TLS, as most managed PostgreSQL services do, is
reached through [`connect_with`](crate::connect_with) with a TLS connector:
any type implementing tokio-postgres's
[`MakeTlsConnect<Socket>`](crate::MakeTlsConnect), such as `MakeRustlsConnect`
from the [tokio-postgres-rustls](https://docs.rs/tokio-postgres-rustls) crate
or `MakeTlsConnector` from
[postgres-openssl](https://docs.rs/postgres-openssl). Build the connector
with the root certificates the server's certificate chains to, and set the
`Config`'s `ssl_mode` to [`SslMode::Require`](crate::SslMode::Require) so
that the connection fails rather than falls back to plain text.
[`NoTls`](crate::NoTls) is the connector the other constructors pass.

`connect_with` and [`connect_listener`](crate::connect_listener) are generic
over the connector, and code of yours can be too. The bounds are the ones
`connect_with` states, every type in them re-exported from pgorm:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("configuration_tls").await;
# let config = scratch.config();
use pgorm::{
    ConnectionTrait, DatabasePool, Error, MakeTlsConnect, ManagerConfig, NoTls, Socket,
    TlsConnect, connect_with,
};

/// The application's pool, over whichever connector the deployment uses.
fn bakery_pool<T>(config: pgorm::Config, tls: T) -> Result<DatabasePool, Error>
where
    T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
    T::Stream: Sync + Send,
    T::TlsConnect: Sync + Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    connect_with(config, tls, ManagerConfig::default(), |builder| builder.max_size(8))
}

let pool = bakery_pool(config, NoTls)?;
let db = pool.get().await?;
let ssl: bool = db
    .query_one("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()", &[])
    .await?
    .get(0);
assert!(!ssl);
# drop(db);
# pool.close();
# scratch.finish().await;
# Ok(())
# }
```

A listener opened with [`DatabasePool::listener`](crate::DatabasePool::listener)
uses the pool's connector, so it speaks TLS when the pool does.

## Several pools

Some programs keep more than one pool against one server: a small one for
background jobs beside the web requests', say, so a burst of jobs cannot take
every connection.
[`connect_multi_with_builder`](crate::connect_multi_with_builder) builds one
pool per name, each tagged with its name and shaped by its own closure, and
returns them as a [`DatabaseMultiPool`](crate::DatabaseMultiPool). Its pools
speak plaintext and recycle with `Fast`;
[`connect_multi_with`](crate::connect_multi_with) takes a TLS connector and
a `ManagerConfig` beside the closures, as `connect_with` does, and gives
each pool the manager's configuration under its own name.
[`get`](crate::DatabaseMultiPool::get) gives the pool with a tag,
[`iter`](crate::DatabaseMultiPool::iter) every pool with its tag,
[`status`](crate::DatabaseMultiPool::status) each pool's status by tag, and
[`close`](crate::DatabaseMultiPool::close) closes them all. Choosing between
them is the program's: nothing balances or falls back across them.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("configuration_multi").await;
# let config = scratch.config();
use std::collections::BTreeMap;

use pgorm::{ConnectionTrait, PoolBuilder, connect_multi_with_builder};

let mut shapes: BTreeMap<String, Box<dyn Fn(PoolBuilder) -> PoolBuilder>> = BTreeMap::new();
shapes.insert("web".to_owned(), Box::new(|builder| builder.max_size(8)));
shapes.insert("jobs".to_owned(), Box::new(|builder| builder.max_size(2)));

let pools = connect_multi_with_builder(config, shapes)?;
let jobs = pools.get("jobs").expect("a pool is tagged jobs");
assert_eq!(jobs.tag().as_str(), "jobs");
let db = jobs.get().await?;
assert_eq!(db.query_one("SELECT 1", &[]).await?.get::<_, i32>(0), 1);

let sizes: Vec<(&str, usize)> = pools
    .status()
    .into_iter()
    .map(|(tag, status)| (tag, status.max_size))
    .collect();
assert_eq!(sizes, [("jobs", 2), ("web", 8)]);
assert!(pools.get("reports").is_none());

drop(db);
pools.close();
assert!(pools.iter().all(|(_, pool)| pool.is_closed()));
# scratch.finish().await;
# Ok(())
# }
```

A pool built without a tag is given one, `default-0`, `default-1` and so on,
counting the untagged pools the process has built.

## Closing a pool

[`DatabasePool::close`](crate::DatabasePool::close) closes the pool: callers
waiting for a connection get an error, idle connections are closed, and every
later `get` fails. A connection already checked out stays usable until its
holder drops it. Dropping the last clone of a pool closes it too.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("configuration_close").await;
# let pool = scratch.pool.clone();
use pgorm::ConnectionTrait;

let db = pool.get().await?;
pool.close();
assert!(pool.is_closed());
assert!(pool.get().await.is_err());

// The connection already lent still works until it is dropped.
let one: i32 = db.query_one("SELECT 1", &[]).await?.get(0);
assert_eq!(one, 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```
