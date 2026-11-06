# Connection configuration and TLS

[Getting started](01-getting-started.md) built a pool from a connection URL
and nothing else. This chapter covers the rest: what the URL can say, how big
the pool is and how long it waits, how connections are checked when they are
reused, TLS, and pools kept side by side.

Here Python differs from Rust in shape. Rust has four constructors,
`connect`, `connect_with_builder`, `connect_multi_with_builder` and
`connect_with`, each a shorthand for the next, taking a parsed `Config`,
closures over deadpool's `PoolBuilder`, a `ManagerConfig` and a TLS
connector. Python has `Pool`, which takes the connection string and, as
keyword options, everything those arguments say, and `MultiPool`, which
builds several pools at once ([Several pools](#several-pools)):

| Option | Default | Sets | Rust's |
| --- | --- | --- | --- |
| `max_size` | `10` | how many connections the pool opens at most | `PoolBuilder::max_size` |
| `connect_timeout` | `10.0` | seconds a new connection may take to connect | `Config::connect_timeout` |
| `acquire_timeout` | `30.0` | seconds a caller waits for a connection before `pgorm.TimeoutError` | none |
| `wait_timeout`, `create_timeout`, `recycle_timeout` | `None` | seconds waiting for a free connection, opening one and checking a returned one may take | `PoolBuilder`'s of the same names |
| `queue_mode` | `"fifo"` | whether the pool lends the idle connection returned first or last, `"lifo"` | `PoolBuilder::queue_mode` |
| `post_create`, `pre_recycle`, `post_recycle` | `None` | hooks run on a connection as it opens, before a returned one is checked, and after | `PoolBuilder`'s of the same names |
| `recycle` | `"verified"` | how a returned connection is checked before reuse: `"verified"`, `"fast"` or `"clean"` | `ManagerConfig::recycling_method` |
| `recycle_sql` | `None` | SQL of your own that recycles a connection, in place of `recycle` | `RecyclingMethod::Custom` |
| `statement_cache_size` | `128` | statements each connection keeps prepared; `0` turns the cache off | `ManagerConfig::statement_cache` |
| `statement_cache_channel` | `None` | a channel whose notifications clear the caches ([Statement caches and DDL](19-statement-caches.md)) | `ManagerConfig::statement_cache_channel` |
| `tag` | `None` | the pool's name | `ManagerConfig::tag` |
| `tls` | from the URL | how the pool uses TLS ([TLS](#tls)) | the connector |
| `ca`, `cafile` | `None` | the certificates TLS trusts, as PEM data or a file of it | the connector's roots |
| `cert`, `certfile`, `key`, `keyfile` | `None` | a client certificate and its private key | the connector's client certificate |

Building a pool connects to nothing. A pool opens connections when they are
first asked for, so a wrong host, password or database name shows up at the
first statement, or as the pool's `async with` block starts, since that
checks a connection. `await pgorm.connect(url, **options)` builds a pool and
checks a connection the same way before it returns, for a program that keeps
its pool outside an `async with` block. Here Python differs from Rust, whose
`connect` checks nothing:

```python
import asyncio
import os

import pgorm


async def main() -> None:
    # The server answered before `connect` returned.
    pool = await pgorm.connect(os.environ["DATABASE_URL"], max_size=2)
    try:
        assert pool.status().size == 1
        assert await pool.ping()
    finally:
        await pool.close()


asyncio.run(main())
```

An option of the wrong type, or a choice outside its spellings, raises
`TypeError`; a value the pool cannot use, such as a `max_size` of `0` or a
timeout that is not positive, raises `ConstructionError` as the pool is built
([Errors](09-errors.md)).

## The connection's settings

The connection string is tokio-postgres's connection configuration, written
as a URL or in libpq's `key=value` form. Either names the `host`, `port`,
`user`, `password` and `dbname`, and a URL's query or the `key=value` pairs
carry the rest: `application_name`, `keepalives_idle`, `sslmode`,
`channel_binding`, and `options`, which passes server settings for every
session, such as a `search_path` or a `statement_timeout`. A string that does
not parse raises `ConstructionError`, without repeating the string, which
may hold a password.

Here Python differs from Rust in spelling: Rust parses the string into a
`Config` and sets each part with a builder method. Python takes the string
alone, so a setting is part of it, percent-encoded in a URL:

```python
import asyncio
import os
from urllib.parse import parse_qsl, quote, urlencode, urlsplit, urlunsplit

from pgorm import Pool, RawSQL


def with_settings(url: str, **settings: str) -> str:
    """The URL with these settings in its query, replacing any of the same name."""
    parts = urlsplit(url)
    query = dict(parse_qsl(parts.query)) | settings
    return urlunsplit(parts._replace(query=urlencode(query, quote_via=quote)))


async def main() -> None:
    url = with_settings(
        os.environ["DATABASE_URL"],
        application_name="bakery-web",
        options="-c search_path=kitchen,public -c statement_timeout=5s",
    )
    async with Pool(url) as pool:

        async def setting(name: str) -> str:
            row = await pool.one(RawSQL("SELECT current_setting($1) AS value", [name]))
            value: str = row["value"]
            return value

        assert await setting("application_name") == "bakery-web"
        assert await setting("search_path") == "kitchen,public"
        assert await setting("statement_timeout") == "5s"

    # The key=value form says the same; building a pool connects to nothing.
    unused = Pool("host=localhost dbname=bakery user=postgres application_name=bakery-web sslmode=disable")
    await unused.close()


asyncio.run(main())
```

An `application_name` shows in `pg_stat_activity` and the server's log, which
makes the program's sessions easy to find. A `search_path` is how a program
whose tables live in a schema other than `public` reaches them without
qualifying every name.

The pool's `connect_timeout` option, ten seconds unless given, replaces a
`connect_timeout` the string sets: give it as the option.

## The pool's size and timeouts

These options shape the pool, as Rust's closure over its `PoolBuilder` does:

- `max_size` caps how many connections the pool opens, 10 unless given.
- `connect_timeout` bounds how long a new connection may take to connect.
  One that takes longer, or cannot open at all, fails as
  `pgorm.ConnectionError`.
- `wait_timeout`, `create_timeout` and `recycle_timeout` are Rust's: they
  bound waiting for a free connection, opening one, and checking a returned
  one before it is lent again. Each is off unless given. One of the first two
  that runs out raises `pgorm.TimeoutError`, its message naming which; a
  check that outlasts `recycle_timeout` discards the connection, and the
  pool lends another or opens one.
- `acquire_timeout` bounds the whole of an acquisition, whether it waits for
  a connection to come free or for a new one to open, 30 seconds unless
  given, and raises `pgorm.TimeoutError` too.
- `queue_mode` chooses which idle connection the pool lends: the one
  returned first, `"fifo"`, the default, or the one returned last,
  `"lifo"`, which leaves the others idle long enough for the server to see
  them go unused.

Here Python differs from Rust. Rust's pool is 2 connections per CPU unless
shaped, and with none of its timeouts set a `get` on an exhausted pool waits
until a connection is returned. A Python pool always has its acquire
deadline, and needs no runtime named for a timeout: its pools time on the
binding's own.

```python
import asyncio
import os

import pgorm
from pgorm import Pool


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"], max_size=2, acquire_timeout=0.2) as pool:
        first = await pool.acquire()
        second = await pool.acquire()
        status = pool.status()
        assert (status.max_size, status.size, status.available) == (2, 2, 0)

        # Both connections are lent, so a third waits, then gives up.
        try:
            await pool.acquire()
        except pgorm.TimeoutError as error:
            assert str(error) == "no connection became free within the pool's acquire_timeout"
        else:
            raise AssertionError("every connection is lent")

        await first.close()
        await second.close()
        assert pool.status().available == 2


asyncio.run(main())
```

`pool.status()` is a snapshot of the pool, a `PoolStatus` named tuple: its
`max_size`, how many connections it holds (`size`), how many of those are
idle (`available`), and how many callers are `waiting`. It is worth exporting
as a gauge. `connection.close()` returns a connection to its pool; so does
leaving the `async with pool.connection()` block.

A hook runs code of yours on a connection the pool holds. `post_create`
runs once on each new connection before the pool lends it, which makes it
the place for a check or a setup the connection string cannot express;
`pre_recycle` and `post_recycle` run on a returned connection before the
pool checks it for reuse and after. Each is called on the pool's event loop
with a `HookConnection`: the connection the hook is about, whose `execute`,
`query`, `one` and `optional` run as a connection's do, and which says how
old the connection is in seconds (`age`) and how many times it has been
recycled (`recycle_count`). A hook may be an `async def` or a plain
function. Once it returns, its `HookConnection` is the pool's again, and
using it raises `LifecycleError`.

A hook that raises refuses the connection. A refused new connection fails
the acquisition with `pgorm.ConnectionError`, which names the hook and the
exception; a refused returned one is discarded, and the pool lends another
or opens one. Here a `post_create` hook refuses a server older than the one
the program needs, and a `pre_recycle` hook retires connections older than
an hour:

```python
import asyncio
import os
from collections.abc import Awaitable, Callable

import pgorm
from pgorm import HookConnection, Pool, RawSQL


def requiring(minimum: int) -> Callable[[HookConnection], Awaitable[None]]:
    """A hook refusing a server older than ``minimum``, a ``server_version_num``
    such as 180000 for PostgreSQL 18."""

    async def check(connection: HookConnection) -> None:
        row = await connection.one(RawSQL("SELECT current_setting('server_version_num')::int AS version"))
        if row["version"] < minimum:
            raise RuntimeError(f"server version {row['version']} is older than {minimum}")

    return check


def retire_old(connection: HookConnection) -> None:
    if connection.age > 3600:
        raise RuntimeError("an hour old")


async def main() -> None:
    url = os.environ["DATABASE_URL"]
    async with Pool(url, post_create=requiring(180000), pre_recycle=retire_old) as pool:
        assert (await pool.one(RawSQL("SELECT 1 AS one")))["one"] == 1

    refusing = Pool(url, post_create=requiring(990000))
    try:
        await refusing.acquire()
    except pgorm.ConnectionError as error:
        assert str(error).startswith("the post_create hook refused the connection: RuntimeError:")
    else:
        raise AssertionError("the server is older than 99")
    finally:
        await refusing.close()


asyncio.run(main())
```

## Reusing connections

A connection returned to the pool keeps its session: a setting changed with
`SET`, a temporary table, an advisory lock, a `LISTEN`. The next caller to
take it finds them. Before lending a connection again, the pool *recycles*
it, checking it in the way the `recycle` option says:

| `recycle` | Before each reuse |
| --- | --- |
| `"verified"`, the default | runs an empty query, catching a connection the network dropped without telling it |
| `"fast"` | checks that the connection is still open, and sends nothing |
| `"clean"` | also resets the session: `CLOSE ALL; SET SESSION AUTHORIZATION DEFAULT; RESET ALL; UNLISTEN *; SELECT pg_advisory_unlock_all(); DISCARD TEMP; DISCARD SEQUENCES;` |
| `recycle_sql=` | runs your SQL, in place of `recycle` |

Here Python differs from Rust: Rust's default is `Fast`, and Python's is
`"verified"`. `"clean"` leaves the prepared statements alone, so the
[statement cache](19-statement-caches.md) keeps working. A `recycle_sql`
that deallocates them, such as `DISCARD ALL`, leaves the cache naming
statements the server no longer has; pair one with
`statement_cache_size=0`. Giving both `recycle` and `recycle_sql` raises
`TypeError`.

```python
import asyncio
import os

from pgorm import Pool, RawSQL

QUERY = RawSQL("SELECT current_setting('enable_seqscan') AS seqscan")


async def seen_by_the_next_caller(pool: Pool) -> str:
    """What the next caller sees of a setting the last one changed."""
    async with pool.connection() as connection:
        await connection.execute(RawSQL("SET enable_seqscan = off"))

    # The pool has one connection, so this is the same session.
    seen: str = (await pool.one(QUERY))["seqscan"]

    # A setting made with SET LOCAL ends with its transaction.
    async with pool.transaction() as tx:
        await tx.execute(RawSQL("SET LOCAL enable_seqscan = off"))
        assert (await tx.one(QUERY))["seqscan"] == "off"
    assert (await pool.one(QUERY))["seqscan"] == seen
    return seen


async def main() -> None:
    url = os.environ["DATABASE_URL"]
    async with Pool(url, max_size=1, recycle="verified", tag="kitchen") as pool:
        assert pool.tag == "kitchen"
        assert await seen_by_the_next_caller(pool) == "off"
    async with Pool(url, max_size=1, recycle="clean") as pool:
        assert await seen_by_the_next_caller(pool) == "on"


asyncio.run(main())
```

Under `"verified"`, the setting one caller made is still there for the next;
under `"clean"`, the pool reset it. Code that changes session state on a
pooled connection either recycles with `"clean"` or puts the state back
itself; a setting meant for one transaction is better made with `SET LOCAL`,
which ends with the transaction.

The `tag` names the pool, and `pool.tag` reports it; a pool built without
one is given one, `default-0`, `default-1` and so on, counting the untagged
pools the process has built.

A connection whose state is not known at all can be thrown away rather than
returned: `await connection.discard()` closes it, as Rust's
`DatabaseConnection::discard` does, and the pool opens a fresh one when it
next needs one. The binding discards a connection itself whenever its state
became unknown: a statement on it was cancelled, a `COMMIT` or `ROLLBACK`
failed, a `COPY` was refused or a stream was closed before its end.

```python
import asyncio
import os

from pgorm import Pool, RawSQL


async def main() -> None:
    pid = RawSQL("SELECT pg_backend_pid() AS pid")
    async with Pool(os.environ["DATABASE_URL"], max_size=1) as pool:
        connection = await pool.acquire()
        await connection.execute(RawSQL("SELECT set_config('search_path', 'kitchen', false)"))
        thrown_away = (await connection.one(pid))["pid"]
        await connection.discard()

        # The next caller gets a fresh session.
        assert (await pool.one(pid))["pid"] != thrown_away


asyncio.run(main())
```

## TLS

A pool verifies the server's certificate and host name by default, and never
falls back to plaintext. Here Python differs from Rust in shape: Rust's pool
speaks plaintext unless it is given a TLS connector through `connect_with`,
any type implementing tokio-postgres's `MakeTlsConnect<Socket>`. Python's
connector is built in, rustls, and the TLS configuration of your own is its
options: `tls`, the mode, named as libpq's `sslmode` names it; `ca` or
`cafile`, the certificates a verifying mode trusts; and `cert` and `key`, or
`certfile` and `keyfile`, a client certificate and its private key, as PEM,
shown to a server that asks for one.

| `tls` | A Python pool |
| --- | --- |
| `"verify-full"` | connects with TLS, the server's certificate chaining to a trusted one and naming the host |
| `"verify-ca"` | connects with TLS, the server's certificate chaining to one `ca` gives, whatever host it names |
| `"require"` | connects with TLS, verifying no certificate |
| `"prefer"` | connects with TLS when the server offers it and in plaintext when it does not, verifying nothing |
| `"disable"` | connects in plaintext |

Without the option the connection string's `sslmode` decides: `disable`
connects in plaintext, and `prefer`, `require` or none connect as
`"verify-full"`. So a mode that verifies less than `"verify-full"` is always
an option your program gave, never a default a connection string carried in.
`"verify-ca"` needs `ca` or `cafile`: the platform's store trusts
certificates issued to anyone, which says nothing about whose server this
is. libpq's `allow`, which tries plaintext first, is not a mode a pool
takes: it would hand an impostor the plaintext. Neither are libpq's
`sslrootcert`, `sslcert` and `sslkey` settings in the string, which
tokio-postgres's parser refuses: give them as the options.

A client certificate is how a server that authenticates by certificate
(`cert` in `pg_hba.conf`) lets the pool's user in, with no password: the
certificate's common name is the user. A server's `ssl_ca_file` names the
certificates a client's must chain to. With TLS, `channel_binding=require`
in the string binds a password exchange to the server's certificate.

The certificates the pool trusts are, in this order:

- the PEM certificates passed as `ca`, a `str` or `bytes`, or read from the
  file `cafile` names, when one is given, never both;
- otherwise the roots the platform's certificate store trusts: on macOS the
  system roots and any certificate the user or an administrator has marked
  trusted in Keychain, on Windows the root certificate store, and on Linux
  and other Unix systems the CA bundle and certificate directory OpenSSL
  would use, such as `/etc/ssl/certs`.

`SSL_CERT_FILE` and `SSL_CERT_DIR`, when set, replace the platform's store
rather than adding to it. The package carries no root list of its own, so a
pool trusts what the host trusts. The store is read when a verifying pool is
built, and a store holding no usable root certificate, as in a minimal
container image without a CA bundle or with `SSL_CERT_FILE` naming a missing
or empty file, raises `ConstructionError` there, naming what was wrong. It
never yields a pool that trusts nothing, or one that skips verification. Pass
`ca` or `cafile`, or install the host's CA bundle.

What a pool would not do as asked raises `ConstructionError` as it is built,
before anything is sent: plaintext beside certificates or the string's
`sslmode=require`, `"prefer"` beside `sslmode=require`, a CA beside a mode
that verifies nothing, a certificate without its key, PEM that holds no
certificate or no key. The guide's examples run against a server without
TLS, so this example shows what a plaintext pool reports and what a
verifying one refuses:

```python
import asyncio
import os
from collections.abc import Callable
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

import pgorm
from pgorm import ConstructionError, Pool, RawSQL


def without_sslmode(url: str) -> str:
    parts = urlsplit(url)
    query = [(key, value) for key, value in parse_qsl(parts.query) if key != "sslmode"]
    return urlunsplit(parts._replace(query=urlencode(query)))


async def main() -> None:
    url = os.environ["DATABASE_URL"]

    # The guide's server speaks plaintext, and says so.
    async with Pool(url, tls="disable") as pool:
        row = await pool.one(RawSQL("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()"))
        assert row["ssl"] is False

    # A verifying pool never falls back to plaintext.
    verifying = Pool(without_sslmode(url))
    try:
        await verifying.ping()
    except pgorm.ConnectionError:
        pass
    else:
        raise AssertionError("the server has no certificate to verify")
    finally:
        await verifying.close()

    # Certificates are given one way, and only to a pool that uses TLS.
    server = "postgres://postgres@db.example.com/bakery"
    refusals: list[tuple[Callable[[], Pool], str]] = [
        (lambda: Pool(server, ca="-----BEGIN CERTIFICATE-----", cafile="ca.pem"),
         "pass the CA as ca or as cafile, not both"),
        (lambda: Pool(server, ca="not a certificate"), "ca holds no certificate"),
        (lambda: Pool(server, tls="disable", ca=b"..."), "TLS configuration conflicts with tls='disable'"),
        (lambda: Pool(server, cafile="/no/such/ca.pem"), "could not read the CA certificate file"),
        (lambda: Pool(server, tls="verify-ca"),
         "tls='verify-ca' needs ca or cafile: the certificates the server's certificate must chain to"),
        (lambda: Pool(server, tls="require", ca="-----BEGIN CERTIFICATE-----"),
         "ca is trusted only to verify the server, which tls='require' does not do"),
        (lambda: Pool(server, tls="require", cert=b"-----BEGIN CERTIFICATE-----"),
         "cert and key go together: give the client certificate and its private key, or neither"),
    ]
    for build, reason in refusals:
        try:
            build()
        except ConstructionError as error:
            assert str(error) == reason
        else:
            raise AssertionError(reason)


asyncio.run(main())
```

A listener opened with `pool.listener()` uses the pool's TLS, so it speaks
TLS when the pool does, and `pgorm.connect_listener` takes the same TLS
options as a pool ([LISTEN and NOTIFY](18-listen-notify.md)).

## Several pools

Some programs keep more than one pool against one server: a small one for
background jobs beside the web requests', say, so a burst of jobs cannot take
every connection. `MultiPool` builds one pool per key, as Rust's
`connect_multi_with` does: each tagged with its key and shaped by its own
options — `max_size`, the four timeouts, `queue_mode` and the hooks — and
every one sharing the connection string, TLS and the rest, which the
keyword options after the mapping give. `get` gives the pool with a tag, or
`None`; `items` every pool with its tag, in the tags' order; `status` each
pool's status by tag; and `close`, or the end of its `async with` block,
closes them all. Each is a `Pool`. Choosing between them is the program's:
nothing balances or falls back across them.

```python
import asyncio
import os

from pgorm import MultiPool, RawSQL


async def main() -> None:
    url = os.environ["DATABASE_URL"]
    async with MultiPool(url, {"web": {"max_size": 8}, "jobs": {"max_size": 2}}, recycle="clean") as pools:
        jobs = pools.get("jobs")
        assert jobs is not None and jobs.tag == "jobs"
        assert (await jobs.one(RawSQL("SELECT 1 AS one")))["one"] == 1

        sizes = {tag: status.max_size for tag, status in pools.status().items()}
        assert sizes == {"jobs": 2, "web": 8}
        assert pools.get("reports") is None
    assert all(pool.closed for _, pool in pools.items())


asyncio.run(main())
```

## Closing a pool

`await pool.close()`, or `aclose()`, or the end of its `async with` block,
closes the pool: callers waiting for a connection get `LifecycleError`, the
operations running on its connections are interrupted, its listeners and
its cache-clearing connection are closed, and every later use raises
`LifecycleError`. It finishes once every connection is released.

Here Python differs from Rust. A Rust connection already checked out stays
usable until its holder drops it, and dropping the last clone of a pool
closes it. A Python pool's close reaches its checked-out connections too: a
connection lent before the close is closed with it. Close a pool explicitly,
or with its `async with` block, before its event loop ends ([Python and
asyncio](02-asyncio.md)).

```python
import asyncio
import os

import pgorm
from pgorm import Pool, RawSQL


async def main() -> None:
    pool = Pool(os.environ["DATABASE_URL"])
    connection = await pool.acquire()
    await pool.close()
    assert pool.closed

    # The connection lent before the close is closed with it.
    assert connection.closed
    try:
        await connection.one(RawSQL("SELECT 1 AS one"))
    except pgorm.LifecycleError as error:
        assert str(error) == "pool is closed"
    else:
        raise AssertionError("a closed pool's connections run nothing")

    try:
        await pool.acquire()
    except pgorm.LifecycleError:
        pass
    else:
        raise AssertionError("a closed pool lends nothing")


asyncio.run(main())
```
