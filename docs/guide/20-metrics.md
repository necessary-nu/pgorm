# Metrics

There are two places to watch what a program asks of its database: the
database itself, and the program. PostgreSQL's own statistics see every
statement from every client, with its plan and its cost, and are usually the
best source of truth. pgorm's metrics layer, [`pgorm::metric`](crate::metric),
sees what the program sees: how long each statement took from its side,
including the wait for a connection, labelled the way the program wants.
The two supplement each other.

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

The pool's own state is a snapshot away.
[`DatabasePool::status`](crate::DatabasePool::status) reports the pool's
`max_size`, how many connections it holds (`size`), how many of those are
idle (`available`), and how many callers are `waiting` for one; a pool that
is often at `available == 0` with callers waiting is too small for its load.
A health check can watch it:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("metrics_status").await;
# let config = scratch.config();
use pgorm::DatabasePool;

/// Whether the pool has room, and a warning when it is running low.
fn pool_health(pool: &DatabasePool) -> Result<(), String> {
    let status = pool.status();
    if status.size == status.max_size && status.available == 0 {
        return Err(format!("all {} connections are in use", status.max_size));
    }
    if status.available < status.size / 4 {
        tracing::warn!("pool running low: {}/{} idle", status.available, status.size);
    }
    Ok(())
}

let pool = pgorm::connect_with_builder(config, |builder| builder.max_size(2))?;
assert_eq!(pool_health(&pool), Ok(()));

let busy = (pool.get().await?, pool.get().await?);
assert_eq!(pool_health(&pool), Err("all 2 connections are in use".to_owned()));
drop(busy);
assert_eq!(pool_health(&pool), Ok(()));
# pool.close();
# scratch.finish().await;
# Ok(())
# }
```

## The metrics layer

pgorm's own types carry no metrics code. Instrumentation lives in wrappers you
choose to build, so a program that does not wrap pays nothing:

- [`InstrumentedPool`](crate::metric::InstrumentedPool) wraps a
  [`DatabasePool`](crate::DatabasePool). Its `get` times the wait for a
  connection and returns an
  [`InstrumentedConnection`](crate::metric::InstrumentedConnection).
- An `InstrumentedConnection` implements
  [`ConnectionTrait`](crate::ConnectionTrait),
  [`TransactionTrait`](crate::TransactionTrait) and
  [`CopyTrait`](crate::CopyTrait). It runs every call on the connection it
  wraps, times it, reports it, and returns the result unchanged.
- Its transactions are
  [`InstrumentedTransaction`](crate::metric::InstrumentedTransaction)s, so
  the statements, savepoints and COPYs inside a transaction are reported too.

Each wrapper reports to a *collector*, a value implementing
[`MetricsCollector`](crate::metric::MetricsCollector). Two ship with pgorm:
[`NoOpMetrics`](crate::metric::NoOpMetrics), whose every hook does nothing,
and [`LoggingMetrics`](crate::metric::LoggingMetrics), which writes each
report as a `tracing` event: `debug` for a successful query, a connection, a
transaction's begin and its commit; `warn` for a failed query and a rollback;
`error` for a failure to get a connection.

Because every entity query, insert and update runs through
`ConnectionTrait`, the ORM's work is reported like hand-written SQL:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("metrics_wrap").await;
# let pool = scratch.pool.clone();
use pgorm::entity::prelude::*;
use pgorm::metric::{InstrumentedPool, LoggingMetrics};

let instrumented = InstrumentedPool::new(pool, LoggingMetrics);

// Reports the acquisition, then the query, as tracing events.
let db = instrumented.get().await?;
let cakes = cake::Entity::find().all(&db).await?;
assert_eq!(cakes.len(), 5);

// The pool's own methods are forwarded.
assert_eq!(instrumented.status().size, 1);
assert_eq!(instrumented.tag(), instrumented.inner().tag());
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

`InstrumentedPool` also forwards
[`clear_statement_caches`](crate::metric::InstrumentedPool::clear_statement_caches),
and [`inner`](crate::metric::InstrumentedPool::inner) and
[`metrics`](crate::metric::InstrumentedPool::metrics) reach the pool and the
collector it wraps. A statement run through an `InstrumentedTransaction`'s
[`inner`](crate::metric::InstrumentedTransaction::inner) transaction, or an
`InstrumentedConnection`'s
[`inner`](crate::metric::InstrumentedConnection::inner) connection, bypasses
the collector.

## Writing a collector

A collector is any type implementing
[`MetricsCollector`](crate::metric::MetricsCollector): `Clone`, `Send`,
`Sync` and `'static`, with seven async hooks and no default bodies, so a
collector that cares only about queries writes the rest empty.

