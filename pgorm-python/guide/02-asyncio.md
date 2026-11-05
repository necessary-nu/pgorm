# Python and asyncio

The Rust library runs on the tokio runtime. The Python package runs on
asyncio, and this chapter says what that asks of a program: where pgorm's
objects may be made and used, how statements run side by side, and what
cancelling one does. It has no Rust counterpart. Each chapter that follows
adds the rules of its own feature, such as what a cancelled transaction or
`COPY` does.

## One event loop

Every database operation is a coroutine: awaiting it hands the statement to
pgorm's Rust connections, which wait for the server on a Tokio runtime of
their own, in threads beside Python's. The event loop is free to run other
tasks while a statement is in flight, and no thread of the program blocks on
the network.

A pool belongs to the asyncio event loop that was running when it was made,
and so do the connections, transactions and streams it hands out. Make it
inside a coroutine, and use it only from that loop. Outside a running loop
there is nothing for it to belong to:

```python
import pgorm

try:
    pgorm.Pool("postgres://postgres@localhost/bakery")
except pgorm.LifecycleError as error:
    assert str(error) == "pgorm resources require a running asyncio event loop"
else:
    raise AssertionError("a pool needs a running event loop")
```

A pool used from another loop raises `LifecycleError` too, so a pool cannot
move to a second thread's loop or outlive the `asyncio.run` that made it.
A program with several loops makes a pool in each. Builders are different:
a query, a model or a schema statement is a plain value, made anywhere,
shared freely, and run on any loop's pool.

There is no blocking interface. A program that is not async runs its database
work inside `asyncio.run`, as every example in this guide does.

## Statements side by side

A pool runs each statement on a connection it lends for that statement alone,
so statements started together run together, each on its own connection, up
to the pool's `max_size`:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Baker, Cake, Customer


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"], max_size=3) as pool:
        cakes, bakers, customers = await asyncio.gather(
            Cake.find().count(pool),
            Baker.find().count(pool),
            Customer.find().count(pool),
        )
        assert (cakes, bakers, customers) == (5, 3, 2)


asyncio.run(main())
```

A statement that finds every connection in use waits for one to come back,
for at most the pool's `acquire_timeout`, and then raises `pgorm.TimeoutError`
([Connection configuration and TLS](20-configuration.md) covers the
pool's sizes and timeouts).

A connection checked out with `pool.connection()` runs one statement at a
time. It does not queue a second: a statement started while another is in
flight raises `LifecycleError`, and the one already running carries on
undisturbed. Of two statements started together, which one comes second is
not defined:

```python
import asyncio
import os

import pgorm
from pgorm import Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with pool.connection() as connection:
            results = await asyncio.gather(
                connection.one(RawSQL("SELECT 1 AS one FROM pg_sleep(0.2)")),
                connection.one(RawSQL("SELECT 1 AS one FROM pg_sleep(0.2)")),
                return_exceptions=True,
            )
            ran = [result for result in results if isinstance(result, pgorm.Record)]
            refused = [result for result in results if isinstance(result, pgorm.LifecycleError)]
            assert len(ran) == 1 and ran[0]["one"] == 1
            assert len(refused) == 1
            assert str(refused[0]) == "connection already has an active operation"


asyncio.run(main())
```

Await each statement on a connection before starting the next. Work that
should run side by side takes a connection each, which the pool's own
terminals do for you.

## Cancellation and timeouts

asyncio cancels a task by raising `asyncio.CancelledError` inside it, and
`asyncio.wait_for` and `asyncio.timeout` cancel the work they guard when time
runs out. A pgorm operation honours both. Cancelling one stops waiting for the
server and raises `asyncio.CancelledError` as it is, never a pgorm error;
`pgorm.CancelledError` is that same class. A connection whose state the
cancellation left unknown, one that may still be running the statement, is
discarded rather than lent again:

```python
import asyncio
import os

from pgorm import Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"], max_size=2) as pool:
        async with pool.connection(), pool.connection():
            assert pool.status().size == 2

        try:
            await asyncio.wait_for(pool.one(RawSQL("SELECT 1 AS slept FROM pg_sleep(1)")), 0.2)
        except TimeoutError:
            pass
        else:
            raise AssertionError("the statement outlasts its timeout")

        # The connection the statement ran on is gone; the pool still works.
        assert pool.status().size == 1
        row = await pool.one(RawSQL("SELECT 1 AS one"))
        assert row["one"] == 1


asyncio.run(main())
```

Cancelling stops the waiting, not the write. A statement the server has
already received may still commit; cancellation never promises that a write
did not happen. Inside a transaction it does promise the rollback:
[Transactions](08-transactions.md) says how.

`asyncio.wait_for` raises Python's own `TimeoutError`. `pgorm.TimeoutError` is
a different class, raised when a pool's `acquire_timeout` runs out. A
connection that cannot be opened within `connect_timeout` raises
`pgorm.ConnectionError`.

## Closing

A pool, a connection and a stream each have an `async with` form that closes
it on the way out, whatever way the block ends, and an explicit `close()`
(`aclose()` too) for code that cannot use a block. A transaction has only its
block, whose ending decides whether it commits.

Closing a pool refuses every waiting acquisition and cancels the operations
running on its connections. Anything used after it closes raises
`LifecycleError`:

```python
import asyncio
import os

import pgorm
from pgorm import Pool, RawSQL


async def main() -> None:
    pool = Pool(os.environ["DATABASE_URL"])
    assert (await pool.one(RawSQL("SELECT 1 AS one")))["one"] == 1
    await pool.close()
    assert pool.closed

    try:
        await pool.one(RawSQL("SELECT 1 AS one"))
    except pgorm.LifecycleError as error:
        assert str(error) == "pool is closed"
    else:
        raise AssertionError("a closed pool runs nothing")


asyncio.run(main())
```

Close a pool before its event loop ends. An `async with Pool(..)` block inside
the coroutine `asyncio.run` runs does that for you.
