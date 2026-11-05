# Metrics

There are two places to watch what a program asks of its database: the
database itself, and the program. PostgreSQL's own statistics see every
statement from every client, with its plan and its cost, and are usually the
best source of truth. In Rust, pgorm's metrics layer, `pgorm::metric`, sees
what the program sees: how long each statement took from its side, including
the wait for a connection, labelled the way the program wants.

Here Python differs from Rust: the metrics layer is not available in Python
yet. PostgreSQL's statistics, and the pool's own status, serve a Python
program exactly as they serve a Rust one, and the first half of this chapter
applies as it is.

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

pgorm's Rust types carry no metrics code; instrumentation lives in wrappers a
Rust program chooses to build. None of it is available in Python yet:

- `InstrumentedPool`, which times the wait for a connection and lends an
  `InstrumentedConnection`, whose statements, transactions and COPYs, and
  their `InstrumentedTransaction`s', are timed and reported;
- the `MetricsCollector` trait, with its seven hooks for a query that
  succeeded or failed, a connection acquired or refused, and a transaction's
  begin, commit and rollback, and the two collectors that ship with it,
  `NoOpMetrics` and `LoggingMetrics`;
- `QueryContext`, which tells a query hook the operation that ran, its SQL
  and its fingerprint;
- `QueryFingerprint`, libpg_query's hash of a statement's shape, behind the
  `metrics-fingerprint` feature, the same notion of "same query"
  `pg_stat_statements` groups by.

A Python pool therefore reports nothing of its own beyond `status()`, and
nothing a Python program does pays for instrumentation. The Rust layer's
statements are the same statements a Python program sends, so what the
server's statistics say of them holds for both.

## Choosing what to run

- In development, `log_statement = 'all'` on the server.
- In production, `pg_stat_statements` on the server, and `pool.status()`
  exported as a gauge.
- To find a program's sessions among the server's, an `application_name`
  in its URL, and `pg_stat_activity`.
