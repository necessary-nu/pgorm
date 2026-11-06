"""Asyncio ownership and context managers for the native PostgreSQL resources."""

from collections.abc import AsyncIterator, Awaitable, Callable
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from functools import partial
from pathlib import Path
from typing import Any, Literal, NamedTuple, TypeVar

from . import _native
from .pool_options import PoolHook, QueueMode, Recycle, TlsMode, _shape, _shared, _tls
from .copy import CopyOutStream, CopyRowStream, CopySource, copy_in
from .copy_rows import RowCopies
from .listen import Listener
from .raw_methods import RawStatements
from .results import Query, Record, RowStream
from .transactions import IsolationLevel, Transaction, TransactionMode, _scope, _with_retry

T = TypeVar("T")

# The most retries a transaction takes, a Rust u32.
_RETRIES_MAX = 2**32 - 1


def _retries(retries: object) -> int:
    if type(retries) is not int:
        raise TypeError("retries is an int")
    if not 0 <= retries <= _RETRIES_MAX:
        raise _native.ConstructionError("retries is a non-negative int within u32")
    return retries


# The largest listener capacity the native side holds, a Rust usize.
_USIZE_MAX = 2**64 - 1

STATEMENT_CACHE_CHANNEL: str = _native.STATEMENT_CACHE_CHANNEL
"""The channel pgorm-migration's runner notifies once a batch that applied a
migration commits; pass it as ``statement_cache_channel`` to clear on it."""

class PoolStatus(NamedTuple):
    """A snapshot of native pool capacity and current usage."""

    max_size: int
    size: int
    available: int
    waiting: int


