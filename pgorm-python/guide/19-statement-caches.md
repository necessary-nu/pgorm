# Statement caches and DDL

Every statement pgorm runs is prepared: the server parses and plans it once,
and the connection runs it again by name with new parameters. pgorm keeps the
statements each connection has prepared in a *statement cache* keyed by their
SQL text, so a query the program runs a thousand times is parsed once per
connection, not a thousand times. This is automatic, and most programs never
think about it.

A cached statement outlives the call that prepared it, and that is what this
chapter is about. A prepared statement was planned against the schema as it
was then. When DDL changes the schema under it, PostgreSQL usually notices and
plans the statement again, but not always. This chapter says when it does
not, and what to do.

## What the cache holds

Each connection has its own cache, shared by every transaction and savepoint
on it. Every statement a pool, a connection or a transaction runs looks its
SQL text up there, and is prepared on a miss: a model's queries and writes,
built statements, schema statements and `RawSQL` alike. A COPY is the
exception: it is prepared afresh each time ([COPY](17-copy.md)).

`batch_execute` is the other exception, as Rust's is: it runs a script of
several statements through the simple-query protocol, preparing nothing, so
nothing it sends is cached. Every other statement is prepared, DDL included,
and so a `RawSQL` holds one statement: the server refuses one holding
several, with SQLSTATE `42601`, *cannot insert multiple commands into a
prepared statement* ([Raw SQL](10-raw-sql.md)).

The cache is bounded. One query can be many texts, such as an `IN` list
rendered at every length it is used with, and each cached statement is a live
prepared statement on the server, so an unbounded cache would hold statements
the connection never runs again for as long as it lives. A full cache evicts
an entry to make room. A pool's `statement_cache_size` option sets the bound,
128 statements per connection by default, and `0` turns the cache off, in
which case every call prepares its statement and closes it afterwards:

```python
import asyncio
import os

from pgorm import Connection, Pool, RawSQL


async def prepared(connection: Connection, prefix: str) -> int:
    """How many statements this session holds prepared whose text starts with `prefix`."""
    row = await connection.one(RawSQL(
        "SELECT count(*) AS held FROM pg_prepared_statements WHERE starts_with(statement, $1)",
        [prefix],
    ))
    held: int = row["held"]
    return held


async def main() -> None:
    for size, kept in [(64, 1), (0, 0)]:
        async with Pool(os.environ["DATABASE_URL"], max_size=1, statement_cache_size=size) as pool:
            async with pool.connection() as connection:
                await connection.query(RawSQL("SELECT name FROM bakery"))
                assert await prepared(connection, "SELECT name FROM bakery") == kept


asyncio.run(main())
```

Here Python differs from Rust in its default and its spelling: Rust's pool
holds 256 statements unless its `ManagerConfig`'s `StatementCacheSize` says
otherwise, `Bounded(n)` or `Disabled`; Python's option is a whole number, 128
unless given, with `0` for off.

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
aborted the transaction, so the block raises the `0A000` `DatabaseError`, and
the recovery is yours to run at the transaction's boundary, for example by
running the transaction again ([Transactions](08-transactions.md)):

```python
import asyncio
import os

from pgorm import DatabaseError, Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"], max_size=1) as pool:
        async with pool.connection() as connection:
            before = await connection.query(RawSQL("SELECT * FROM baker"))
            assert len(before[0]) == 4

            await connection.execute(RawSQL("ALTER TABLE baker ADD COLUMN apron text"))

            # Outside a transaction, the refused statement is prepared again and run.
            after = await connection.query(RawSQL("SELECT * FROM baker"))
            assert len(after[0]) == 5

            await connection.execute(RawSQL("ALTER TABLE baker DROP COLUMN apron"))

            # Inside one, the refusal is the transaction's error.
            try:
                async with connection.transaction() as tx:
                    await tx.query(RawSQL("SELECT * FROM baker"))
            except DatabaseError as error:
                assert error.sqlstate == "0A000"
            else:
                raise AssertionError("the cached plan's result type changed")

            # It was dropped from the cache, so running the transaction again succeeds.
            async with connection.transaction() as tx:
                rows = await tx.query(RawSQL("SELECT * FROM baker"))
            assert len(rows[0]) == 4


asyncio.run(main())
```

Selecting named columns rather than `*`, as every query a model builds does,
keeps a statement's result from changing under it.

## When it does not: clearing the caches

Some DDL changes what a plan was built from without the server planning the
statement again. A domain's default is one: an `INSERT` prepared before
`ALTER DOMAIN .. SET DEFAULT` can go on writing the old default. For such
changes, `pool.clear_statement_caches()` drops every cached statement of
every connection in the pool, closing each on the server, so each statement
is prepared again the next time it runs.

The call does not clear anything in place, because a connection that is
checked out may be preparing a statement at that very moment, against the
schema the DDL just replaced. It marks every connection's cache as out of
date instead. An idle connection is cleared before the pool next lends it; a
connection checked out at the time keeps its statements until it is
returned, and is cleared before it is lent again. So code holding a
connection across the DDL returns it and takes another to see the change.
The call is an ordinary method, not a coroutine: it sends nothing, waits for
nothing, and cannot fail, even on a closed pool.

