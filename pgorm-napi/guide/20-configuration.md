# Connection configuration and TLS

[Getting started](01-getting-started.md) built a pool from a connection string
and used defaults for everything else. This chapter covers the rest: what the
connection string can say, how big the pool is and how long it waits, how
connections are checked when they are reused, TLS, and pools kept side by
side. It is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/19-configuration.md) of the same name.

JavaScript has three ways to build pools:

| Call | Does |
| --- | --- |
| [`new Pool(dsn, options)`](../lib/index.d.ts) | builds the pool, sending nothing |
| [`connect(dsn, options)`](../lib/index.d.ts) | builds the pool and pings the server, resolving once it answers |
| [`new MultiPool(dsn, pools, options)`](../lib/index.d.ts) | builds one pool per name, sending nothing |

The first two take the same [`PoolOptions`](../lib/index.d.ts), a plain
object. Rust has four constructors, each a shorthand for `connect_with`, which
takes a `Config`, a TLS connector, a `ManagerConfig` and a closure shaping
deadpool's `PoolBuilder`. JavaScript's options hold all four: the connection
string is the `Config`, the `tls`, `ca`, `cert` and `key` options configure
the TLS connector, and the rest are the `ManagerConfig`'s and the
`PoolBuilder`'s, durations in milliseconds. A pool opens connections when they are first asked for, so
`new Pool` with a wrong host, password or database name fails at the first
statement, and `connect` fails at once. An option of the wrong type is a
`TypeError`, and a value the pool cannot use a `ConstructionError`: `new
Pool` throws either as the pool is built, and `connect` rejects with it.

## The connection's settings

The connection string is parsed as tokio-postgres's `Config` is, the
configuration Rust's pool takes: a URL, or libpq's `key=value` form. Whatever
Rust sets with `Config`'s builder methods, JavaScript writes into the string:
`host`, `port`, `user`, `password`, `dbname`, `application_name`,
`keepalives_idle`, `target_session_attrs`, which Rust sets with a
`TargetSessionAttrs` and a string spells `read-write` to refuse a read-only
standby, and `options`, which passes server settings for every session, such
as a `search_path` or a `statement_timeout`. In a URL each is a
query parameter, its value percent-encoded with `encodeURIComponent`.
`URLSearchParams` writes a space as `+`, which the parser does not read as a
space, so it does not suit these values:

```ts
import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";

const settings = "-c search_path=kitchen,public -c statement_timeout=5s";
const configured = new URL(url);
configured.search += `&application_name=bakery-web&target_session_attrs=read-write&options=${encodeURIComponent(settings)}`;

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
- `connectTimeout` bounds how long a new connection may take to open, 10 000
  ms by default.
- `waitTimeout`, `createTimeout` and `recycleTimeout` are Rust's
  `wait_timeout`, `create_timeout` and `recycle_timeout`: how long taking a
  connection waits for one to come free, how long opening a new one may take,
  and how long checking a returned one may take. Each is off unless given.
  The first two reject the acquisition with a `TimeoutError` naming the
  timeout; a check that outlasts `recycleTimeout` discards the connection,
  and the pool lends another or opens one.
- `queueMode` is `"fifo"`, the default, to lend the idle connection returned
  first, or `"lifo"` to lend the one returned last, as Rust's `QueueMode`.

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

// waitTimeout gives up sooner, and says which bound it was.
await using strict = new Pool(url, { maxSize: 1, waitTimeout: 50 });
const only = await strict.acquire();
await assert.rejects(strict.acquire(), (error: unknown) => error instanceof TimeoutError && /waitTimeout/.test(error.message));
await only.close();

// A pool that cannot be built is refused as it is made.
assert.throws(() => new Pool(url, { maxSize: 0 }), ConstructionError);
assert.throws(() => new Pool(url, { maxSize: "2" as unknown as number }), TypeError);
```

[`pool.status()`](../lib/index.d.ts) is a snapshot of the pool: its
`maxSize`, how many connections it holds (`size`), how many of those are idle
(`available`), and how many callers are `waiting`. It is worth exporting as a
gauge.

A pool's options are read by name, and one it does not take, such as a
misspelt one, is a `TypeError` naming it.