| Hook | Called when |
| --- | --- |
| `record_query_success(query, duration, rows)` | a statement succeeded |
| `record_query_error(query, duration, error)` | a statement failed, or a transaction's `BEGIN`, `COMMIT` or `ROLLBACK` did |
| `record_connection_acquired(duration)` | the pool lent a connection |
| `record_connection_error(duration, error)` | the pool could not |
| `record_transaction_begin(duration)` | a transaction or savepoint opened |
| `record_transaction_commit(duration)` | a transaction or savepoint committed |
| `record_transaction_rollback(duration)` | a transaction or savepoint ended without committing |

Two hooks mean slightly less than their names suggest, and it decides what a
counter built on them counts:

- `record_transaction_commit` is called on success only.
- `record_transaction_rollback` is called when a transaction ended without
  taking effect: its closure returned `Err` and it was rolled back, or its
  `COMMIT` failed, since PostgreSQL aborts a transaction whose commit fails. A
  failed commit also reports its error through `record_query_error`, as a
  failed `ROLLBACK` does. A counter on this hook counts transactions that did
  not take effect, which is the number worth having.

A transaction whose closure's future is dropped before it finishes, by a
timeout or a cancelled task, still rolls back, but no hook hears of it: every
hook is async, and the drop that rolls it back is not. The transaction logs a
`tracing` warning, *Transaction dropped without committing!*, instead.

The pool wrapper clones the collector into every connection it lends, so a
collector should be cheap to clone: a handle to shared counters, not the
counters themselves. Here is one that counts queries by operation and
transactions by outcome:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("metrics_collector").await;
# let pool = scratch.pool.clone();
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pgorm::entity::prelude::*;
use pgorm::metric::{InstrumentedPool, MetricsCollector, QueryContext};

#[derive(Clone, Default)]
struct Counts(Arc<Mutex<BTreeMap<String, u64>>>);

impl Counts {
    fn bump(&self, key: &str) {
        let mut counts = self.0.lock().expect("no hook panics");
        *counts.entry(key.to_owned()).or_default() += 1;
    }

    fn get(&self, key: &str) -> u64 {
        self.0.lock().expect("no hook panics").get(key).copied().unwrap_or(0)
    }
}

#[async_trait::async_trait]
impl MetricsCollector for Counts {
    async fn record_query_success(&self, query: QueryContext<'_>, _: Duration, _: Option<u64>) {
        self.bump(query.operation());
    }
    async fn record_query_error(&self, query: QueryContext<'_>, _: Duration, _: &Error) {
        self.bump(&format!("{} failed", query.operation()));
    }
    async fn record_connection_acquired(&self, _: Duration) {
        self.bump("connection");
    }
    async fn record_connection_error(&self, _: Duration, _: &Error) {}
    async fn record_transaction_begin(&self, _: Duration) {
        self.bump("begin");
    }
    async fn record_transaction_commit(&self, _: Duration) {
        self.bump("commit");
    }
    async fn record_transaction_rollback(&self, _: Duration) {
        self.bump("rollback");
    }
}

let counts = Counts::default();
let instrumented = InstrumentedPool::new(pool, counts.clone());
let mut db = instrumented.get().await?;

cake::Entity::find().all(&db).await?;
cake::Entity::find_by_id(1).one(&db).await?;
assert!(db.query_one("SELECT * FROM no_such_table", &[]).await.is_err());

// One transaction commits, with a savepoint inside it; one rolls back.
db.transaction(async |txn| {
    cake::Entity::find().count(&txn).await?;
    txn.transaction(async |savepoint| cake::Entity::find().count(&savepoint).await)
        .await
        .map_err(|error| Error::Custom(error.to_string()))
})
.await?;
let refused = db
    .transaction(async |_txn| Err::<(), _>(Error::Custom("changed my mind".to_owned())))
    .await;
assert!(refused.is_err());