# [spec:pgorm:req:python.runtime+2]
# [spec:pgorm:req:bindings.runtime]
# [spec:pgorm:req:bindings.connections+3]
# [spec:pgorm:req:python.lifecycle+4]
class Pool(RowCopies, RawStatements):
    """A pool owned by the asyncio loop running at construction.

    TLS verifies the certificate and hostname by default. Explicit
    ``sslmode=disable`` in the DSN or ``tls="disable"`` selects plaintext, and
    ``tls`` names the weaker modes, ``"prefer"``, ``"require"`` and
    ``"verify-ca"``, which only an explicit option selects. ``ca`` supplies
    PEM trust anchors as ``str`` or ``bytes``, and ``cafile`` names a file
    holding them, never both; otherwise the roots the platform's certificate
    store trusts are used, and a store holding none raises
    ``ConstructionError``. ``cert`` and ``key``, or ``certfile`` and
    ``keyfile``, are a client certificate and its private key, presented to a
    server that asks for one. A pool keeps reusable connections and one
    shared Tokio runtime.

    ``wait_timeout``, ``create_timeout`` and ``recycle_timeout`` bound waiting
    for a free connection, opening one and checking a returned one, each off
    unless given; ``acquire_timeout`` bounds the whole acquisition.
    ``queue_mode`` lends the idle connection returned first (``"fifo"``) or
    last (``"lifo"``). ``post_create``, ``pre_recycle`` and ``post_recycle``
    are hooks, each called with a ``HookConnection`` as a connection opens,
    before a returned one is checked and after; one that raises refuses the
    connection. ``recycle_sql`` recycles with SQL of the program's own in
    place of ``recycle``'s methods. ``tag`` names the pool; without one pgorm
    names it ``default-N``.

    An argument of a type the pool does not take, a choice outside its names
    among them, raises ``TypeError``; a value the pool cannot use raises
    ``ConstructionError``.

    ``statement_cache_channel`` names a channel the pool listens on through a
    connection of its own, from its first acquisition until it closes; every
    notification there, whatever its payload, clears every connection's cached
    statements as ``clear_statement_caches()`` does. ``STATEMENT_CACHE_CHANNEL``
    is the one pgorm-migration's runner notifies after a migration.
    """

    def __init__(
        self,
        dsn: str,
        *,
        tls: TlsMode | None = None,
        ca: str | bytes | None = None,
        cafile: str | Path | None = None,
        cert: str | bytes | None = None,
        certfile: str | Path | None = None,
        key: str | bytes | None = None,
        keyfile: str | Path | None = None,
        max_size: int = 10,
        connect_timeout: float = 10.0,
        acquire_timeout: float = 30.0,
        wait_timeout: float | None = None,
        create_timeout: float | None = None,
        recycle_timeout: float | None = None,
        queue_mode: QueueMode = "fifo",
        statement_cache_size: int = 128,
        recycle: Recycle | None = None,
        recycle_sql: str | None = None,
        statement_cache_channel: str | None = None,
        tag: str | None = None,
        post_create: PoolHook | None = None,
        pre_recycle: PoolHook | None = None,
        post_recycle: PoolHook | None = None,
    ) -> None:
        shared = _shared(dsn, recycle, recycle_sql, statement_cache_size, statement_cache_channel, connect_timeout)
        if tag is not None and not isinstance(tag, str):
            raise TypeError("tag is a str or None")
        shape = _shape(
            max_size, acquire_timeout, wait_timeout, create_timeout, recycle_timeout, queue_mode,
            post_create, pre_recycle, post_recycle,
        )
        try:
            self._native = _native._Pool(
                dsn, shape, tag=tag, **_tls(tls, ca, cafile, cert, certfile, key, keyfile), **shared,
            )
        except OverflowError as error:
            raise _native.ConstructionError(str(error)) from None

    @classmethod
    def _adopt(cls, native: "_native._Pool") -> "Pool":
        """A pool around ``native``, which a ``MultiPool`` built."""
        pool = cls.__new__(cls)
        pool._native = native
        return pool

    @property
    def closed(self) -> bool:
        return self._native.closed()

    @property
    def tag(self) -> str:
        """The pool's name: its ``tag``, or the ``default-N`` pgorm gave it."""
        return self._native.tag()

    def status(self) -> PoolStatus:
        return PoolStatus(*self._native.status())

    def clear_statement_caches(self) -> None:
        """Drop every connection's cached prepared statements, so each statement
        is prepared again: an idle connection's before it is next lent, and a
        checked-out one's before it is lent again after its return. Call it after
        DDL a cached plan does not see, such as a domain's changed default."""
        self._native.clear_statement_caches()

    async def acquire(self) -> "Connection":
        """Check out a connection, with a cancellable acquisition deadline."""
        return Connection(await self._native.acquire(), self)

    @asynccontextmanager
    async def connection(self) -> AsyncIterator["Connection"]:
        """Check out a connection and release it when the block exits."""
        connection = await self.acquire()
        try:
            yield connection
        finally:
            await connection.close()

    async def ping(self) -> bool:
        """Check connectivity using pgorm's ordinary cached query path."""
        async with self.connection() as connection:
            return await connection.ping()

    @asynccontextmanager
    async def transaction(self, *, mode: TransactionMode = "default", isolation: IsolationLevel | None = None) -> AsyncIterator[Transaction]:
        """Open a transaction on a connection of its own for an ``async with``
        block; see ``Connection.transaction``."""
        async with self.connection() as connection:
            async with connection.transaction(mode=mode, isolation=isolation) as transaction:
                yield transaction

    async def transaction_with_retry(
        self,
        work: Callable[[Transaction], Awaitable[T]],
        *,
        retries: int,
        mode: TransactionMode = "default",
        isolation: IsolationLevel | None = None,
    ) -> T:
        """Run ``work`` in a transaction on a connection of its own, and again
        from ``BEGIN`` while it fails retryably; see
        ``Connection.transaction_with_retry``."""
        async with self.connection() as connection:
            return await connection.transaction_with_retry(
                work, retries=retries, mode=mode, isolation=isolation
            )

    async def execute(self, query: Query) -> int:
        """Execute a native builder or explicit RawSQL and return affected rows."""
        async with self.connection() as connection:
            return await connection.execute(query)

    async def query(self, query: Query) -> list[Record]:
        async with self.connection() as connection:
            return await connection.query(query)

    async def one(self, query: Query) -> Record:
        async with self.connection() as connection:
            return await connection.one(query)

    async def optional(self, query: Query) -> Record | None:
        async with self.connection() as connection:
            return await connection.optional(query)

    async def stream(self, query: Query) -> RowStream:
        """Open a stream; its iterator keeps the acquired connection alive."""
        connection = await self.acquire()
        try:
            native = await connection._native.stream(query)
        except BaseException:
            await connection.close()
            raise
        return RowStream(native, connection, release_connection=True)

    async def copy_in(self, statement: "_native.CopyFrom[Any]", source: CopySource) -> int:
        """Run a COPY FROM on a connection of its own; see ``Connection.copy_in``."""
        async with self.connection() as connection:
            return await connection.copy_in(statement, source)

    async def copy_out(self, statement: "_native.CopyTo[Any]") -> CopyOutStream:
        """Open a COPY TO whose stream owns the connection it acquired."""
        connection = await self.acquire()
        try:
            native = await _native._copy_out(connection._native, statement)
        except BaseException:
            await connection.close()
            raise
        return CopyOutStream(native, connection)

    async def _copy_holder(self) -> tuple[_native._Connection, "Connection"]:
        connection = await self.acquire()
        return connection._native, connection

    async def copy_out_rows(self, statement: "_native.CopyTo[Literal['binary']]") -> CopyRowStream:
        """Open a binary COPY TO of rows whose stream owns the connection it acquired."""
        connection = await self.acquire()
        try:
            native = await _native._copy_out(connection._native, statement, True)
        except BaseException:
            await connection.close()
            raise
        return CopyRowStream(native, connection)

    # [spec:pgorm:req:bindings.argument-errors+1]
    async def listener(self, *, capacity: int = 1024) -> Listener:
        """Open a dedicated listening connection with this pool's configuration.

        The connection is the listener's own, outside the pool's size, and
        closing the pool closes it. ``capacity`` bounds the notifications
        waiting unread; see ``Listener``. A ``capacity`` that is no ``int``
        raises ``TypeError``, and one that is not positive ``ValueError``.
        """
        if type(capacity) is not int:
            raise TypeError("capacity is an int")
        if not 1 <= capacity <= _USIZE_MAX:
            raise ValueError("capacity must be a positive integer")
        return Listener(await self._native.listener(capacity))

    async def notify(self, channel: str, payload: str) -> None:
        """Send a notification on a connection of its own; see ``Connection.notify``."""
        async with self.connection() as connection:
            await connection.notify(channel, payload)

    async def close(self) -> None:
        """Reject waiters, cancel active operations and release connections."""
        await self._native.close()

    aclose = close

    async def __aenter__(self) -> "Pool":
        try:
            await self.ping()
        except BaseException:
            await self.close()
            raise
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.close()

    def __repr__(self) -> str:
        return f"Pool(closed={self.closed})"


