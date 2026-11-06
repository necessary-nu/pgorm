# Transactions

A transaction groups statements so that they take effect together or not at
all. Placing an order writes the order and each of its lines; a transaction
makes sure no one ever sees an order with half its lines, and that a failure
part-way leaves nothing behind.

In pgorm a transaction exists only while a block of code runs. In Rust the
block is a closure; in Python it is an `async with` block. How the block ends
ends the transaction: leaving it normally commits, and leaving it with an
exception rolls back. There is no handle to commit later and no transaction
to forget to end. The transaction is pgorm's own Rust transaction, and pgorm
sends its `BEGIN`, `COMMIT` and `ROLLBACK`; Python never sends them as SQL.

## Running a block in a transaction

`pool.transaction()` opens a transaction on a connection the pool lends for
the block, and `connection.transaction()` opens one on a connection you
already hold. The block receives a `Transaction`. It runs statements as a
pool or a connection does, with `execute`, `query`, `one` and `optional`, and
a model's terminals take it in the same place, as do a pipeline's and a
registered Rust entity's queries, writes and graphs, so every statement given
it is part of the transaction:

```python
import asyncio
import os
from datetime import datetime, timezone
from decimal import Decimal

from pgorm import Pool

from bakery import Order, OrderLine


async def main() -> None:
    placed_at = datetime(2026, 11, 4, 10, 0, tzinfo=timezone.utc)
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with pool.transaction() as tx:
            order = await Order.insert({
                "customer_id": 2,
                "bakery_id": 1,
                "placed_at": placed_at,
            }).returning().one(tx)
            await OrderLine.insert_many([
                {"order_id": order["id"], "cake_id": 5, "quantity": 1, "price": Decimal("6.00")},
                {"order_id": order["id"], "cake_id": 2, "quantity": 2, "price": Decimal("3.20")},
            ]).execute(tx)

        lines = await OrderLine.find().where(OrderLine.col("order_id") == order["id"]).count(pool)
        assert (order["id"], lines) == (3, 2)


asyncio.run(main())
```

Here Python differs from Rust: a Rust closure returns its result, and the
transaction hands it back. A Python block has no result. What it reads or
writes it keeps in variables, which outlive the block, as `order` does here.

While the transaction is open it holds its connection exclusively. In Rust
the compiler refuses a statement on the connection while its transaction
borrows it. In Python the same mistake raises `LifecycleError` when it runs,
rather than waiting behind the transaction. A `Transaction` used after its
block has ended raises `LifecycleError` too:

```python
import asyncio
import os

import pgorm
from pgorm import Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with pool.connection() as connection:
            async with connection.transaction() as tx:
                # The connection is the transaction's for now: use `tx`.
                try:
                    await connection.one(RawSQL("SELECT count(*) AS cakes FROM cake"))
                except pgorm.LifecycleError as error:
                    assert str(error) == "connection already has an active operation"
                else:
                    raise AssertionError("the transaction holds the connection")
                row = await tx.one(RawSQL("SELECT count(*) AS cakes FROM cake"))
                assert row["cakes"] == 5

            try:
                await tx.one(RawSQL("SELECT 1 AS one"))
            except pgorm.LifecycleError as error:
                assert str(error) == "transaction is closed"
            else:
                raise AssertionError("the transaction ended with its block")


asyncio.run(main())
```

A `Transaction` is not itself a context manager: only the block that opened
it ends it, and nothing commits or rolls it back by hand. A transaction holds
one statement at a time, as a connection does, so two statements started on
it together raise `LifecycleError` for the second.

## Rolling back

When an exception leaves the block, the transaction rolls back, and the
rollback has finished by the time the exception reaches the code around the
block. The exception is the one the block raised, unchanged. Here the second
line names a cake that does not exist, so its insert raises
`ForeignKeyViolationError`, and the order written before it is rolled back
with it:

```python
import asyncio
import os
from datetime import datetime, timezone
from decimal import Decimal

from pgorm import ForeignKeyViolationError, Pool

from bakery import Order, OrderLine


async def main() -> None:
    placed_at = datetime(2026, 11, 4, 10, 0, tzinfo=timezone.utc)
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        try:
            async with pool.transaction() as tx:
                order = await Order.insert({
                    "customer_id": 1,
                    "bakery_id": 1,
                    "placed_at": placed_at,
                }).returning().one(tx)
                await OrderLine.insert({
                    "order_id": order["id"],
                    "cake_id": 99,
                    "quantity": 1,
                    "price": Decimal("1.00"),
                }).execute(tx)
        except ForeignKeyViolationError as error:
            assert error.constraint == "fk-order_line-cake_id"
        else:
            raise AssertionError("there is no cake 99")

        assert await Order.find().count(pool) == 2


asyncio.run(main())
```

Raising is how a block rolls back. An exception of your own, raised when the
program decides the work should not stand, rolls back the same way and
reaches the code around the block as itself.

Here Python differs from Rust. Rust wraps a failed transaction in a
`TransactionError` that keeps apart the closure's own error, a failed `BEGIN`
or `COMMIT`, and a `ROLLBACK` that failed after the closure's error. Python
has no wrapper, and raises one exception:

- The block's own exception, after a rollback that succeeded.
- A `COMMIT` that fails raises its error, a `DatabaseError` or a
  `ConnectionError`, from the block's exit.
- A `ROLLBACK` that fails raises its error in place of the block's
  exception, which it carries as `__context__`. A cancellation,
  `KeyboardInterrupt` or `SystemExit` is never replaced this way: it
  propagates as itself, so that asyncio still sees the cancellation.

In Rust, a connection whose `COMMIT` or `ROLLBACK` failed is in a state no one
knows, and the program gives it back with `discard` so the pool closes it.
Python has no `discard`: the binding discards such a connection itself, and
the pool opens a fresh one when it next needs one. A block whose transaction
was already discarded, by a cancelled statement or by a `COPY` the server
refused ([COPY](17-copy.md)), keeps its own exception; if it exits normally
instead, its exit raises `LifecycleError`, since nothing is left to commit.

## Savepoints

`tx.transaction()` opens a savepoint: a transaction nested inside the first,
which commits into it or rolls back alone. It is an `async with` block of its
own, ended the same way. Here the order survives a line that fails:

```python
import asyncio
import os
from datetime import datetime, timezone
from decimal import Decimal

from pgorm import ForeignKeyViolationError, Pool

from bakery import Order, OrderLine


async def main() -> None:
    placed_at = datetime(2026, 11, 4, 10, 0, tzinfo=timezone.utc)
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        skipped = []
        async with pool.transaction() as tx:
            order = await Order.insert({
                "customer_id": 1,
                "bakery_id": 2,
                "placed_at": placed_at,
            }).returning().one(tx)
            for cake_id in [3, 99, 4]:
                try:
                    async with tx.transaction() as savepoint:
                        await OrderLine.insert({
                            "order_id": order["id"],
                            "cake_id": cake_id,
                            "quantity": 1,
                            "price": Decimal("3.00"),
                        }).execute(savepoint)
                except ForeignKeyViolationError:
                    skipped.append(cake_id)

        lines = await OrderLine.find().where(OrderLine.col("order_id") == order["id"]).count(pool)
        assert (lines, skipped) == (2, [99])


asyncio.run(main())
```

Only a missing cake is caught there. Any other failure leaves the
savepoint's block and then the transaction's, rolling the whole order back,
as Rust's `?` passes a savepoint's error on to the closure around it.

Without the savepoint, the failed insert would have aborted the whole
transaction: PostgreSQL refuses every later statement in a transaction one
statement has failed in, with SQLSTATE `25P02`, until it rolls back. Catching
the error inside the block does not undo that. A block that catches a failed
statement's error without a savepoint and then exits normally commits
nothing: PostgreSQL answers the `COMMIT` of a failed transaction by rolling it
back, and the block ends without an error. Put the statement that may fail in
a savepoint and catch its error outside the savepoint's block.

While a savepoint is open, its parent transaction refuses statements and
other savepoints with `LifecycleError`; use the savepoint's own `Transaction`
until its block ends.

## Choosing the isolation level and access mode

