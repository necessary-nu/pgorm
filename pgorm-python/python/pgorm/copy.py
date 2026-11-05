"""COPY's data flow: bytes into a table, and bytes out of a table or query,
or, in the binary format, rows of values both ways."""
from __future__ import annotations

from collections.abc import (
    AsyncGenerator, AsyncIterable, AsyncIterator, Buffer, Iterable, Sequence,
)
from contextlib import aclosing
from types import TracebackType
from typing import TYPE_CHECKING, Any, Literal, TypeAlias

from . import _native

if TYPE_CHECKING:
    from .runtime import Connection

CopySource: TypeAlias = Buffer | Iterable[Buffer] | AsyncIterable[Buffer]
"""What ``copy_in`` reads: bytes-like data, or a sync or async iterable of it."""

CopyRows: TypeAlias = Iterable[Sequence[object]] | AsyncIterable[Sequence[object]]
"""What ``copy_in_rows`` reads: a sync or async iterable of rows, each a
sequence of its columns' values in order."""

# Native writes gather small chunks and slice large ones, so each write is one
# bounded copy and a source of tiny rows costs few event-loop round trips.
_GATHER = 64 * 1024
_SLICE = 256 * 1024


# [spec:pgorm:req:bindings.argument-errors+1]
def _view(item: object) -> memoryview:
    if isinstance(item, str):
        raise TypeError("COPY data is bytes; encode text before copying it")
    if not isinstance(item, Buffer):
        raise TypeError(f"COPY data must be bytes-like, not {type(item).__name__}")
    try:
        return memoryview(item).cast("B")
    except TypeError:
        raise _native.ConstructionError("COPY data must be a contiguous buffer") from None


async def _single(source: Buffer) -> AsyncGenerator[memoryview]:
    yield _view(source)


async def _asynchronous(source: AsyncIterable[object]) -> AsyncGenerator[memoryview]:
    async for item in source:
        yield _view(item)


async def _synchronous(source: Iterable[object]) -> AsyncGenerator[memoryview]:
    for item in source:
        yield _view(item)


def _items(source: object) -> AsyncGenerator[memoryview]:
    """Check the source's shape before the COPY starts; items are checked as pulled."""
    if isinstance(source, str):
        raise TypeError("COPY data is bytes; encode text before copying it")
    if isinstance(source, Buffer):
        return _single(source)
    if isinstance(source, AsyncIterable):
        return _asynchronous(source)
    if isinstance(source, Iterable):
        return _synchronous(source)
    raise TypeError(
        "copy_in reads bytes-like data or a sync or async iterable of it, "
        f"not {type(source).__name__}"
    )


async def _pieces(items: AsyncIterator[memoryview]) -> AsyncGenerator[Buffer]:
    gathered = bytearray()
    async for view in items:
        if len(view) < _GATHER:
            gathered += view
            if len(gathered) >= _GATHER:
                yield gathered
                gathered = bytearray()
            continue
        if gathered:
            yield gathered
            gathered = bytearray()
        for start in range(0, len(view), _SLICE):
            yield view[start:start + _SLICE]
    if gathered:
        yield gathered


# [spec:pgorm:req:bindings.copy]
# [spec:pgorm:req:bindings.cancellation]
# [spec:pgorm:req:python.cancellation+1]
async def copy_in(
    native: _native._Connection | _native._Transaction,
    statement: _native.CopyFrom[Any],
    source: CopySource,
) -> int:
    """Run ``statement`` over ``source``, pulled only as the COPY takes data.

    Anything raised while the data is sent — by the source, by an item that is
    not bytes, or a cancellation — aborts the COPY and propagates unchanged.
    """
    items = _items(source)
    copy = await _native._copy_in(native, statement)
    try:
        async with aclosing(items), aclosing(_pieces(items)) as pieces:
            async for piece in pieces:
                if not await copy.write(piece):
                    break
    except BaseException:
        await copy.abort()
        raise
    return await copy.finish()


# [spec:pgorm:req:bindings.copy]
class CopyOutStream:
    """COPY TO's data as an async iterator of ``bytes`` chunks.

    A chunk is any slice of the format's byte stream: chunks already received
    are joined up to 64 KiB, and no row boundary is promised. At most a few
    chunks wait unread, so the server is held back when nothing pulls. The
    end of the data releases the connection or transaction; closing before it
    discards the connection. Use ``async with`` when reading might stop early.
    """

    def __init__(self, native: _native._CopyOut, connection: Connection | None) -> None:
        if not isinstance(native, _native._CopyOut):
            raise TypeError("copy streams must be opened by a pool, connection or transaction")
        self._native = native
        self._connection = connection
        self._closed = False

    @property
    def closed(self) -> bool:
        return self._closed or self._native.closed()

    def __aiter__(self) -> CopyOutStream:
        return self

    async def __anext__(self) -> bytes:
        if self._closed:
            raise StopAsyncIteration
        pending = self._native.next()
        try:
            chunk = await pending
        except BaseException:
            await self.aclose()
            raise
        if chunk is None:
            await self.aclose()
            raise StopAsyncIteration
        return chunk

    async def aclose(self) -> None:
        pending = self._native.close()
        self._closed = True
        try:
            await pending
        finally:
            if self._connection is not None:
                await self._connection.close()

    close = aclose

    async def __aenter__(self) -> CopyOutStream:
        if self.closed:
            raise _native.LifecycleError("copy stream is closed")
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        await self.aclose()

    # [spec:pgorm:req:python.truth-testing+4]
    def __bool__(self) -> bool:
        raise _native.ConstructionError("copy streams cannot be tested as Python booleans")

    def __repr__(self) -> str:
        return f"CopyOutStream(closed={self.closed})"