A hook runs code of yours on a connection the pool holds, as Rust's
`post_create`, `pre_recycle` and `post_recycle` do: `postCreate` on each new
connection before the pool lends it, `preRecycle` on a returned one before the
pool checks it for reuse, and `postRecycle` after. Each is a function, called
on this thread with a [`HookConnection`](../lib/index.d.ts): the connection
the hook is about, whose `execute`, `query`, `one` and `optional` run as a
connection's do, and which reports the connection's `age` in milliseconds and
its `recycleCount`. It may be `async`, and the pool waits for its promise.
Once the hook has returned the `HookConnection` is the pool's again, and its
statements reject with a `LifecycleError`.

A hook that throws or rejects refuses the connection. A refused `postCreate`
fails the acquisition with a `ConnectionError` naming the hook and carrying
the error's name and message; a refused `preRecycle` or `postRecycle`
discards the connection, and the pool lends another or opens one.

```ts
import assert from "node:assert/strict";
import { ConnectionError, type HookConnection, Pool } from "@necessary/pgorm";

/** A hook refusing a server older than `minimum`, a `server_version_num` such as 180000. */
function requiring(minimum: number) {
  return async (db: HookConnection) => {
    const { version } = await db.one("SELECT current_setting('server_version_num')::int AS version");
    if ((version as number) < minimum) throw new Error(`server version ${version} is older than ${minimum}`);
  };
}

await using checked = new Pool(url, { postCreate: requiring(180000) });
assert.equal((await checked.one("SELECT 1 AS one")).one, 1);

await using refusing = new Pool(url, { postCreate: requiring(990000) });
await assert.rejects(refusing.acquire(), (error: unknown) =>
  error instanceof ConnectionError && error.message.startsWith("the postCreate hook refused the connection"));
```

## Reusing connections

A connection returned to the pool keeps its session: a setting changed with
`SET`, a temporary table, an advisory lock, a `LISTEN`. The next caller to
take it finds them. Before lending a connection again, the pool *recycles*
it, checking it in the way its `recycle` option says:

| Option | Before each reuse |
| --- | --- |
| `recycle: "verified"`, the default | checks that the connection is still open, and runs an empty query, catching a connection the network dropped without telling it |
| `recycle: "fast"` | only checks that the connection is still open, sending nothing |
| `recycle: "clean"` | also resets the session: `CLOSE ALL; SET SESSION AUTHORIZATION DEFAULT; RESET ALL; UNLISTEN *; SELECT pg_advisory_unlock_all(); DISCARD TEMP; DISCARD SEQUENCES;` |
| `recycleSql: sql` | runs your SQL |

These are Rust's `RecyclingMethod::Verified`, `Fast`, `Clean` and
`Custom(sql)`. Rust's default is `Fast`; JavaScript's is `Verified`. Giving
both `recycle` and `recycleSql` is a `TypeError`. `"clean"` leaves the
prepared statements alone, so the [statement cache](19-statement-caches.md)
keeps working; `recycleSql` that deallocates them, such as `DISCARD ALL`,
leaves the cache naming statements the server no longer has, so pair it with
`statementCacheSize: 0`.

Under `"verified"` and `"fast"` the setting one caller made is still there for
the next; under `"clean"` the pool reset it:

```ts
import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";

// What the next caller sees of a setting the last one changed.
for (const [recycle, seen] of [["verified", "off"], ["fast", "off"], ["clean", "on"]] as const) {
  await using single = new Pool(url, { maxSize: 1, recycle });

  let db = await single.acquire();
  await db.execute("SET enable_seqscan = off");
  await db.close();

  // The pool has one connection, so this is the same session.
  db = await single.acquire();
  const row = await db.one("SELECT current_setting('enable_seqscan') AS seen");
  assert.equal(row.seen, seen, recycle);
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

A connection whose state the program changed and cannot put back can be
thrown away rather than returned: `discard()` closes it as Rust's
`DatabaseConnection::discard` does, and the pool opens a fresh one when it
next needs one.

```ts
import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";

await using single = new Pool(url, { maxSize: 1 });
const db = await single.acquire();
const { pid } = await db.one("SELECT pg_backend_pid() AS pid");
await db.execute("SELECT set_config('application_name', 'half-finished', false)");
await db.discard();

