"""Asyncio ownership and context managers for the native PostgreSQL resources."""

from collections.abc import AsyncIterator
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from functools import partial
from pathlib import Path
from typing import Any, Literal, NamedTuple

from . import _native
from .copy import CopyOutStream, CopyRows, CopyRowStream, CopySource, copy_in, copy_in_rows
from .listen import Listener
from .results import Query, Record, RowStream
from .transactions import IsolationLevel, Transaction, TransactionMode, _scope


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
# [spec:pgorm:req:bindings.connections+2]
# [spec:pgorm:req:python.lifecycle+3]
class Pool:
    """A pool owned by the asyncio loop running at construction.

    TLS verifies the certificate and hostname by default. Explicit
    ``sslmode=disable`` in the DSN or ``tls="disable"`` selects plaintext.
    ``ca`` supplies PEM trust anchors as ``str`` or ``bytes``, and ``cafile``
    names a file holding them, never both; otherwise the roots the platform's
    certificate store trusts are used, and a store holding none raises
    ``ConstructionError``. A pool keeps reusable connections and one shared
    Tokio runtime.

    An argument of a type the pool does not take, a ``tls`` or ``recycle``
    outside its choices among them, raises ``TypeError``; a value the pool
    cannot use raises ``ConstructionError``.

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
        tls: Literal["verify-full", "disable"] | None = None,
        ca: str | bytes | None = None,
        cafile: str | Path | None = None,
        max_size: int = 10,
        connect_timeout: float = 10.0,
        acquire_timeout: float = 30.0,
        statement_cache_size: int = 128,
        recycle: Literal["verified", "fast"] = "verified",
        statement_cache_channel: str | None = None,
    ) -> None:
        if not isinstance(dsn, str):
            raise TypeError("dsn is a connection string")
        if tls not in (None, "verify-full", "disable"):
            raise TypeError("tls is 'verify-full', 'disable' or None")
        if recycle not in ("verified", "fast"):
            raise TypeError("recycle is 'verified' or 'fast'")
        for name, count in (("max_size", max_size), ("statement_cache_size", statement_cache_size)):
            if type(count) is not int:
                raise TypeError(f"{name} is an int")
            if count < 0:
                raise _native.ConstructionError(f"{name} must be a nonnegative integer")
        for name, seconds in (("connect_timeout", connect_timeout), ("acquire_timeout", acquire_timeout)):
            if isinstance(seconds, bool) or not isinstance(seconds, (int, float)):
                raise TypeError(f"{name} is a number of seconds")
        if statement_cache_channel is not None and not isinstance(statement_cache_channel, str):
            raise TypeError("statement_cache_channel is a str or None")
        if ca is not None and not isinstance(ca, (str, bytes)):
            raise TypeError("ca is PEM certificate data, as str or bytes")
        if ca is not None and cafile is not None:
            raise _native.ConstructionError("pass the CA as ca or as cafile, not both")
        if isinstance(ca, str):
            ca_pem: bytes | None = ca.encode()
        elif ca is not None:
            ca_pem = ca
        else:
            try:
                ca_pem = Path(cafile).read_bytes() if cafile is not None else None
            except OSError:
                raise _native.ConstructionError("could not read the CA certificate file") from None
        try:
            self._native = _native._Pool(
                dsn, tls=tls, ca_pem=ca_pem, max_size=max_size,
                connect_timeout=float(connect_timeout), acquire_timeout=float(acquire_timeout),
                statement_cache_size=statement_cache_size, recycle=recycle,
                statement_cache_channel=statement_cache_channel,
            )
        except OverflowError as error:
            raise _native.ConstructionError(str(error)) from None

    @property
    def closed(self) -> bool:
        return self._native.closed()

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

    async def copy_in_rows(
        self, statement: "_native.CopyFrom[Literal['binary']]", rows: CopyRows
    ) -> int:
        """Run a binary COPY FROM of rows on a connection of its own; see ``Connection.copy_in_rows``."""
        async with self.connection() as connection:
            return await connection.copy_in_rows(statement, rows)

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


class Connection:
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

    async def copy_in_rows(
        self, statement: "_native.CopyFrom[Literal['binary']]", rows: CopyRows
    ) -> int:
        """Run a binary COPY FROM whose rows are sequences of values and return the rows it inserted.

        The columns are the ones the server describes for the statement, before
        the COPY starts; a description the server refuses raises its database
        error and leaves this connection usable. Each value is read as a
        parameter is and written against its column's type as a parameter is
        bound. A row of another width, or a value its column's type does not
        take, raises a construction error and aborts the COPY, as anything
        raised while the data is sent does, so nothing is written.
        """
        return await copy_in_rows(self._native, statement, rows)

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