`pool.transaction()` and `connection.transaction()` take two keyword
arguments. `mode` chooses among the combinations PostgreSQL acts on, and
`isolation` the isolation level where the mode takes one:

| `mode` | Opens | `isolation` |
| --- | --- | --- |
| `"default"`, the default | a transaction under the session's defaults | not taken |
| `"read_write"` | `READ WRITE`, at the level given or the session's | optional |
| `"read_only"` | `READ ONLY`, in which every write is refused | optional |
| `"deferrable"` | `SERIALIZABLE READ ONLY DEFERRABLE`: it may wait for a safe snapshot, and then no serialization failure can end it | not taken |

The isolation levels are `"read_uncommitted"`, `"read_committed"`,
`"repeatable_read"` and `"serializable"`, and PostgreSQL decides what each
does. A mode or level outside these spellings raises `TypeError`, and an
isolation level given to `"default"` or `"deferrable"` raises
`ConstructionError`. `pgorm.TransactionMode` and `pgorm.IsolationLevel` name
the two choices for type annotations. A savepoint takes neither argument.

Here Python differs from Rust in spelling: Rust passes a `TransactionMode`
value to `transaction_with`, a method of a connection alone. Python
passes the same choices as keyword arguments, to a pool's `transaction` as
well as a connection's.

```python
import asyncio
import os

from pgorm import DatabaseError, Pool, RawSQL

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Two reads that see the same snapshot of the database.
        async with pool.transaction(mode="read_only", isolation="repeatable_read") as tx:
            cakes = await Cake.find().count(tx)
            total = await tx.one(RawSQL("SELECT sum(price) AS total FROM cake"))
        assert (cakes, str(total["total"])) == (5, "20.40")

        # A write in a read-only transaction is refused.
        try:
            async with pool.transaction(mode="read_only") as tx:
                await Cake.delete().where(Cake.col("id") == 1).execute(tx)
        except DatabaseError as error:
            assert error.sqlstate == "25006"
        else:
            raise AssertionError("a read-only transaction writes nothing")


asyncio.run(main())
```

## Retrying serializable transactions

Under `SERIALIZABLE` and `REPEATABLE READ`, PostgreSQL refuses a transaction
whose result another concurrent transaction has made inconsistent, with a
serialization failure (SQLSTATE `40001`), and expects the client to run it
again. It does the same to one side of a deadlock (`40P01`).

Rust's `transaction_with_retry` runs the whole transaction again, from
`BEGIN`, while it fails with one of those two errors. It is not available in
Python yet, nor is Rust's `is_retryable`, which tells those errors from the
rest. A Python program writes the loop: open the transaction again while its
block fails with a `DatabaseError` whose `sqlstate` is one of the two. The
binding itself never retries a statement or a transaction.

The work may run more than once, so it is a function the loop calls on each
attempt, and anything it does outside the database happens again each time.
Here another connection changes the cake's price after the first attempt has
read it, and the first attempt's update is refused; the second attempt reads
the new price and succeeds:

```python
import asyncio
import os
from collections.abc import Awaitable, Callable
from decimal import Decimal

from pgorm import DatabaseError, Pool, RawSQL, Transaction

from bakery import Cake

RETRYABLE = {"40001", "40P01"}


async def serializable[T](pool: Pool, retries: int, work: Callable[[Transaction], Awaitable[T]]) -> T:
    """Run `work` in a serializable transaction, and again from the start while
    it fails with a serialization failure or a deadlock, up to `retries` times."""
    attempt = 0
    while True:
        attempt += 1
        try:
            async with pool.transaction(mode="read_write", isolation="serializable") as tx:
                return await work(tx)
        except DatabaseError as error:
            if error.sqlstate not in RETRYABLE or attempt > retries:
                raise


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        attempts = 0

        async def double_fudge(tx: Transaction) -> Decimal:
            nonlocal attempts
            attempts += 1
            fudge = await Cake.find_by_key({"id": 1}).one(tx)
            if attempts == 1:
                # Someone else's change commits after this transaction read the row.
                await pool.execute(RawSQL("UPDATE cake SET price = price + 1 WHERE id = 1"))
            changed = await (
                Cake.update({"price": fudge["price"] * 2})
                .where(Cake.col("id") == 1)
                .returning("price")
                .one(tx)
            )
            price: Decimal = changed["price"]
            return price

        doubled = await serializable(pool, 3, double_fudge)
        assert (attempts, str(doubled)) == (2, "11.00")


asyncio.run(main())
```

