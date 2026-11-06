# LISTEN and NOTIFY

PostgreSQL can tell a session when something happens. A session that has run
`LISTEN orders` receives a *notification* each time any session sends one on
the `orders` channel, with a text payload. It is how one process wakes
another: a new order is placed, and the kitchen's display hears about it at
once instead of polling the table.

A notification is delivered to the session that listened, outside the result
of any statement. A pooled connection cannot receive one: the pool's task
that drives it discards such messages, and the pool lends the connection to
someone else once it is returned. So a listener is a connection of its own,
opened for the purpose and owned by the listener alone.

## Listening

`await pool.listener()` opens a listening connection with the pool's
configuration and TLS. The connection is not taken from the pool and does not
count against its size, and closing the pool closes it. The `Listener` it
returns does two things:

- it chooses the channels, with `listen`, `unlisten` and `unlisten_all`;
- it is an async iterator of each `Notification` the connection receives,
  with its `channel`, its `payload` and the `process_id` of the session that
  sent it.

`notify(channel, payload)` sends a notification from a pool, a connection or
a transaction:

```python
import asyncio
import os

from pgorm import Pool


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with await pool.listener() as listener:
            await listener.listen("orders")
            await pool.notify("orders", "order 3 placed")

            heard = await asyncio.wait_for(anext(listener), 5)
            assert (heard.channel, heard.payload) == ("orders", "order 3 placed")
            assert heard.process_id > 0


asyncio.run(main())
```

Here Python differs from Rust in shape. Rust's `listener` returns two halves,
a `Listener` that chooses the channels and a `Notifications` stream; Python's
`Listener` is both, and its `async with` block, or `aclose()`, closes its
connection. Rust's capacity is an argument with `Listener::DEFAULT_CAPACITY`,
1024, to pass; Python's is the keyword argument `capacity`, 1024 by default.
Rust's `notify` is a function taking the connection; Python's is a method of
the pool, the connection and the transaction.

The listener's connection is driven by a task of its own, which queues each
notification as it arrives, so `listen` and `unlisten` complete whether or
not anyone is reading. A Rust `Listener` is cloned to hand to another task;
a Python program shares the one `Listener` instead: one task may add
channels while another iterates it. Only one pull may wait at a time, and a
second raises `LifecycleError`. A cancelled pull loses nothing: the
notification it was waiting for is the next pull's.

Once `listen` returns, every notification on the channel from a transaction
that commits afterwards is delivered. Once `unlisten` returns, none is,
though one already received is still waiting to be read.

Rust's `connect_listener`, which opens a listener from a connection
configuration and a TLS connector of the program's own rather than a pool's,
is not available in Python yet. A pool is cheap to make for the purpose,
since a pool connects to nothing until it is asked for a connection, and its
listener uses its URL and TLS options:

```python
import asyncio
import os

import pgorm
from pgorm import Pool


async def main() -> None:
    # A pool kept for its listener; it opens no connection of its own.
    listening = Pool(os.environ["DATABASE_URL"], max_size=1)
    listener = await listening.listener()
    await listener.listen("deliveries")
    await listener.unlisten_all()
    assert listening.status().size == 0

    # Closing the pool closes its listener.
    await listening.close()
    assert listener.closed
    try:
        await listener.listen("deliveries")
    except pgorm.LifecycleError as error:
        assert str(error) == "pool is closed"
    else:
        raise AssertionError("the listener closed with its pool")


asyncio.run(main())
```

## Channel names

A channel name is exact. `listen` sends it as a quoted identifier, so its
case and punctuation are kept: `Orders` and `orders` are two channels, and the
name is the same text `notify` sends to. PostgreSQL keeps 63 bytes of an
identifier and would quietly listen on a shortened name nothing can send to,
so a longer name raises `ConstructionError` before anything is sent. A name
that is not a `str` raises `TypeError`.

```python
import asyncio
import os

from pgorm import ConstructionError, Pool


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with await pool.listener() as listener:
            await listener.listen("Orders")
            await pool.notify("orders", "not heard: another channel")
            await pool.notify("Orders", "heard")
            heard = await asyncio.wait_for(anext(listener), 5)
            assert heard.payload == "heard"

            try:
                await listener.listen("o" * 64)
            except ConstructionError as error:
                assert "longer than the 63 bytes PostgreSQL keeps" in str(error)
            else:
                raise AssertionError("a channel name has at most 63 bytes")


asyncio.run(main())
```

Here Python differs from Rust: Rust's refusal is
`ListenError::ChannelTooLong`, inside `Error::Listen`.

## Notifications and transactions

`notify` runs `SELECT pg_notify($1, $2)` with the channel and payload bound
as parameters, so neither is ever written into the SQL, and the statement is
prepared once per connection. `NOTIFY` itself takes no parameters, which is
why pgorm does not send it.

A notification is delivered when the transaction that sent it commits: at
once outside a transaction, and never if the transaction rolls back. That
makes it safe to notify beside the write it announces, since listeners hear
of the order only once the order is there to read. Notifications arrive in
commit order across transactions and in sending order within one, and one
that repeats an earlier notification of the same transaction, the same
channel and the same payload, is folded into it:

