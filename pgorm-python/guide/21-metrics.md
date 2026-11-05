# Metrics

There are two places to watch what a program asks of its database: the
database itself, and the program. PostgreSQL's own statistics see every
statement from every client, with its plan and its cost, and are usually the
best source of truth. pgorm's metrics layer, `pgorm.InstrumentedPool` over
Rust's `pgorm::metric`, sees what the program sees: how long each statement
took from its side, including the wait for a connection, labelled the way the
program wants. The two supplement each other.

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

What is running now is a query away. `pg_stat_activity` holds a row per
session, with the database it is on, its `application_name`, its state and
the statement it last ran. A pool whose URL names the program's
`application_name` ([Connection configuration and
TLS](20-configuration.md)) makes its sessions easy to pick out:

```python
import asyncio
import os
from urllib.parse import parse_qsl, quote, urlencode, urlsplit, urlunsplit

from pgorm import Pool, RawSQL


def named(url: str, application_name: str) -> str:
    parts = urlsplit(url)
    query = dict(parse_qsl(parts.query)) | {"application_name": application_name}
    return urlunsplit(parts._replace(query=urlencode(query, quote_via=quote)))


async def main() -> None:
    sessions = RawSQL(
        "SELECT count(*) AS sessions FROM pg_stat_activity "
        "WHERE datname = current_database() AND application_name = $1",
        ["bakery-web"],
    )
    async with Pool(named(os.environ["DATABASE_URL"], "bakery-web"), max_size=3) as pool:
        # Two connections checked out: two sessions on the server.
        async with pool.connection() as first, pool.connection():
            assert pool.status().size == 2
            row = await first.one(sessions)
            assert row["sessions"] == 2


asyncio.run(main())
```

The pool's own state is a snapshot away. `pool.status()` reports the pool's
`max_size`, how many connections it holds (`size`), how many of those are
idle (`available`), and how many callers are `waiting` for one; a pool that
is often at `available == 0` with callers waiting is too small for its load.
A health check can watch it:

```python
import asyncio
import logging
import os

from pgorm import Pool


def pool_health(pool: Pool) -> str | None:
    """None when the pool has room, or why it has none, warning when it runs low."""
    status = pool.status()
    if status.size == status.max_size and status.available == 0:
        return f"all {status.max_size} connections are in use"
    if status.available < status.size / 4:
        logging.warning("pool running low: %d/%d idle", status.available, status.size)
    return None


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"], max_size=2) as pool:
        assert pool_health(pool) is None

        async with pool.connection(), pool.connection():
            assert pool_health(pool) == "all 2 connections are in use"

        assert pool_health(pool) is None


asyncio.run(main())
```

## The metrics layer

A pool carries no metrics code of its own. Instrumentation is a wrapper you
choose to build, so a program that does not wrap pays nothing:

- `InstrumentedPool(pool, metrics)` wraps a `Pool`, and is one: it runs
  everything a pool runs, over the same connections. Its `acquire` and
  `connection` time the wait for a connection and lend a `Connection` that
  reports what runs on it.
- A transaction or savepoint opened on such a connection, or on the
  instrumented pool, reports its own statements, savepoints and COPYs, and
  its begin and end.

Underneath, these are Rust's `InstrumentedPool`, `InstrumentedConnection` and
`InstrumentedTransaction`, so what is timed, and which hook hears it, is
exactly what this chapter's Rust dialect describes. Python has no
`InstrumentedConnection` or `InstrumentedTransaction` class of its own: a
`Connection` or `Transaction` is instrumented when it came from an
instrumented pool.

Each instrumented pool reports to a *collector*, an object with the seven
hooks of `MetricsCollector`. Two ship with pgorm: `NoOpMetrics`, whose every
hook does nothing, and `LoggingMetrics`, which writes each report to Python's
`logging`, on the `pgorm.metrics` logger unless you give it another, at the
levels Rust's writes its `tracing` events at: `DEBUG` for a successful query,
a connection, a transaction's begin and its commit; `WARNING` for a failed
query and a rollback; `ERROR` for a failure to get a connection.

Because every model query, insert and update, every registered entity's and
every pipeline's terminal runs on the pool, the ORM's work is reported like
hand-written SQL:

```python
import asyncio
import logging
import os

from pgorm import InstrumentedPool, LoggingMetrics, Pool

from bakery import Cake


class Remembered(logging.Handler):
    """Keeps each message the metrics logger writes."""

    def __init__(self) -> None:
        super().__init__()
        self.messages: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


async def main() -> None:
    remembered = Remembered()
    logger = logging.getLogger("pgorm.metrics")
    logger.setLevel(logging.DEBUG)
    logger.addHandler(remembered)

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        instrumented = InstrumentedPool(pool, LoggingMetrics())

        # Reports the acquisition, then the query.
        cakes = await Cake.find().all(instrumented)
        assert len(cakes) == 5
        assert remembered.messages[0].startswith("Connection acquired in ")
        assert remembered.messages[1].startswith("Query query_all succeeded in ")

        # It is the same pool, sharing its connections.
        assert instrumented.status() == pool.status()
        assert instrumented.inner is pool


asyncio.run(main())
```

