# Connection configuration and TLS

[Getting started](01-getting-started.md) built a pool from a connection URL
and nothing else. This chapter covers the rest: what the URL can say, how big
the pool is and how long it waits, how connections are checked when they are
reused, TLS, and pools kept side by side.

Here Python differs from Rust. Rust has four constructors, `connect`,
`connect_with_builder`, `connect_multi_with_builder` and `connect_with`, each
a shorthand for the next, taking a parsed `Config`, closures over deadpool's
`PoolBuilder`, a `ManagerConfig` and a TLS connector. Python has one, `Pool`,
which takes the connection string and keyword options:

| Option | Default | Sets |
| --- | --- | --- |
| `max_size` | `10` | how many connections the pool opens at most |
| `connect_timeout` | `10.0` | seconds a new connection may take to open |
| `acquire_timeout` | `30.0` | seconds a caller waits for a connection before `pgorm.TimeoutError` |
| `recycle` | `"verified"` | how a returned connection is checked before reuse: `"verified"` or `"fast"` |
| `statement_cache_size` | `128` | statements each connection keeps prepared; `0` turns the cache off |
| `statement_cache_channel` | `None` | a channel whose notifications clear the caches ([Statement caches and DDL](19-statement-caches.md)) |
| `tls` | from the URL | `"verify-full"` or `"disable"` |
| `ca`, `cafile` | `None` | the certificates TLS trusts, as PEM data or a file of it |

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

Three options shape the pool:

- `max_size` caps how many connections the pool opens, 10 unless given.
- `connect_timeout` bounds how long a new connection may take to open. One
  that takes longer, or cannot open at all, fails as `pgorm.ConnectionError`.
- `acquire_timeout` bounds how long a caller waits for a connection, whether
  it waits for one to come free or for a new one to open, 30 seconds unless
  given. A caller that waits longer gets `pgorm.TimeoutError`.

Here Python differs from Rust. Rust's pool is 2 connections per CPU unless
shaped, and its `wait_timeout`, `create_timeout` and `recycle_timeout` are
each off unless set, so a `get` on an exhausted pool waits until a connection
is returned. A Python pool always has its acquire deadline. Rust's
`queue_mode`, which chooses whether the pool lends the connection returned
first or last, and its `post_create`, `pre_recycle` and `post_recycle` hooks
are not available in Python yet.

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
            assert str(error) == "pool acquisition timed out"
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

## Reusing connections

A connection returned to the pool keeps its session: a setting changed with
`SET`, a temporary table, an advisory lock, a `LISTEN`. The next caller to
take it finds them. Before lending a connection again, the pool *recycles*
it, checking it in the way the `recycle` option says:

| `recycle` | Before each reuse |
| --- | --- |
| `"verified"`, the default | runs an empty query, catching a connection the network dropped without telling it |
| `"fast"` | checks that the connection is still open, and sends nothing |

Here Python differs from Rust. Rust's default is `Fast`, and Python's is
`"verified"`. Rust's `RecyclingMethod::Clean`, which resets the session
before each reuse, and `Custom(sql)`, which runs SQL of your own, are not
available in Python yet, so neither choice resets a session:

```python
import asyncio
import os

from pgorm import Pool, RawSQL


async def main() -> None:
    query = RawSQL("SELECT current_setting('enable_seqscan') AS seqscan")
    for recycle in ("verified", "fast"):
        async with Pool(os.environ["DATABASE_URL"], max_size=1, recycle=recycle) as pool:
            async with pool.connection() as connection:
                await connection.execute(RawSQL("SET enable_seqscan = off"))

            # The pool has one connection, so this is the same session.
            row = await pool.one(query)
            assert row["seqscan"] == "off"

            # A setting made with SET LOCAL ends with its transaction.
            async with pool.transaction() as tx:
                await tx.execute(RawSQL("SET LOCAL enable_seqscan = on"))
                assert (await tx.one(query))["seqscan"] == "on"
            assert (await pool.one(query))["seqscan"] == "off"


asyncio.run(main())
```

Code that changes session state on a pooled connection puts it back itself,
with `RESET` before it returns the connection; a setting meant for one
transaction is better made with `SET LOCAL`, which ends with the transaction.

Rust's `ManagerConfig` also names the pool with a `tag`; a Python pool has no
name. Rust's `DatabaseConnection::discard` closes a connection whose state is
not known rather than return it. Python has no `discard`: the binding
discards such a connection itself, whether a statement on it was cancelled,
a `COMMIT` or `ROLLBACK` failed, a `COPY` was refused or a stream was closed
before its end, and the pool opens a fresh one when it next needs one.

## TLS

A pool verifies the server's certificate and host name by default, and never
falls back to plaintext. Only `sslmode=disable` in the connection string, or
the option `tls="disable"`, connects without TLS. `tls="verify-full"` asks for
verified TLS whatever the string says.

Here Python differs from Rust. Rust's pool speaks plaintext unless it is
given a TLS connector through `connect_with`: any type implementing
tokio-postgres's `MakeTlsConnect<Socket>`, such as `MakeRustlsConnect` or
postgres-openssl's `MakeTlsConnector`. Python's TLS is built in, rustls with
verification always on, and choosing another connector, or a TLS
configuration of your own, is not available in Python yet. Nor is a client
certificate: a Python pool authenticates with its user's password, never
with a certificate of its own. The connection string's `sslmode` is read as
follows:

| `sslmode` | A Python pool |
| --- | --- |
| `disable` | connects in plaintext |
| `prefer`, `require`, or none | connects with verified TLS, and fails rather than fall back |

`verify-ca` and `verify-full` are not spellings the connection string takes,
nor are libpq's `sslrootcert` and `sslcert` settings: a string with one
raises `ConstructionError`. Verification is always full, and the
certificates it trusts are the `ca` or `cafile` option's. With TLS,
`channel_binding=require` in the string binds the password exchange to the
server's certificate.

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

The guide's examples run against a server without TLS, so this example shows
what a plaintext pool reports and what a verifying one refuses:

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
        (lambda: Pool(server, ca="not a certificate"), "CA data contains no certificates"),
        (lambda: Pool(server, tls="disable", ca=b"..."), "TLS configuration conflicts with tls='disable'"),
        (lambda: Pool(server, cafile="/no/such/ca.pem"), "could not read the CA certificate file"),
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
TLS when the pool does.

## Several pools

Some programs keep more than one pool against one server: a small one for
background jobs beside the web requests', say, so a burst of jobs cannot take
every connection. Choosing between them is the program's: nothing balances
or falls back across them.

Here Python differs from Rust: Rust's `connect_multi_with_builder` builds
one pool per name, each tagged with its name, into a map. It is not available
in Python, and needs no counterpart: build each `Pool` with its own options.
A Python pool has no tag.

```python
import asyncio
import os

from pgorm import Pool


async def main() -> None:
    url = os.environ["DATABASE_URL"]
    pools = {"web": Pool(url, max_size=8), "jobs": Pool(url, max_size=2)}
    try:
        assert pools["jobs"].status().max_size == 2
        # Neither has connected yet.
        assert [pool.status().size for pool in pools.values()] == [0, 0]
    finally:
        for pool in pools.values():
            await pool.close()


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