```python
import asyncio
import os

from pgorm import Pool


class Declined(Exception):
    pass


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with await pool.listener() as listener:
            await listener.listen("orders")

            # Rolled back: never sent.
            try:
                async with pool.transaction() as tx:
                    await tx.notify("orders", "order 3 placed")
                    raise Declined("the card was declined")
            except Declined:
                pass

            # Committed: sent at the commit, the repeated payload once.
            async with pool.transaction() as tx:
                await tx.notify("orders", "order 4 placed")
                await tx.notify("orders", "order 4 placed")
                await tx.notify("orders", "order 4 paid")

            payloads = [(await asyncio.wait_for(anext(listener), 5)).payload for _ in range(2)]
            assert payloads == ["order 4 placed", "order 4 paid"]


asyncio.run(main())
```

The server refuses an empty channel name, one longer than 63 bytes, and a
payload of 8000 bytes or more, each a `DatabaseError` with SQLSTATE `22023`.
A channel or payload that is not a `str` raises `TypeError`. A payload is
best kept small, such as the id of the row that changed, with the listener
reading the row itself.

## When a listener stops

A listener's iteration ends only after an error, or when the listener is
closed, and nothing reconnects it. A notification sent while no session is
listening reaches nobody, so a listener that reconnected silently would hide
the notifications it missed. When iteration fails, open a new listener,
listen again, and then re-read whatever the notifications were telling you
about, since some of it may have changed in between. It fails in two ways:

- The connection fails or is ended, such as by a server restart. Iteration
  yields every notification it had already received, then raises the error
  the connection failed with: a `DatabaseError` with the server's SQLSTATE
  where the server sent one, such as `57P01` for a session an administrator
  ended, and a `ConnectionError` otherwise.
- The reader falls behind. At most `capacity` notifications wait unread.
  When that many are waiting and another arrives, the listener closes its
  connection at once, and iteration yields those waiting, then raises
  `ConnectionError`.

Either way the listener is then closed: the next pull ends the iteration, and
`listen` and `unlisten` raise. Here Python differs from Rust, whose stream
yields `Error::Postgres` and `ListenError::Overflow` as its last items.

Closing on overflow rather than waiting is deliberate. A listener that
stopped reading its connection would stop consuming PostgreSQL's
notification queue, which every database on the server shares, and once that
queue fills every `NOTIFY` on the server fails at commit. The default
capacity, 1024, is more than a reader that keeps up ever holds. A capacity
that is not an `int` raises `TypeError`, and one that is not positive
`ValueError`.

Here a listener of capacity two is sent three notifications before it reads
any:

```python
import asyncio
import os

import pgorm
from pgorm import Pool


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        listener = await pool.listener(capacity=2)
        await listener.listen("orders")
        async with pool.transaction() as tx:
            for order in (3, 4, 5):
                await tx.notify("orders", f"order {order} placed")

        # Each `listen` is a round trip, answered after the listener has been
        # sent what is pending, so this waits until the overflow has closed it.
        async def until_closed() -> None:
            while True:
                try:
                    await listener.listen("orders")
                except pgorm.ConnectionError:
                    return

        await asyncio.wait_for(until_closed(), 5)

        first = await anext(listener)
        second = await anext(listener)
        assert (first.payload, second.payload) == ("order 3 placed", "order 4 placed")
        try:
            await anext(listener)
        except pgorm.ConnectionError as error:
            assert "while 2 were waiting unread" in str(error)
        else:
            raise AssertionError("the third notification overflowed")
        assert [notification async for notification in listener] == []


asyncio.run(main())
```

And here the listener's session is ended from another connection, as a
server shutting down would end it. Iteration raises the server's error, and a
new listener takes over:

```python
import asyncio
import os

from pgorm import DatabaseError, Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        listener = await pool.listener()
        await listener.listen("kitchen")

        ended = await pool.one(RawSQL(
            "SELECT count(pg_terminate_backend(pid)) AS ended FROM pg_stat_activity "
            "WHERE datname = current_database() AND query = 'LISTEN \"kitchen\"'"
        ))
        assert ended["ended"] == 1

        try:
            await asyncio.wait_for(anext(listener), 5)
        except DatabaseError as error:
            assert error.sqlstate == "57P01"
        else:
            raise AssertionError("the listener's session was ended")
        assert listener.closed

        # Nothing reconnects: open a new listener and listen again.
        async with await pool.listener() as listener:
            await listener.listen("kitchen")
            await pool.notify("kitchen", "back")
            heard = await asyncio.wait_for(anext(listener), 5)
            assert heard.payload == "back"


asyncio.run(main())
```

Cancelling a `listen`, `unlisten` or `unlisten_all` closes the listener too,
since which channels the server listens on is then unknown. Closing the
listener ends its iteration, a pull waiting on it included. Closing its pool
closes it too, and then a pull, or one already waiting, raises
`LifecycleError`, as `listen` and `unlisten` do.

## A listener runs no statements

A `Listener` sends nothing but `LISTEN` and `UNLISTEN`, and has no `execute`,
`query` or `one`: mypy reports a call of one as an `attr-defined` error. A
statement on its connection would wait behind the notifications arriving
there, and a transaction would hold their delivery back until it ended. Read
and write through the pool or a connection from it, and send notifications
from there with `notify`.

```python
import asyncio
import os

from pgorm import Pool


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with await pool.listener() as listener:
            assert not hasattr(listener, "query")
            assert not hasattr(listener, "execute")

            await listener.listen("kitchen")
            async with pool.connection() as connection:
                await connection.notify("kitchen", "oven 2 is hot")
            heard = await asyncio.wait_for(anext(listener), 5)
            assert heard.payload == "oven 2 is hot"


asyncio.run(main())
```

The metrics layer reports a `notify` through an instrumented pool as the
statement it is, and does not see a listener at all ([Metrics](21-metrics.md)).
