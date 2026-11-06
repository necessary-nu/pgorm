# Statement caches and DDL

Every statement pgorm runs is prepared: the server parses and plans it once,
and the connection runs it again by name with new parameters. pgorm keeps the
statements each connection has prepared in a *statement cache* keyed by their
SQL text, so a query the program runs a thousand times is parsed once per
connection, not a thousand times. This is automatic, and most programs never
think about it. This chapter is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/18-statement-caches.md) of the same name.

A cached statement outlives the call that prepared it, and that is what this
chapter is about. A prepared statement was planned against the schema as it
was then. When DDL changes the schema under it, PostgreSQL usually notices and
plans the statement again, but not always. This chapter says when it does
not, and what to do.

## What the cache holds

Each connection has its own cache, shared by every transaction and savepoint
on it. Every statement a pool, a connection or a transaction runs, whether
SQL text or a statement a builder made, looks its SQL text up there and is
prepared on a miss. A COPY is prepared afresh each time ([COPY](17-copy.md)).

The cache is bounded, at 128 statements per connection unless the pool says
otherwise; Rust's default is 256. One query can be many texts, such as an
`IN` list rendered at every length it is used with, and each cached statement
is a live prepared statement on the server, so an unbounded cache would hold
statements the connection never runs again for as long as it lives. A full
cache evicts an entry to make room. The pool's `statementCacheSize` option
sets the bound, and `0` turns the cache off, in which case every statement is
prepared and closed again afterwards. It is Rust's `StatementCacheSize`, set
through `ManagerConfig` and `connect_with`; JavaScript takes it as an option
of the pool ([Connection configuration and TLS](20-configuration.md)):

```ts
import assert from "node:assert/strict";
import { type Connection, Pool } from "@necessary/pgorm";

/** How many statements this session holds prepared whose text starts with `prefix`. */
async function prepared(db: Connection, prefix: string): Promise<bigint> {
  const row = await db.one(
    "SELECT count(*) AS prepared FROM pg_prepared_statements WHERE starts_with(statement, $1)",
    [prefix],
  );
  return row.prepared as bigint;
}

for (const [statementCacheSize, kept] of [[64, 1n], [0, 0n]] as const) {
  await using cached = new Pool(url, { maxSize: 1, statementCacheSize });
  await using db = await cached.acquire();
  await db.query("SELECT name FROM bakery");
  assert.equal(await prepared(db, "SELECT name FROM bakery"), kept);
}
```

Rust's `batch_execute` runs a script of several statements through the
simple-query protocol, preparing nothing. JavaScript has no such method: each
call runs one statement, prepared.

## When the server notices

PostgreSQL plans a cached statement again by itself when what it reads
changes: an index added, a table dropped and created again, a different table
found through a changed `search_path`. One kind of change it cannot hide. A
statement's result columns were described to the client when it was
prepared, and if DDL changes them, as adding a column under `SELECT *` does,
the server refuses to run it with SQLSTATE `0A000`, *cached plan must not
change result type*.

pgorm handles that refusal for you as far as it safely can. It always drops
the refused statement from the cache, so the next call prepares it afresh.
Outside a transaction it also prepares the statement again at once and runs
it, a single retry. Inside a transaction it cannot: the refusal has already
aborted the transaction, so the `DatabaseError` with SQLSTATE `0A000` is
yours, and the recovery is yours to run at the transaction's boundary, for
example by running the transaction again ([Transactions](08-transactions.md)):

```ts
import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";

await using db = await pool.acquire();
const before = await db.query("SELECT * FROM baker");
assert.equal(Object.keys(before[0]!).length, 4);

await db.execute("ALTER TABLE baker ADD COLUMN apron text");

// Outside a transaction, the refused statement is prepared again and run.
const after = await db.query("SELECT * FROM baker");
assert.equal(Object.keys(after[0]!).length, 5);

await db.execute("ALTER TABLE baker DROP COLUMN apron");

// Inside one, the refusal is the transaction's error.
await assert.rejects(
  db.transaction((tx) => tx.query("SELECT * FROM baker")),
  (error) => error instanceof DatabaseError && error.sqlstate === "0A000",
);

// It was dropped from the cache, so running the transaction again succeeds.
const rows = await db.transaction((tx) => tx.query("SELECT * FROM baker"));
assert.equal(Object.keys(rows[0]!).length, 4);
```

Selecting named columns rather than `*`, as every query a model or a builder
makes does, keeps a statement's result from changing under it.

## When it does not: clearing the caches

Some DDL changes what a plan was built from without the server planning the
statement again. A domain's default is one: an `INSERT` prepared before
`ALTER DOMAIN .. SET DEFAULT` can go on writing the old default. For such
changes, [`pool.clearStatementCaches()`](../lib/index.d.ts) drops every
cached statement of every connection in the pool, closing each on the server,
so each statement is prepared again the next time it runs.

The call does not clear anything in place, because a connection that is
checked out may be preparing a statement at that very moment, against the
schema the DDL just replaced. It marks every connection's cache as out of
date instead. An idle connection is cleared before the pool next lends it; a
connection checked out at the time keeps its statements until it is returned,
and is cleared before it is lent again. So code holding a connection across
the DDL closes it and acquires another to see the change. The call sends
nothing, waits for nothing and cannot fail.

