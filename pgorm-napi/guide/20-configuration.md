# Connection configuration and TLS

[Getting started](01-getting-started.md) built a pool from a connection string
and used defaults for everything else. This chapter covers the rest: what the
connection string can say, how big the pool is and how long it waits, how
connections are checked when they are reused, TLS, and pools kept side by
side. It is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/19-configuration.md) of the same name.

JavaScript has two ways to build a pool:

| Call | Does |
| --- | --- |
| [`new Pool(dsn, options)`](../lib/index.d.ts) | builds the pool, sending nothing |
| [`connect(dsn, options)`](../lib/index.d.ts) | builds the pool and pings the server, resolving once it answers |

Both take the same [`PoolOptions`](../lib/index.d.ts), a plain object. Rust
has four constructors, each a shorthand for `connect_with`, which takes a
`Config`, a TLS connector, a `ManagerConfig` and a closure shaping deadpool's
`PoolBuilder`; JavaScript's options hold what of those it exposes, durations
in milliseconds. A pool opens connections when they are first asked for, so
`new Pool` with a wrong host, password or database name fails at the first
statement, and `connect` fails at once. An option of the wrong type is a
`TypeError`, and a value the pool cannot use a `ConstructionError`: `new
Pool` throws either as the pool is built, and `connect` rejects with it.

## The connection's settings

The connection string is parsed as tokio-postgres's `Config` is, the
configuration Rust's pool takes: a URL, or libpq's `key=value` form. Whatever
Rust sets with `Config`'s builder methods, JavaScript writes into the string:
`host`, `port`, `user`, `password`, `dbname`, `application_name`,
`keepalives_idle`, and `options`, which passes server settings for every
session, such as a `search_path` or a `statement_timeout`. In a URL each is a
query parameter, its value percent-encoded with `encodeURIComponent`.
`URLSearchParams` writes a space as `+`, which the parser does not read as a
space, so it does not suit these values:

```ts
import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";

const settings = "-c search_path=kitchen,public -c statement_timeout=5s";
const configured = new URL(url);
configured.search += `&application_name=bakery-web&options=${encodeURIComponent(settings)}`;

await using web = new Pool(configured.toString());
const seen = await web.one(
  "SELECT current_setting('application_name') AS name, current_setting('search_path') AS path, " +
    "current_setting('statement_timeout') AS timeout",
);
assert.deepEqual(seen, { name: "bakery-web", path: "kitchen,public", timeout: "5s" });
```

An `application_name` shows in `pg_stat_activity` and the server's log, which
makes the program's sessions easy to find. A `search_path` is how a program
whose tables live in a schema other than `public` reaches them without
qualifying every name. How long opening a connection may take is the pool's
`connectTimeout` option, 10 000 ms by default, which takes the place of the
string's `connect_timeout`. A pool's errors never carry the password the
string holds.

## The pool's size and timeouts

Rust shapes a pool through deadpool's `PoolBuilder`. JavaScript takes the
parts it exposes as options:

- `maxSize` caps how many connections the pool opens, 10 by default. Rust's
  default is twice the number of CPUs the machine reports.
- `acquireTimeout` bounds how long taking a connection waits for one to come
  free, 30 000 ms by default, after which it rejects with a
  [`TimeoutError`](../lib/index.d.ts). Rust's `wait_timeout` is off by
  default, and needs a runtime to time it on; JavaScript's always has one.
- `connectTimeout` bounds how long a new connection may take to open, as
  Rust's `create_timeout` does, 10 000 ms by default.

A statement run on the pool takes its connection the same way, so it too
waits at most `acquireTimeout` for one. An `AbortSignal` bounds any one wait
instead ([Promises, cancellation and cleanup](02-promises.md)); its timeout
rejects with the signal's own `TimeoutError` `DOMException`, not the pool's.

```ts
import assert from "node:assert/strict";
import { ConstructionError, Pool, TimeoutError } from "@necessary/pgorm";

await using small = new Pool(url, { maxSize: 2, acquireTimeout: 200, connectTimeout: 5000 });

const first = await small.acquire();
const second = await small.acquire();
const status = small.status();
assert.deepEqual(
  [status.maxSize, status.size, status.available, status.waiting],
  [2, 2, 0, 0],
);

// Both connections are lent, so a third waits, then gives up.
await assert.rejects(small.acquire(), TimeoutError);
await first.close();
await second.close();
assert.equal(small.status().available, 2);

// A pool that cannot be built is refused as it is made.
assert.throws(() => new Pool(url, { maxSize: 0 }), ConstructionError);
assert.throws(() => new Pool(url, { maxSize: "2" as unknown as number }), TypeError);
```

[`pool.status()`](../lib/index.d.ts) is a snapshot of the pool: its
`maxSize`, how many connections it holds (`size`), how many of those are idle
(`available`), and how many callers are `waiting`. It is worth exporting as a
gauge.

**Not available in JavaScript yet:** Rust's `PoolBuilder` also bounds how long
a returned connection's check may take (`recycle_timeout`), chooses whether
the pool lends the connection returned first or last (`queue_mode`), and
takes hooks that run as a connection is created and around its recycling
(`post_create`, `pre_recycle`, `post_recycle`). JavaScript's options have
none of these.

A pool's options are read by name, and an option it does not know is left
unread rather than refused, so check the spelling of each.

## Reusing connections

A connection returned to the pool keeps its session: a setting changed with
`SET`, a temporary table, an advisory lock, a `LISTEN`. The next caller to
take it finds them. Before lending a connection again, the pool *recycles*
it, checking it in the way its `recycle` option says:

| `recycle` | Before each reuse |
| --- | --- |
| `"verified"`, the default | checks that the connection is still open, and runs an empty query, catching a connection the network dropped without telling it |
| `"fast"` | only checks that the connection is still open, sending nothing |