The `return` inside the block commits before the value leaves the function,
and a serialization failure can also come from the `COMMIT`, which this loop
retries like any other.

## A cancelled block rolls back

A transaction also ends if the task running its block is cancelled before the
block finishes, as when an `asyncio.timeout` runs out or the task is
cancelled. [Python and asyncio](02-asyncio.md) covers cancellation in
general; this is what it does to a transaction:

- Cancelled while no statement is running, as while it awaits something
  else, the block's exit rolls the transaction back, and the connection
  stays usable.
- Cancelled while a statement is running, the whole connection is discarded,
  with the transaction and every savepoint on it, and the server rolls the
  transaction back as the connection closes.
- Cancelled while its exit commits, the commit's outcome is unknown: the
  connection is discarded, and the transaction may or may not have
  committed.

In every case the cancellation reaches the code around the block as
asyncio's own `CancelledError`, or the `TimeoutError` asyncio raises for a
timeout. In the first two, the next statement never sees the transaction's
writes:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Customer


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        try:
            async with asyncio.timeout(0.1):
                async with pool.transaction() as tx:
                    await Customer.insert({
                        "name": "Mo",
                        "email": "mo@example.com",
                        "notes": None,
                    }).execute(tx)
                    await asyncio.sleep(30)
        except TimeoutError:
            pass
        else:
            raise AssertionError("the block outlasts its timeout")

        # Mo's insert was rolled back with the transaction.
        assert await Customer.find().count(pool) == 2


asyncio.run(main())
```

A block that ends while a statement or a savepoint of its transaction is
still running, such as one whose task started a statement and did not await
it, cannot be rolled back normally either, and its connection is discarded.
Closing the pool, or the transaction's connection, interrupts the
transaction, running or idle, and the close finishes once the transaction has
let its connection go.

The block's context manager can be entered by hand, with `__aenter__`, rather
than with `async with`. One entered that way and dropped without its
`__aexit__` rolls its transaction back when asyncio finalizes it; that
rollback has five seconds, after which the connection is discarded. Use
`async with`, which never leaves it to that.

## Transactions in tasks

Rust runs a transaction inside `tokio::spawn` because its future is `Send`.
Python has no such bound: a transaction runs in any task of the event loop
that owns its pool. Each `pool.transaction()` takes a connection of its own,
so transactions in several tasks run side by side, up to the pool's size:

```python
import asyncio
import os

from pgorm import ModelRecord, Pool

from bakery import Bakery


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:

        async def open_bakery(name: str, city: str) -> ModelRecord:
            async with pool.transaction() as tx:
                return await Bakery.insert({"name": name, "city": city}).returning().one(tx)

        async with asyncio.TaskGroup() as group:
            noon = group.create_task(open_bakery("Noon", "Tromsø"))
            dusk = group.create_task(open_bakery("Dusk", "Bodø"))

        assert sorted([noon.result()["name"], dusk.result()["name"]]) == ["Dusk", "Noon"]
        assert await Bakery.find().count(pool) == 4


asyncio.run(main())
```

One transaction belongs to one piece of work at a time. Two tasks that share
a `Transaction` and run statements on it together get a `LifecycleError` for
the second, as two statements on one connection do. A pool used from another
event loop, such as one running in another thread, raises `LifecycleError`
before anything is sent.

A transaction runs every statement the package offers except a stream:
`stream` is not available on a `Transaction` in Python yet, though Rust
streams rows inside a transaction. Read a transaction's rows whole, or a page
at a time ([Pages, cursors and streams](06-pagination.md)). A transaction
does run `COPY` ([COPY](17-copy.md)) and `notify` ([LISTEN and
NOTIFY](18-listen-notify.md)).

A transaction on an instrumented pool, or on a connection it lent, reports
its begin, its statements, its savepoints and its commit or rollback to the
pool's collector ([Metrics](21-metrics.md)), whatever its mode.