```ts
import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";

const INSERT = "INSERT INTO box (contents) VALUES ($1) RETURNING ribbon";
const PREPARED =
  "SELECT count(*) AS kept FROM pg_prepared_statements WHERE starts_with(statement, 'INSERT INTO box')";

// One connection, so every checkout is the same session.
await using single = new Pool(url, { maxSize: 1 });
let db = await single.acquire();
await db.execute("CREATE DOMAIN ribbon AS text DEFAULT 'red'");
await db.execute("CREATE TABLE box (id serial PRIMARY KEY, contents text, ribbon ribbon)");
assert.equal((await db.one(INSERT, ["truffles"])).ribbon, "red");

await db.execute("ALTER DOMAIN ribbon SET DEFAULT 'gold'");
single.clearStatementCaches();

// The connection was checked out, so it keeps its statement until returned.
assert.equal((await db.one(PREPARED)).kept, 1n);
await db.close();

// Lent again, it was cleared first, and the insert is prepared afresh.
db = await single.acquire();
assert.equal((await db.one(PREPARED)).kept, 0n);
assert.equal((await db.one(INSERT, ["fudge"])).ribbon, "gold");
await db.close();
```

pgorm-migration's runner clears the caches of the pool it was given once a
batch that applied a migration commits. JavaScript has no migration runner
([Migrations](22-migrations.md)), so a program that changes such things
itself calls `clearStatementCaches()` after.

## Clearing other processes' caches

`clearStatementCaches()` reaches the pool it is called on, in the process
that holds it. An application usually runs as several processes, and the one
that migrates is often none of them. For those, a pool can listen for a
clear: the `statementCacheChannel` option names a channel, and a pool given
one keeps a connection of its own `LISTEN`ing there ([LISTEN and
NOTIFY](18-listen-notify.md)). Every notification on the channel clears the
pool's caches as `clearStatementCaches()` does, whatever its payload. It is
Rust's `ManagerConfig::statement_cache_channel`.

It is off by default, because it costs each pool one connection, opened by the
pool's first acquisition. That connection is not lent, counts against no size
limit, and is reopened if it drops, with a wait of 100 ms that doubles to 10 s
between attempts; and since a notification sent while it was down reached
nobody, the caches are cleared each time its `LISTEN` takes effect, the first
time included. Closing the pool closes it. Nothing travels on it between
notifications, so a network middlebox that drops idle connections drops it
too; TCP keepalives, `keepalives_idle=60` in the connection string, keep it
open.

[`STATEMENT_CACHE_CHANNEL`](../lib/index.d.ts), `pgorm_statement_caches`, is
the channel pgorm-migration's runner notifies, inside the transaction of a
batch that applied a migration, so the notification is delivered exactly when
the migration commits. A process that changes the schema some other way sends
one itself with `notify`. A notification reaches only sessions on the
database it was sent in, so one channel name serves every database on a
server.

Here a second pool, standing for another process, writes a domain's default
and hears the clear the first sends after changing it:

```ts
import assert from "node:assert/strict";
import { Pool, STATEMENT_CACHE_CHANNEL } from "@necessary/pgorm";

assert.equal(STATEMENT_CACHE_CHANNEL, "pgorm_statement_caches");
const INSERT = "INSERT INTO box (contents) VALUES ($1) RETURNING ribbon";

// The process that migrates.
await pool.execute("CREATE DOMAIN ribbon AS text DEFAULT 'red'");
await pool.execute("CREATE TABLE box (id serial PRIMARY KEY, contents text, ribbon ribbon)");

// Another process's pool, listening for clears.
await using shop = new Pool(url, { maxSize: 1, statementCacheChannel: STATEMENT_CACHE_CHANNEL });
assert.equal((await shop.one(INSERT, ["truffles"])).ribbon, "red");

await pool.execute("ALTER DOMAIN ribbon SET DEFAULT 'gold'");
await pool.notify(STATEMENT_CACHE_CHANNEL, "");

// The clear arrives as a notification, so the shop sees it a moment later.
const deadline = Date.now() + 10_000;
let ribbon = "red";
while (ribbon !== "gold" && Date.now() < deadline) {
  ribbon = String((await shop.one(INSERT, ["fudge"])).ribbon);
  if (ribbon !== "gold") await new Promise((resolve) => setTimeout(resolve, 50));
}
assert.equal(ribbon, "gold");
```

The clear is the same deferred one: a connection the other process holds
checked out for its whole life keeps its statements until it closes it. The
`Pool` constructor refuses an empty channel name and one longer than 63
bytes, which no notification could reach, with a `ConstructionError`, where
Rust's `connect_with` returns an error:

```ts
import assert from "node:assert/strict";
import { ConstructionError, Pool } from "@necessary/pgorm";

assert.throws(() => new Pool(url, { statementCacheChannel: "" }), ConstructionError);
assert.throws(() => new Pool(url, { statementCacheChannel: "c".repeat(64) }), ConstructionError);
```

The metrics layer does not see these clears ([Metrics](21-metrics.md)).