class Connection(RowCopies, RawStatements):
    """One checked-out native connection; concurrent operations are errors.

    Keep the pool alive until the connection is released. Query cancellation
    discards the connection when its state cannot be established.
    """

    def __init__(self, native: _native._Connection, pool: Pool) -> None:
        if not isinstance(native, _native._Connection):
            raise TypeError("connections must be acquired from a pool")
        self._native = native
        self._pool = pool

    @property
    def closed(self) -> bool:
        return self._native.closed()

    def transaction(
        self, *, mode: TransactionMode = "default", isolation: IsolationLevel | None = None
    ) -> AbstractAsyncContextManager[Transaction]:
        """Open a transaction for an ``async with`` block, reserving this
        connection until the block exits: a clean exit commits, and an
        exception rolls back; see ``Transaction``."""
        return _scope(partial(self._native.begin, mode=mode, isolation=isolation), self)

    async def transaction_with_retry(
        self,
        work: Callable[[Transaction], Awaitable[T]],
        *,
        retries: int,
        mode: TransactionMode = "default",
        isolation: IsolationLevel | None = None,
    ) -> T:
        """Run ``work`` in a transaction, and again from ``BEGIN``, up to
        ``retries`` more times, while it fails with a serialization failure
        or a deadlock, as Rust's ``transaction_with_retry`` does: an exception
        from ``work`` whose ``is_retryable`` is true, or a ``COMMIT`` that
        fails so. ``work`` receives each attempt's ``Transaction`` and its
        result is returned once an attempt commits. It may run more than once,
        so whatever it does outside the database happens again each time.
        Any other failure, or one past the last retry, propagates as a
        transaction block's does."""
        retrying = await self._native.begin_retrying(
            _retries(retries), mode=mode, isolation=isolation
        )
        return await _with_retry(retrying, self, work)

    async def ping(self) -> bool:
        return await self._native.ping()

    async def execute(self, query: Query) -> int:
        return await self._native.execute(query)

    async def query(self, query: Query) -> list[Record]:
        """Materialize all rows through Rust decoders, preserving output names."""
        return await self._native.fetch(query, "all")

    async def one(self, query: Query) -> Record:
        """Require exactly one row; zero or multiple rows raise DecodeError."""
        return await self._native.fetch(query, "one")

    async def optional(self, query: Query) -> Record | None:
        """Require at most one row; only zero rows produce None."""
        return await self._native.fetch(query, "optional")

    async def stream(self, query: Query) -> RowStream:
        native = await self._native.stream(query)
        return RowStream(native, self, release_connection=False)

    async def copy_in(self, statement: "_native.CopyFrom[Any]", source: CopySource) -> int:
        """Run a COPY FROM over ``source`` and return the rows it inserted.

        ``source`` is bytes-like data, or a sync or async iterable of it, in the
        statement's format; it is pulled only as the COPY takes data. If the
        source raises, the COPY is aborted, nothing is written and the
        exception propagates. A COPY the server refuses before any data
        moves closes this connection. The connection is reserved until the
        COPY ends.
        """
        return await copy_in(self._native, statement, source)

    async def copy_out(self, statement: "_native.CopyTo[Any]") -> CopyOutStream:
        """Open a COPY TO; the stream reserves this connection until it ends."""
        return CopyOutStream(await _native._copy_out(self._native, statement), None)

    async def _copy_holder(self) -> tuple[_native._Connection, None]:
        return self._native, None

    async def copy_out_rows(self, statement: "_native.CopyTo[Literal['binary']]") -> CopyRowStream:
        """Open a binary COPY TO whose rows arrive as tuples of values, each
        decoded by its column's type as a result column is; the stream reserves
        this connection until it ends."""
        return CopyRowStream(await _native._copy_out(self._native, statement, True), None)

    async def notify(self, channel: str, payload: str) -> None:
        """Send ``payload`` on ``channel`` with ``SELECT pg_notify($1, $2)``.

        Both values are bound. Every session listening on the channel
        receives it, at once here and at commit inside a transaction. The
        server refuses a payload of 8000 bytes or more.
        """
        await self._native.notify(channel, payload)

    async def close(self) -> None:
        await self._native.close()

    async def discard(self) -> None:
        """Close the connection and discard it rather than return it to its
        pool, as Rust's ``DatabaseConnection::discard`` does: for a connection
        whose session state is not what the next borrower should find. The pool
        opens a fresh one in its place."""
        await self._native.discard()

    aclose = close

    async def __aenter__(self) -> "Connection":
        if self.closed:
            raise _native.LifecycleError("connection is closed")
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.close()

    def __repr__(self) -> str:
        return f"Connection(closed={self.closed})"


async def connect(dsn: str, **options: Any) -> Pool:
    """Construct a pool and validate its PostgreSQL connection before return."""
    pool = Pool(dsn, **options)
    return await pool.__aenter__()
