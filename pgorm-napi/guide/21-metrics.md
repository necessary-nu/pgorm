# Metrics

There are two places to watch what a program asks of its database: the
database itself, and the program. PostgreSQL's own statistics see every
statement from every client, with its plan and its cost, and are usually the
best source of truth. Rust's pgorm adds a metrics layer, `pgorm::metric`,
which sees what the program sees: how long each statement took from its side,
including the wait for a connection. The JavaScript package has no such layer
yet, so this chapter, the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/20-metrics.md), teaches the server's statistics and
the pool's own state, and says what the layer would add.

## PostgreSQL's own statistics

The server can log statements and their durations. These settings take
effect on reload:

```sql
-- Log every statement.
ALTER SYSTEM SET log_statement = 'all';
-- Log each statement's duration.
ALTER SYSTEM SET log_duration = 'on';
-- Or log only statements slower than a second.
ALTER SYSTEM SET log_min_duration_statement = 1000;

SELECT pg_reload_conf();
```

The `pg_stat_statements` extension keeps running totals per query shape. It
has to be loaded when the server starts:

```sql
ALTER SYSTEM SET shared_preload_libraries = 'pg_stat_statements';
-- After a restart:
CREATE EXTENSION pg_stat_statements;

-- The statements that took the most time in all.
SELECT substring(query, 1, 50) AS query,
       calls,
       total_exec_time / calls AS mean_ms,
       100.0 * total_exec_time / sum(total_exec_time) OVER () AS percent
FROM pg_stat_statements
ORDER BY total_exec_time DESC
LIMIT 20;

-- Start counting again.
SELECT pg_stat_statements_reset();
```

The `auto_explain` module logs the plans of slow statements. Because pgorm
prepares every statement and binds its values, `pg_stat_statements` sees one
query per shape rather than one per value, which is what makes its totals
useful.

The server also says what each session is doing now, in `pg_stat_activity`.
A pool whose connection string names an `application_name` ([Connection
configuration and TLS](20-configuration.md)) makes the program's sessions
easy to pick out there:

```ts
import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";

const named = new URL(url);
named.search += "&application_name=bakery-web";
await using web = new Pool(named.toString(), { maxSize: 2 });

const row = await web.one(
  "SELECT count(*) AS sessions FROM pg_stat_activity WHERE application_name = $1 AND datname = current_database()",
  ["bakery-web"],
);
assert.equal(row.sessions, 1n);
```

The pool's own state is a snapshot away. [`pool.status()`](../lib/index.d.ts)
reports the pool's `maxSize`, how many connections it holds (`size`), how many
of those are idle (`available`), and how many callers are `waiting` for one;
a pool that is often at `available === 0` with callers waiting is too small
for its load. A health check can watch it:

```ts
import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";

/** Whether the pool has room, and a warning when it is running low. */
function poolHealth(pool: Pool): { ok: true; warning?: string } | { ok: false; error: string } {
  const status = pool.status();
  if (status.size === status.maxSize && status.available === 0) {
    return { ok: false, error: `all ${status.maxSize} connections are in use` };
  }
  if (status.available < status.size / 4) {
    return { ok: true, warning: `pool running low: ${status.available}/${status.size} idle` };
  }
  return { ok: true };
}

await using small = new Pool(url, { maxSize: 2 });
assert.deepEqual(poolHealth(small), { ok: true });

const busy = [await small.acquire(), await small.acquire()];
assert.deepEqual(poolHealth(small), { ok: false, error: "all 2 connections are in use" });
for (const db of busy) await db.close();
assert.deepEqual(poolHealth(small), { ok: true });
```

## The metrics layer

**Not available in JavaScript yet:** Rust's metrics layer. In Rust,
`InstrumentedPool` wraps a pool and times each wait for a connection, and the
`InstrumentedConnection` and `InstrumentedTransaction` it lends time and
report every statement, transaction, savepoint and COPY run through them, to a
*collector* the program chooses. A collector implements `MetricsCollector`,
seven async hooks: a statement's success and its failure, a connection lent
and one the pool could not lend, and a transaction's begin, commit and
rollback. pgorm ships `NoOpMetrics` and `LoggingMetrics`, which writes each
report as a `tracing` event. A statement's hook is told its operation, its SQL
text, and, with the `metrics-fingerprint` feature, its *fingerprint*:
libpg_query's hash of the statement's shape, the identity
`pg_stat_statements` groups by. JavaScript has no wrapper, no collector and
no fingerprint.

Until it does, a program measures from its own side by timing the calls it
makes. Every statement and every model's terminal returns a Promise, so one
small function times any of them:

```ts
import assert from "node:assert/strict";

const timings = new Map<string, number[]>();

/** Run `work`, recording how long it took under `label`, whether it succeeded or not. */
async function timed<T>(label: string, work: () => Promise<T>): Promise<T> {
  const started = performance.now();
  try {
    return await work();
  } finally {
    const taken = timings.get(label) ?? [];
    taken.push(performance.now() - started);
    timings.set(label, taken);
  }
}

const cakes = await timed("cakes", () => Cake.find().all(pool));
await timed("cakes", () => Cake.findByKey({ id: 1 }).one(pool));
await assert.rejects(timed("missing table", () => pool.query("SELECT * FROM no_such_table")));

assert.equal(cakes.length, 5);
assert.equal(timings.get("cakes")?.length, 2);
assert.equal(timings.get("missing table")?.length, 1);
```

A statement run on the pool includes its wait for a connection in what this
measures, as Rust's statement hooks do not; the pool's `status()` is how to
tell a wait from slow work.

## What it costs

Rust's wrappers cost two clock reads and a boxed future for each hook call,
and an unwrapped pool pays nothing. JavaScript has no layer, so nothing pays
anything, and code timed as above pays for its own timing alone.

## Choosing what to run

- In development, `log_statement = 'all'` on the server.
- In production, `pg_stat_statements` on the server, with the program's
  sessions named by `application_name`, and the pool's `status()` exported as
  a gauge.
- Where every microsecond counts, the server's statistics alone.
