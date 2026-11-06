# Metrics

There are two places to watch what a program asks of its database: the
database itself, and the program. PostgreSQL's own statistics see every
statement from every client, with its plan and its cost, and are usually the
best source of truth. pgorm's metrics layer, `InstrumentedPool` over Rust's
`pgorm::metric`, sees what the program sees: how long each statement took
from its side, including the wait for a connection, labelled the way the
program wants. The two supplement each other. This chapter is the JavaScript
dialect of the [Rust guide's chapter](../../docs/guide/20-metrics.md).

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

A pool carries no metrics code of its own. Instrumentation is a wrapper you
choose to build, so a program that does not wrap pays nothing:

- `new InstrumentedPool(pool, metrics)` wraps a `Pool`, and is one: it runs
  everything a pool runs, over the same connections. Its `acquire` and
  `connection` time the wait for a connection and lend a `Connection` that
  reports what runs on it.
- A transaction or savepoint opened on such a connection, or on the
  instrumented pool, reports its own statements, savepoints and COPYs, and
  its begin and end.

Underneath, these are Rust's `InstrumentedPool`, `InstrumentedConnection` and
`InstrumentedTransaction`, so what is timed, and which hook hears it, is
exactly what the Rust chapter describes. JavaScript has no
`InstrumentedConnection` or `InstrumentedTransaction` class of its own: a
`Connection` or `Transaction` is instrumented when it came from an
instrumented pool.

Each instrumented pool reports to a *collector*, an object with the seven
hooks of the `MetricsCollector` interface. Two ship with pgorm: `NoOpMetrics`,
whose every hook does nothing, and `LoggingMetrics`, which writes each report
to a logger, `console` unless you give it another with `debug`, `warn` and
`error`, at the levels Rust's writes its `tracing` events at: `debug` for a
successful query, a connection, a transaction's begin and its commit; `warn`
for a failed query and a rollback; `error` for a failure to get a connection.

Because every model query, insert and update, every registered entity's and
every pipeline's terminal runs on the pool, the ORM's work is reported like
hand-written SQL:

```ts
import assert from "node:assert/strict";
import { InstrumentedPool, LoggingMetrics } from "@necessary/pgorm";

const lines: string[] = [];
const logger = {
  debug: (message: string) => lines.push(message),
  warn: (message: string) => lines.push(message),
  error: (message: string) => lines.push(message),
};
const instrumented = new InstrumentedPool(pool, new LoggingMetrics(logger));

// Reports the acquisition, then the query.
const cakes = await Cake.find().all(instrumented);
assert.equal(cakes.length, 5);
assert.match(lines[0] ?? "", /^Connection acquired in /);
assert.match(lines[1] ?? "", /^Query queryAll succeeded in /);

// It is the same pool, sharing its connections.
assert.deepEqual(instrumented.status(), pool.status());
assert.equal(instrumented.inner, pool);
```

`inner` is the pool it wraps and `metrics` the collector it reports to. The
two share their connections, their status and their closing: closing either
closes both. Work run on `inner`, or on a connection it lends, is not
reported, and neither is a [listener](18-listen-notify.md), which is a
connection of its own.

## Writing a collector

A collector is any object with the seven hooks, Rust's names in camelCase. A
hook may return nothing or a promise. A collector that cares only about some
of them extends `NoOpMetrics` and overrides those; a hook left as
`NoOpMetrics` wrote it is never even called. An object lacking a hook is a
`TypeError` when the pool is wrapped.

| Hook | Called when |
| --- | --- |
| `recordQuerySuccess(query, duration, rows)` | a statement succeeded |
| `recordQueryError(query, duration, error)` | a statement failed, or a transaction's `BEGIN`, `COMMIT` or `ROLLBACK` did |
| `recordConnectionAcquired(duration)` | the pool lent a connection |
| `recordConnectionError(duration, error)` | the pool could not |
| `recordTransactionBegin(duration)` | a transaction or savepoint opened |
| `recordTransactionCommit(duration)` | a transaction or savepoint committed |
| `recordTransactionRollback(duration)` | a transaction or savepoint ended without committing |

`duration` is in milliseconds, as the pool's timeouts are. `rows` is a number
or `null`, and `error` is the error the caller is rejected with, handed over
rather than thrown. As in Rust, `recordTransactionCommit` hears only success,
and `recordTransactionRollback` hears every transaction that did not take
effect: one whose callback threw, and one whose `COMMIT` failed, which also
reports its error through `recordQueryError`. A transaction given up with its
connection, by an abort or a closed pool, is rolled back without any hook
hearing of it.

Here is one that counts queries by operation and transactions by outcome:

```ts
import assert from "node:assert/strict";
import { DatabaseError, InstrumentedPool, NoOpMetrics, type QueryContext } from "@necessary/pgorm";

class Counts extends NoOpMetrics {
  readonly counts = new Map<string, number>();

  bump(key: string) {
    this.counts.set(key, (this.counts.get(key) ?? 0) + 1);
  }

  override recordQuerySuccess(query: QueryContext) {
    this.bump(query.operation);
  }
  override recordQueryError(query: QueryContext) {
    this.bump(`${query.operation} failed`);
  }
  override recordConnectionAcquired() {
    this.bump("connection");
  }
  override recordTransactionBegin() {
    this.bump("begin");
  }
  override recordTransactionCommit() {
    this.bump("commit");
  }
  override recordTransactionRollback() {
    this.bump("rollback");
  }
}

const counts = new Counts();
const instrumented = new InstrumentedPool(pool, counts);
await instrumented.connection(async (db) => {
  await Cake.find().all(db);
  await Cake.findByKey({ id: 1 }).one(db);
  await assert.rejects(db.one("SELECT * FROM no_such_table"), DatabaseError);

  // One transaction commits, with a savepoint inside it; one rolls back.
  await db.transaction(async (tx) => {
    await Cake.find().count(tx);
    await tx.transaction((savepoint) => Cake.find().count(savepoint));
  });
  await assert.rejects(db.transaction(async () => {
    throw new Error("changed my mind");
  }));

  // A transaction opened in a mode is reported as the others are.
  await db.transaction((tx) => Cake.find().count(tx), { mode: "readOnly" });
});

assert.deepEqual(Object.fromEntries(counts.counts), {
  connection: 1,
  queryAll: 5,
  "queryAll failed": 1,
  begin: 4,
  commit: 3,
  rollback: 1,
});
```

A savepoint is a transaction to the collector, reporting its own begin and its
own commit or rollback. A failed `BEGIN` is reported through
`recordQueryError`, not through `recordTransactionBegin`.

### Where a hook runs

A hook runs on the JavaScript thread, never on the runtime pgorm's work runs
on, and the statement it reports on waits for it, as Rust's awaited hooks
make it wait: a hook until it returns, and a promise it returns until that
settles. So once a statement's promise settles, its collector has been told,
and one connection's reports arrive in the order they happened. A hook that
blocks blocks the event loop like any other callback, so a collector that
sends its reports over a network returns that send's promise.

A hook that throws, or whose promise rejects, does not fail the statement it
reports on. Its error becomes a process warning, a `MetricsWarning` whose
`cause` is the error: Node.js and Deno print it to standard error, and a
program that wants it listens for `process`'s `warning` event:

```ts
import assert from "node:assert/strict";
import process from "node:process";
import { InstrumentedPool, NoOpMetrics, type QueryContext } from "@necessary/pgorm";

class Exporter extends NoOpMetrics {
  readonly sent: string[] = [];

  override async recordQuerySuccess(query: QueryContext) {
    await new Promise((resolve) => setTimeout(resolve, 10)); // a send to a metrics backend
    this.sent.push(query.operation);
  }

  override recordTransactionCommit(): void {
    throw new Error("the backend is down");
  }
}

const warnings: Error[] = [];
const listener = (warning: Error) => warnings.push(warning);
process.on("warning", listener);

const exporter = new Exporter();
const instrumented = new InstrumentedPool(pool, exporter);

// The promise has settled by the time the query does.
assert.deepEqual(await instrumented.one("SELECT 1 AS one"), { one: 1 });
assert.deepEqual(exporter.sent, ["queryAll"]);

// The failing hook does not undo the commit.
await instrumented.transaction((tx) => tx.execute("UPDATE cake SET price = price WHERE id = 1"));
await new Promise((resolve) => setTimeout(resolve, 10));
process.off("warning", listener);
assert.equal(warnings.length, 1);
assert.equal(warnings[0]?.name, "MetricsWarning");
assert.equal((warnings[0]?.cause as Error).message, "the backend is down");
```

## What a query hook is told

The two query hooks take a `QueryContext`:

- `operation`, the method that ran it, Rust's name for it spelled as
  JavaScript spells a closed choice: `"execute"`, `"executeRaw"`,
  `"queryOne"`, `"queryOpt"`, `"queryAll"`, `"queryRaw"` (a stream's start),
  `"batchExecute"`, `"copyIn"` and `"copyOut"` (a COPY's start, its rendered
  statement as `sql`), or a failed `"begin"`, `"commit"` or `"rollback"`. It is
  the natural label for a metric: a handful of values, each with a fixed
  meaning. JavaScript's queries, a model's and a pipeline's included, run as
  `"queryAll"`, and its `execute` as `"execute"`.
- `sql`, the statement's text, or `null` for a transaction verb.
- `fingerprint`, the statement's identity.

A notification is the `execute` of `SELECT pg_notify($1, $2)`, and is
reported as that. A COPY's hook times its start; the rows it then moves go to
the caller and not to the collector.

### Fingerprints

A fingerprint names a statement's shape: two statements that differ only in
their constants share one, and two that differ in shape do not. It is
libpg_query's hash of the parsed statement, the notion of "same query"
`pg_stat_statements` groups by, so a collector keyed on it lines up with the
server's statistics. A `QueryFingerprint`'s string is its 16 hex digits and
its `value` the same number as a `bigint`; either is a key for a `Map` of
counters, and `equals` compares two. Like Rust's, it follows PostgreSQL 18's
rules for query identity: a schema qualifier is not part of a table's
identity, and a table given an alias is identified by the alias.

```ts
import assert from "node:assert/strict";
import { QueryContext, type QueryFingerprint } from "@necessary/pgorm";

function fingerprint(sql: string): QueryFingerprint {
  const found = new QueryContext("queryAll", sql).fingerprint;
  assert.ok(found, "the statement parses");
  return found;
}

const byId = fingerprint("SELECT id FROM widget WHERE id = 1");
assert.equal(String(byId), "394a2f90c244bffe");
assert.equal(byId.value, 0x394a2f90c244bffen);
assert.ok(byId.equals(fingerprint("SELECT id FROM widget WHERE id = 4242")));
assert.ok(byId.equals(fingerprint("SELECT id FROM public.widget WHERE id = 1")));
assert.ok(!byId.equals(fingerprint("SELECT name FROM widget WHERE id = 1")));
assert.ok(fingerprint("SELECT id FROM widget w WHERE id = 1").equals(fingerprint("SELECT id FROM gadget w WHERE id = 1")));

assert.equal(new QueryContext("begin").fingerprint, null);
```

Here JavaScript differs from Rust, where fingerprints sit behind the
`metrics-fingerprint` feature: the addon is built with it, so `fingerprint`
is there for every statement libpg_query parses. It is `null` for a
transaction verb, which has no statement, and for raw SQL the server accepts
but libpg_query refuses, which runs and is reported as usual with only its
identity missing. A fingerprint is computed when it is read, so a collector
that never reads one pays no parse, and the answer is kept process-wide by
statement text, each text parsed once, up to 1024 texts.

Label a metric by fingerprint only where the backend tolerates the
cardinality: one series per query shape is far more than one per operation,
and statements with no fingerprint need a bucket of their own.

## What it costs

- Not wrapping costs nothing. A pool that is not instrumented runs on pgorm's
  plain types and calls no collector code at all.
- Wrapping costs, on every operation, Rust's two clock reads, and for each
  hook a collector writes, a trip to the JavaScript thread and back before the
  statement settles. A hook left as `NoOpMetrics` wrote it costs no trip.
- A `QueryContext` costs its two strings; the parse happens only when
  `fingerprint` is read, once per text.

## Choosing what to run

- In development, `LoggingMetrics` with `log_statement = 'all'` on the
  server.
- In production, `pg_stat_statements` on the server, with the program's
  sessions named by `application_name`, and a collector of your own feeding
  your metrics backend, labelled by operation, with the pool's `status()`
  exported as a gauge.
- To correlate the two, aggregate by fingerprint, the identity
  `pg_stat_statements` groups by.
- Where every microsecond counts, the server's statistics alone, with the
  pool unwrapped.