These are Rust's `RecyclingMethod::Verified` and `Fast`. Rust's default is
`Fast`; JavaScript's is `Verified`.

**Not available in JavaScript yet:** Rust's `RecyclingMethod::Clean`, which
also resets the session before each reuse (`CLOSE ALL`, `RESET ALL`,
`UNLISTEN *`, the advisory locks released, temporary tables and sequences
discarded), and `Custom`, which runs SQL of the program's own. Neither
JavaScript method resets anything.

So under either method the setting one caller made is still there for the
next:

```ts
import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";

for (const recycle of ["verified", "fast"] as const) {
  await using single = new Pool(url, { maxSize: 1, recycle });

  let db = await single.acquire();
  await db.execute("SET enable_seqscan = off");
  await db.close();

  // The pool has one connection, so this is the same session.
  db = await single.acquire();
  const row = await db.one("SELECT current_setting('enable_seqscan') AS seen");
  assert.equal(row.seen, "off");
  await db.close();
}
```

Code that changes session state on a pooled connection puts the state back
itself. A setting meant for one transaction is better made with `SET LOCAL`,
which ends with the transaction:

```ts
import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";

await using single = new Pool(url, { maxSize: 1 });
await single.transaction(async (tx) => {
  await tx.execute("SET LOCAL enable_seqscan = off");
  assert.equal((await tx.one("SELECT current_setting('enable_seqscan') AS seen")).seen, "off");
});
assert.equal((await single.one("SELECT current_setting('enable_seqscan') AS seen")).seen, "on");
```

A connection whose operation was aborted part way, or whose COPY or stream
was left before its end, is in a state no one knows, and the pool discards
it rather than lend it again; it opens a fresh one when it next needs one.

**Not available in JavaScript yet:** Rust's `DatabaseConnection::discard`,
which throws away a connection the program holds, closing it rather than
returning it to the pool, such as one whose session state the program changed
and cannot put back. A JavaScript `Connection`'s `close()` always returns it
to the pool.

## TLS

A server that requires TLS, as most managed PostgreSQL services do, is reached
over TLS without asking: a pool verifies the server's certificate and its host
name unless the connection string says `sslmode=disable` or the `tls` option
is `"disable"`, and it never falls back to plain text. The certificates it
trusts are the platform's, read from its trust store, or the PEM certificates
the `ca` option gives, as a string or a `Uint8Array`. Building such a pool
sends nothing, so it can be built where no server answers:

```ts
import assert from "node:assert/strict";
import { ConstructionError, Pool } from "@necessary/pgorm";

// Verified against the platform's trust store; nothing is sent yet.
const managed = new Pool("postgres://bakery@db.example.com/bakery");
await managed.close();

// A CA that holds no certificate, and plain text beside a CA, are refused.
assert.throws(() => new Pool("postgres://bakery@db.example.com/bakery", { ca: "not a certificate" }), ConstructionError);
assert.throws(
  () => new Pool("postgres://bakery@db.example.com/bakery", { tls: "disable", ca: "not a certificate" }),
  ConstructionError,
);

// The bakery's own server is reached in plain text: its URL says sslmode=disable.
const { ssl } = await pool.one("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()");
assert.equal(ssl, false);
```

Here JavaScript differs from Rust in shape and in reach. Rust's
`connect_with` takes any TLS connector implementing tokio-postgres's
`MakeTlsConnect<Socket>`, and the shorter constructors connect in plain text
with `NoTls`. JavaScript's pool has one connector built in, rustls verifying
the server fully, and plain text only where the string or the `tls` option
says so. `sslmode=require` without `tls: "disable"` means the same verified
TLS.

**Not available in JavaScript yet:** a TLS connector of the program's own,
which Rust's `connect_with` takes, and with it what such a connector can do
that JavaScript's built-in one cannot: present a client certificate to the
server, or verify the server some other way than its certificate chain and
host name.

A listener opened with `pool.listener()` uses the pool's TLS, so it speaks
TLS when the pool does ([LISTEN and NOTIFY](18-listen-notify.md)).

## Several pools

Some programs keep more than one pool against one server: a small one for
background jobs beside the web requests', say, so a burst of jobs cannot take
every connection. In JavaScript each is a `Pool` of its own, built with its own
options. Choosing between them is the program's: nothing balances or falls
back across them.

```ts
import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";

await using web = new Pool(url, { maxSize: 8 });
await using jobs = new Pool(url, { maxSize: 2 });

assert.equal(web.status().maxSize, 8);
assert.equal(jobs.status().maxSize, 2);
```

**Not available in JavaScript yet:** a pool's tag, the name Rust's
`ManagerConfig` gives it and `DatabasePool::tag` reports, which
`connect_multi_with_builder` uses to build several pools at once and return
them keyed by tag.

## Closing a pool

[`pool.close()`](../lib/index.d.ts), or `await using`, closes the pool. It
refuses new work, cancels what still runs, and resolves once every connection
and listener it lent is released. Here JavaScript differs from Rust, whose
`close` leaves a connection already checked out usable until its holder drops
it: closing a JavaScript pool ends its checked-out connections too, and each
then refuses its next call.

```ts
import assert from "node:assert/strict";
import { LifecycleError, Pool } from "@necessary/pgorm";

const closing = new Pool(url);
const db = await closing.acquire();
await closing.close();
assert.equal(closing.closed, true);

await assert.rejects(closing.acquire(), LifecycleError);
await assert.rejects(db.one("SELECT 1 AS one"), LifecycleError);
```

Nothing waits on garbage collection: a pool collected without being closed
releases only what is idle ([Promises, cancellation and
cleanup](02-promises.md)), so close the pool when the program is done with it.