`inner` is the pool it wraps and `metrics` the collector it reports to. The
two share their connections, their status and their closing: closing either
closes both, and letting the instrumented pool go closes neither. Work run on
`inner`, or on a connection it lends, is not reported, and neither is a
[listener](18-listen-notify.md), which is a connection of its own.

## Writing a collector

A collector is any object with the seven hooks, named as Rust names them. A
hook may be a plain method or an `async` one. A collector that cares only
about some of them subclasses `NoOpMetrics` and overrides those; a hook left
as `NoOpMetrics` wrote it is never even called. An object lacking a hook is a
`TypeError` when the pool is wrapped.

| Hook | Called when |
| --- | --- |
| `record_query_success(query, duration, rows)` | a statement succeeded |
| `record_query_error(query, duration, error)` | a statement failed, or a transaction's `BEGIN`, `COMMIT` or `ROLLBACK` did |
| `record_connection_acquired(duration)` | the pool lent a connection |
| `record_connection_error(duration, error)` | the pool could not |
| `record_transaction_begin(duration)` | a transaction or savepoint opened |
| `record_transaction_commit(duration)` | a transaction or savepoint committed |
| `record_transaction_rollback(duration)` | a transaction or savepoint ended without committing |

`duration` is in seconds, a `float`, as the pool's timeouts are. `rows` is an
`int` or `None`, and `error` is the exception the caller is raised, handed
over rather than raised. As in Rust, `record_transaction_commit` hears only
success, and `record_transaction_rollback` hears every transaction that did
not take effect: one its block left with an exception, and one whose `COMMIT`
failed, which also reports its error through `record_query_error`. A
transaction given up with its connection, by a cancelled task or a closed
pool, is rolled back without any hook hearing of it.

Here is one that counts queries by operation and transactions by outcome:

```python
import asyncio
import os
from collections import Counter

from pgorm import DatabaseError, InstrumentedPool, NoOpMetrics, PgormError, Pool, QueryContext, RawSQL

from bakery import Cake


class Counts(NoOpMetrics):
    def __init__(self) -> None:
        self.counts: Counter[str] = Counter()

    def record_query_success(self, query: QueryContext, duration: float, rows: int | None) -> None:
        self.counts[query.operation] += 1

    def record_query_error(self, query: QueryContext, duration: float, error: PgormError) -> None:
        self.counts[f"{query.operation} failed"] += 1

    def record_connection_acquired(self, duration: float) -> None:
        self.counts["connection"] += 1

    def record_transaction_begin(self, duration: float) -> None:
        self.counts["begin"] += 1

    def record_transaction_commit(self, duration: float) -> None:
        self.counts["commit"] += 1

    def record_transaction_rollback(self, duration: float) -> None:
        self.counts["rollback"] += 1


async def main() -> None:
    counts = Counts()
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        instrumented = InstrumentedPool(pool, counts)
        async with instrumented.connection() as db:
            await Cake.find().all(db)
            await Cake.find_by_key({"id": 1}).one(db)
            try:
                await db.one(RawSQL("SELECT * FROM no_such_table"))
            except DatabaseError:
                pass

            # One transaction commits, with a savepoint inside it; one rolls back.
            async with db.transaction() as tx:
                await Cake.find().count(tx)
                async with tx.transaction() as savepoint:
                    await Cake.find().count(savepoint)
            try:
                async with db.transaction():
                    raise ValueError("changed my mind")
            except ValueError:
                pass

            # A transaction opened in a mode is reported as the others are.
            async with db.transaction(mode="read_only") as tx:
                await Cake.find().count(tx)

    assert counts.counts == {
        "connection": 1,
        "query_all": 5,
        "query_all failed": 1,
        "begin": 4,
        "commit": 3,
        "rollback": 1,
    }


asyncio.run(main())
```

A savepoint is a transaction to the collector, reporting its own begin and its
own commit or rollback. A failed `BEGIN` is reported through
`record_query_error`, not through `record_transaction_begin`.

### Where a hook runs

A hook runs on the pool's event loop, never on the runtime pgorm's work runs
on, and the statement it reports on waits for it, as Rust's awaited hooks
make it wait: a plain hook until it returns, an `async` one until it
completes. So once a statement's `await` returns, its collector has been
told, and one connection's reports arrive in the order they happened. A hook
that blocks blocks the event loop like any other callback, so a collector
that sends its reports over a network does that in an `async` hook.

A hook that raises does not fail the statement it reports on. Its exception
goes to the event loop's exception handler, as an exception raised in a
callback nobody awaits does: by default the `asyncio` logger writes it, and
`loop.set_exception_handler` can collect it instead.