```python
import asyncio
import os

from pgorm import Pool, RawSQL

INSERT = RawSQL("INSERT INTO box (contents) VALUES ($1) RETURNING ribbon", ["truffles"])
PREPARED = RawSQL(
    "SELECT count(*) AS held FROM pg_prepared_statements "
    "WHERE starts_with(statement, 'INSERT INTO box')"
)


async def main() -> None:
    # One connection, so every checkout is the same session.
    async with Pool(os.environ["DATABASE_URL"], max_size=1) as pool:
        async with pool.connection() as connection:
            await connection.execute(RawSQL("CREATE DOMAIN ribbon AS text DEFAULT 'red'"))
            await connection.execute(RawSQL(
                "CREATE TABLE box (id serial PRIMARY KEY, contents text, ribbon ribbon)"
            ))
            assert (await connection.one(INSERT))["ribbon"] == "red"

            await connection.execute(RawSQL("ALTER DOMAIN ribbon SET DEFAULT 'gold'"))
            # The cached insert still writes the old default.
            assert (await connection.one(INSERT))["ribbon"] == "red"
            pool.clear_statement_caches()

            # The connection is checked out, so it keeps its statement until returned.
            assert (await connection.one(PREPARED))["held"] == 1

        # Lent again, it was cleared first, and the insert is prepared afresh.
        async with pool.connection() as connection:
            assert (await connection.one(PREPARED))["held"] == 0
            assert (await connection.one(INSERT))["ribbon"] == "gold"


asyncio.run(main())
```

In Rust, pgorm-migration's runner calls the same clear on the pool it was
given once a batch that applied a migration commits, so a program that
migrates through its own pool needs nothing more. The runner is not available
in Python ([Migrations](22-migrations.md)): a Python program that changes its
schema with its own DDL calls `clear_statement_caches` itself once that DDL
has committed.

## Clearing other processes' caches

`clear_statement_caches` reaches the pool it is called on, in the process
that holds it. An application usually runs as several processes, and the one
that migrates is often none of them. For those, a pool can listen for a
clear: its `statement_cache_channel` option names a channel, and a pool given
one keeps a connection of its own listening there ([LISTEN and
NOTIFY](18-listen-notify.md)). Every notification on the channel clears the
pool's caches as `clear_statement_caches` does, whatever its payload.

It is off by default, because it costs each pool one connection, opened with
the pool's first. That connection is not lent, counts against no size limit,
and is reopened if it drops, with a wait of 100 ms that doubles to 10 s
between attempts; and since a notification sent while it was down reached
nobody, the caches are cleared each time its `LISTEN` takes effect, the first
time included. Closing the pool closes it. Nothing travels on it between
notifications, so a network middlebox that drops idle connections drops it
too; TCP keepalives, set with the URL's `keepalives_idle` setting
([Connection configuration and TLS](20-configuration.md)), keep it open.

`pgorm.STATEMENT_CACHE_CHANNEL`, `pgorm_statement_caches`, is the channel
pgorm-migration's runner notifies, inside the transaction of a batch that
applied a migration, so the notification is delivered exactly when the
migration commits. A process that changes the schema some other way, a
Python program running its own DDL among them, sends one itself with
`notify`, from the DDL's own transaction so that it arrives as the change
commits. A notification reaches only sessions on the
database it was sent in, so one channel name serves every database on a
server.

Here a second pool, standing for another process, writes a domain's default
and hears the clear the first sends after changing it:

```python
import asyncio
import os

import pgorm
from pgorm import Pool, RawSQL

INSERT = RawSQL("INSERT INTO box (contents) VALUES ($1) RETURNING ribbon", ["fudge"])


async def main() -> None:
    url = os.environ["DATABASE_URL"]
    # The process that migrates.
    async with Pool(url) as migrator:
        await migrator.execute(RawSQL("CREATE DOMAIN ribbon AS text DEFAULT 'red'"))
        await migrator.execute(RawSQL(
            "CREATE TABLE box (id serial PRIMARY KEY, contents text, ribbon ribbon)"
        ))

        # Another process's pool, listening for clears.
        async with Pool(url, max_size=1, statement_cache_channel=pgorm.STATEMENT_CACHE_CHANNEL) as shop:
            assert (await shop.one(INSERT))["ribbon"] == "red"

            await migrator.execute(RawSQL("ALTER DOMAIN ribbon SET DEFAULT 'gold'"))
            await migrator.notify(pgorm.STATEMENT_CACHE_CHANNEL, "")

            # The clear arrives as a notification, so the shop sees it a moment later.
            async with asyncio.timeout(10):
                while (await shop.one(INSERT))["ribbon"] != "gold":
                    await asyncio.sleep(0.05)


asyncio.run(main())
```

The clear is the same deferred one: a connection the other process holds
checked out for its whole life keeps its statements until it returns it. An
empty channel name, or one longer than 63 bytes, which no notification could
reach, raises `ConstructionError` when the pool is built. Here Python differs
from Rust in spelling: the channel is a keyword argument of `Pool`, where Rust
sets `ManagerConfig::statement_cache_channel`. Rust's metrics layer does not
see these clears; Python has no metrics layer yet ([Metrics](21-metrics.md)).