# Rows cross into the extension to be encoded a batch at a time: few enough
# that a slow source's rows are not held back long, many enough that a fast
# one costs few round trips.
_BATCH = 256


async def _rows_of(source: Iterable[object] | AsyncIterable[object]) -> AsyncGenerator[object]:
    if isinstance(source, AsyncIterable):
        async for row in source:
            yield row
    else:
        for row in source:
            yield row


def _row_source(rows: object) -> AsyncGenerator[object]:
    """Check the source's shape before the COPY starts; rows are checked as pulled."""
    if isinstance(rows, (str, Buffer)) or not isinstance(rows, (Iterable, AsyncIterable)):
        raise TypeError(
            "copy_in_rows reads a sync or async iterable of rows, "
            f"not {type(rows).__name__}"
        )
    return _rows_of(rows)


# [spec:pgorm:req:bindings.copy-rows+1]
async def copy_in_rows(
    native: _native._Connection | _native._Transaction,
    statement: _native.CopyFrom[Literal["binary"]],
    rows: CopyRows,
) -> int:
    """Run a binary ``statement`` over ``rows``, each a sequence of its columns'
    values, pulled only as the COPY takes data.

    The columns are the ones the server describes for the statement. A row of
    another width, or a value its column's type does not take, raises a
    construction error naming the column and aborts the COPY, as anything else
    raised while the data is sent does.
    """
    items = _row_source(rows)
    copy = await _native._copy_in(native, statement, True)
    try:
        async with aclosing(items):
            batch: list[object] = []
            async for row in items:
                batch.append(row)
                if len(batch) >= _BATCH:
                    if not await copy.write_rows(batch):
                        break
                    batch = []
            if batch:
                await copy.write_rows(batch)
    except BaseException:
        await copy.abort()
        raise
    return await copy.finish()


# [spec:pgorm:req:bindings.copy-rows+1]
class CopyRowStream:
    """A binary COPY TO's rows as an async iterator of tuples of values.

    Each field is decoded by its column's type as a result column of that type
    is; ``columns`` names the columns. Rows are fetched as they arrive, a batch
    at a time, with bounded buffering. The end of the data releases the
    connection or transaction; closing before it, or a field that does not
    decode, discards the connection. Use ``async with`` when reading might
    stop early.
    """

    def __init__(self, native: _native._CopyOut, connection: Connection | None) -> None:
        if not isinstance(native, _native._CopyOut):
            raise TypeError("copy streams must be opened by a pool, connection or transaction")
        self._native = native
        self._connection = connection
        self._closed = False
        self._rows: list[tuple[object, ...]] = []
        self._columns = tuple(native.columns())

    @property
    def columns(self) -> tuple[str, ...]:
        """The names of the columns each row holds, in order."""
        return self._columns

    @property
    def closed(self) -> bool:
        return self._closed or self._native.closed()

    def __aiter__(self) -> CopyRowStream:
        return self

    async def __anext__(self) -> tuple[object, ...]:
        if self._rows:
            return self._rows.pop()
        if self._closed:
            raise StopAsyncIteration
        pending = self._native.next_rows()
        try:
            rows = await pending
        except BaseException:
            await self.aclose()
            raise
        if rows is None:
            await self.aclose()
            raise StopAsyncIteration
        rows.reverse()
        self._rows = rows
        return self._rows.pop()

    async def aclose(self) -> None:
        self._rows = []
        pending = self._native.close()
        self._closed = True
        try:
            await pending
        finally:
            if self._connection is not None:
                await self._connection.close()

    close = aclose

    async def __aenter__(self) -> CopyRowStream:
        if self.closed:
            raise _native.LifecycleError("copy stream is closed")
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        await self.aclose()

    # [spec:pgorm:req:python.truth-testing+4]
    def __bool__(self) -> bool:
        raise _native.ConstructionError("copy streams cannot be tested as Python booleans")

    def __repr__(self) -> str:
        return f"CopyRowStream(columns={self._columns!r}, closed={self.closed})"