```python
import asyncio
import os

from pgorm import InstrumentedPool, NoOpMetrics, Pool, QueryContext, RawSQL


class Exporter(NoOpMetrics):
    def __init__(self) -> None:
        self.sent: list[str] = []

    async def record_query_success(self, query: QueryContext, duration: float, rows: int | None) -> None:
        await asyncio.sleep(0.01)  # a send to a metrics backend
        self.sent.append(query.operation)

    def record_transaction_commit(self, duration: float) -> None:
        raise RuntimeError("the backend is down")


async def main() -> None:
    reported: list[str] = []
    asyncio.get_running_loop().set_exception_handler(
        lambda loop, context: reported.append(str(context["message"]))
    )
    exporter = Exporter()
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        instrumented = InstrumentedPool(pool, exporter)

        # The async hook has finished by the time the query returns.
        row = await instrumented.one(RawSQL("SELECT 1 AS one"))
        assert row["one"] == 1
        assert exporter.sent == ["query_all"]

        # The failing hook does not undo the commit.
        async with instrumented.transaction() as tx:
            await tx.execute(RawSQL("UPDATE cake SET price = price WHERE id = 1"))
        assert reported == ["pgorm's metrics hook record_transaction_commit raised"]


asyncio.run(main())
```

## What a query hook is told

The two query hooks take a `QueryContext`, pgorm's own copied out for Python
to keep:

- `operation`, the method that ran it: `"execute"`, `"execute_raw"`,
  `"query_one"`, `"query_opt"`, `"query_all"`, `"query_raw"` (a stream's
  start), `"batch_execute"`, `"copy_in"` and `"copy_out"` (a COPY's start, its
  rendered statement as `sql`), or a failed `"begin"`, `"commit"` or
  `"rollback"`. It is the natural label for a metric: a handful of values,
  each with a fixed meaning. Python's queries, a model's and a pipeline's
  included, run as `"query_all"`, and its `execute` as `"execute"`.
- `sql`, the statement's text, or `None` for a transaction verb.
- `fingerprint`, the statement's identity.

A notification is the `execute` of `SELECT pg_notify($1, $2)`, and is
reported as that. A COPY's hook times its start; the rows it then moves go to
the caller and not to the collector.

### Fingerprints

A fingerprint names a statement's shape: two statements that differ only in
their constants share one, and two that differ in shape do not. It is
libpg_query's hash of the parsed statement, the notion of "same query"
`pg_stat_statements` groups by, so a collector keyed on it lines up with the
server's statistics. A `QueryFingerprint` is its 16 hex digits as a `str`, the
same number as its `value`, and it compares and hashes as that number, the
cheaper key for a dictionary of counters. Like Rust's, it follows PostgreSQL
18's rules for query identity: a schema qualifier is not part of a table's
identity, and a table given an alias is identified by the alias.

```python
from pgorm import QueryContext, QueryFingerprint


def fingerprint(sql: str) -> QueryFingerprint:
    found = QueryContext("query_all", sql).fingerprint
    assert found is not None, "the statement parses"
    return found


by_id = fingerprint("SELECT id FROM widget WHERE id = 1")
assert str(by_id) == "394a2f90c244bffe"
assert by_id.value == 0x394A2F90C244BFFE
assert by_id == fingerprint("SELECT id FROM widget WHERE id = 4242")
assert by_id == fingerprint("SELECT id FROM public.widget WHERE id = 1")
assert by_id != fingerprint("SELECT name FROM widget WHERE id = 1")
assert fingerprint("SELECT id FROM widget w WHERE id = 1") == fingerprint(
    "SELECT id FROM gadget w WHERE id = 1"
)

assert QueryContext("begin").fingerprint is None
```

Here Python differs from Rust, where fingerprints sit behind the
`metrics-fingerprint` feature: the Python package is built with it, so
`fingerprint` is there for every statement libpg_query parses. It is `None`
for a transaction verb, which has no statement, and for raw SQL the server
accepts but libpg_query refuses, which runs and is reported as usual with
only its identity missing. A fingerprint is computed when it is read, so a
collector that never reads one pays no parse, and the answer is kept
process-wide by statement text, each text parsed once, up to 1024 texts.

Label a metric by fingerprint only where the backend tolerates the
cardinality: one series per query shape is far more than one per operation,
and statements with no fingerprint need a bucket of their own.

## What it costs

- Not wrapping costs nothing. A pool that is not instrumented runs on pgorm's
  plain types and calls no collector code at all.
- Wrapping costs, on every operation, Rust's two clock reads, and for each
  hook a collector writes, a trip to the event loop and back before the
  statement returns. A hook left as `NoOpMetrics` wrote it costs no trip.
- A `QueryContext` costs its two strings; the parse happens only when
  `fingerprint` is read, once per text.

## Choosing what to run

- In development, `LoggingMetrics` with `log_statement = 'all'` on the
  server.
- In production, `pg_stat_statements` on the server, and a collector of your
  own feeding your metrics backend, labelled by operation, with
  `pool.status()` exported as a gauge.
- To correlate the two, aggregate by fingerprint, the identity
  `pg_stat_statements` groups by.
- To find a program's sessions among the server's, an `application_name`
  in its URL, and `pg_stat_activity`.
- Where every microsecond counts, the server's statistics alone, with the
  pool unwrapped.