const fresh = await single.one("SELECT pg_backend_pid() AS pid, current_setting('application_name') AS name");
assert.notEqual(fresh.pid, pid);
assert.notEqual(fresh.name, "half-finished");
```

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

Here JavaScript differs from Rust in shape. Rust's `connect_with` takes any
TLS connector implementing tokio-postgres's `MakeTlsConnect<Socket>`, which
the program builds and configures, and the shorter constructors connect in
plain text with `NoTls`. JavaScript's pool has one connector built in,
rustls, and the program configures it with options: the `tls` mode, named as
libpq's `sslmode` names it, the `ca` it trusts, and `cert` and `key`, a
client certificate and its private key, as PEM in a string or a `Uint8Array`:

| `tls` | The server's certificate |
| --- | --- |
| `"verify-full"` | must chain to a trusted certificate and name the host connected to |
| `"verify-ca"` | must chain to a certificate `ca` gives, whatever host it names; without `ca` the mode is refused |
| `"require"` | is not verified: the connection is encrypted, to whoever holds the key of the certificate the server sent |
| `"prefer"` | is not verified, and a server that offers no TLS is reached in plain text |
| `"disable"` | is never asked for: plain text |

With no `tls` option the string's `sslmode` decides: `disable` is plain text,
and `prefer`, `require` or none is `"verify-full"`. So a mode that verifies
less is always the program's explicit choice, never a default a connection
string carried in. libpq's `allow` is not offered: it tries plain text first,
so a server's impostor is always given the plain text. A client certificate
is shown to a server that asks for one, such as one whose `pg_hba.conf` lets a
role in by its certificate (`cert`). What the pool would not do as asked is a
`ConstructionError` as it is built:

```ts
import assert from "node:assert/strict";
import { ConstructionError, Pool } from "@necessary/pgorm";

const server = "postgres://bakery@db.example.com/bakery";
const pem = "-----BEGIN CERTIFICATE-----\nMA==\n-----END CERTIFICATE-----\n";

// Encrypted but unverified, which only the option can ask for.
const unverified = new Pool(server, { tls: "require" });
await unverified.close();

// verify-ca trusts the CA it is given, never the platform's store.
assert.throws(() => new Pool(server, { tls: "verify-ca" }), ConstructionError);
// A CA beside a mode that verifies nothing would trust nothing.
assert.throws(() => new Pool(server, { tls: "require", ca: pem }), ConstructionError);
// A client certificate comes with its key.
assert.throws(() => new Pool(server, { cert: pem }), ConstructionError);
// The string demands TLS, and prefer would allow plain text.
assert.throws(() => new Pool(`${server}?sslmode=require`, { tls: "prefer" }), ConstructionError);
```

A listener opened with `pool.listener()` uses the pool's TLS, so it speaks
TLS when the pool does, and `connectListener` takes the same options for a
listener of no pool ([LISTEN and NOTIFY](18-listen-notify.md)).

## Several pools

Some programs keep more than one pool against one server: a small one for
background jobs beside the web requests', say, so a burst of jobs cannot take
every connection. [`new MultiPool(dsn, pools, options)`](../lib/index.d.ts)
builds one pool per property of `pools`, each tagged with its name and shaped
by its own options — the size, the timeouts, the queue mode and the hooks,
which Rust's per-pool `PoolBuilder` closure sets — every one sharing
`options`: TLS, the recycling method and the statement cache. It is Rust's
`connect_multi_with`. `get(tag)` gives the pool with a tag, or `null`,
`entries()` every pool with its tag in the tags' order, `status()` each pool's
status by tag, and `close()` closes them all. Choosing between them is the
program's: nothing balances or falls back across them.

```ts
import assert from "node:assert/strict";
import { MultiPool } from "@necessary/pgorm";

await using pools = new MultiPool(url, { web: { maxSize: 8 }, jobs: { maxSize: 2 } }, { recycle: "clean" });
const jobs = pools.get("jobs");
assert.ok(jobs !== null);
assert.equal(jobs.tag, "jobs");
assert.equal((await jobs.one("SELECT 1 AS one")).one, 1);

assert.deepEqual(
  Object.entries(pools.status()).map(([tag, status]) => [tag, status.maxSize]),
  [["jobs", 2], ["web", 8]],
);
assert.equal(pools.get("reports"), null);
```

A pool's `tag` option names it, and `pool.tag` reports the name; a pool built
without one is given `default-0`, `default-1` and so on, counting the untagged
pools the process has built.

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