assert_eq!(counts.get("connection"), 1);
assert_eq!(counts.get("query_all"), 1);
assert_eq!(counts.get("query_opt"), 3);
assert_eq!(counts.get("query_one failed"), 1);
assert_eq!(counts.get("begin"), 3);
assert_eq!(counts.get("commit"), 2);
assert_eq!(counts.get("rollback"), 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A savepoint is a transaction to the collector, reporting its own begin and its
own commit or rollback. The closure is handed a `&mut InstrumentedTransaction`,
which a query takes as `&txn`, as it takes a plain transaction's handle, and
whose statements are reported like the connection's. A failed `BEGIN` is
reported through `record_query_error`, not through
`record_transaction_begin`.

To feed a metrics backend such as Prometheus, the hooks increment its
counters and observe its histograms; they are ordinary async functions. The
collector is a type parameter of the wrappers rather than a trait object, so
the calls are statically dispatched.

## What a query hook is told

The two query hooks take a [`QueryContext`](crate::metric::QueryContext), a
view of the statement valid for the length of the call; a collector that
keeps anything copies it out:

- [`operation`](crate::metric::QueryContext::operation), the method that ran
  it. It is the natural label for a metric: a handful of values, each with a
  fixed meaning.
- [`sql`](crate::metric::QueryContext::sql), the statement's text, or `None`
  for a transaction's `"begin"`, `"commit"` or `"rollback"`.
- [`fingerprint`](crate::metric::QueryContext::fingerprint), the statement's
  identity.

| Operation | What ran | `rows` on success |
| --- | --- | --- |
| `"execute"`, `"execute_raw"` | a statement through `execute` or `execute_raw` | the rows it changed |
| `"query_one"` | `query_one` | `Some(1)` |
| `"query_opt"` | `query_opt` | `Some(1)` or `Some(0)` |
| `"query_all"` | `query_all` | the number of rows |
| `"query_raw"` | the start of a row stream | `None` |
| `"batch_execute"` | a script, its whole text as `sql` | `None` |
| `"copy_in"`, `"copy_out"` | the start of a [COPY](crate::guide::copy), its rendered statement as `sql` | `None` |
| `"begin"`, `"commit"`, `"rollback"` | only on failure, through `record_query_error` | |

A COPY's hook times its start, as `query_raw`'s times the start of a stream.
The row count its sink's `finish` returns, and a row the server refuses there,
go to the caller and not to the collector. [`notify`](crate::notify) is
`ConnectionTrait::execute` of `SELECT pg_notify($1, $2)`, and is reported as
that. A [listener](crate::guide::listen_notify) is not instrumented at all: it is a
connection of its own, not one from the pool, and what it sends and receives
are not statements a caller wrote.

### Fingerprints

A fingerprint names a statement's shape: two statements that differ only in
their constants share one, and two that differ in shape do not. It is
libpg_query's hash of the parsed statement, the same notion of "same query"
`pg_stat_statements` groups by, so a collector keyed on it lines up with the
server's statistics. [`QueryFingerprint`](crate::metric::QueryFingerprint)
writes itself as 16 hex digits, and
[`value`](crate::metric::QueryFingerprint::value) gives the same number as a
`u64`, the cheaper key for a map of counters.

Fingerprints follow PostgreSQL 18's rules for query identity: a schema
qualifier is not part of a table's identity, and a table given an alias is
identified by the alias.

```rust
use pgorm::metric::QueryContext;

let print = |sql| {
    QueryContext::new("query_all", Some(sql))
        .fingerprint()
        .expect("the statement parses")
};

let by_id = print("SELECT id FROM widget WHERE id = 1");
assert_eq!(by_id.to_string(), "394a2f90c244bffe");
assert_eq!(by_id, print("SELECT id FROM widget WHERE id = 4242"));
assert_eq!(by_id, print("SELECT id FROM public.widget WHERE id = 1"));
assert_ne!(by_id, print("SELECT name FROM widget WHERE id = 1"));

assert_eq!(
    print("SELECT id FROM widget w WHERE id = 1"),
    print("SELECT id FROM gadget w WHERE id = 1"),
);
```

Fingerprints are off by default, behind the `metrics-fingerprint` feature:

```toml
[dependencies]
pgorm = { git = "https://github.com/necessary-nu/pgorm", features = ["metrics-fingerprint"] }
```

Without the feature no statement is parsed for its identity and `fingerprint`
answers `None`. The types and hooks are the same either way, so a collector
compiles against both builds. With it, `fingerprint` still answers `None`, and
never fails the query, in two more cases: a transaction verb has no statement
to parse, and libpg_query may refuse raw SQL that the server accepts. Such a
statement runs and is reported as usual; only its identity is missing.

A fingerprint is computed when a collector asks for it, so a collector that
never asks pays no parse. Answers are kept process-wide by statement text, so
each text is parsed once, up to 1024 texts; past that, new texts are parsed
each time they are asked about rather than kept. A program's query shapes are
a fixed set well under that. Text built per call, such as an `IN` list sized
by its input, is what the cap keeps from piling up.

Label a metric by fingerprint only where the backend tolerates the
cardinality: one series per query shape is far more than one per operation,
and statements with no fingerprint need a bucket of their own.

## What it costs

- Not wrapping costs nothing. `DatabasePool`, `DatabaseConnection` and
  `DatabaseTransaction` contain no metrics code.
- Wrapping costs, on every operation and whatever the collector: two clock
  reads, and a boxed future for each hook call, since the hooks are
  `async_trait` methods. `NoOpMetrics` saves the reporting, not the timing.
- Building a `QueryContext` costs nothing beyond two borrowed fields; the
  parse happens only when `fingerprint` is asked for, once per text.
- Turning `metrics-fingerprint` off costs nothing either. libpg_query is
  linked into every build anyway, since pgorm parses with it elsewhere.

## Choosing what to run

- In development, `LoggingMetrics` with `log_statement = 'all'` on the server.
- In production, `pg_stat_statements` on the server, and a collector of your
  own feeding your metrics backend, labelled by operation.
- To correlate the two, turn `metrics-fingerprint` on and aggregate by
  fingerprint, the identity `pg_stat_statements` groups by.
- Where every microsecond counts, the server's statistics alone, with the
  pool unwrapped.
