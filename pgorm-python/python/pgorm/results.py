"""Detached records and explicit asynchronous stream ownership."""

from collections.abc import Callable, Mapping
from typing import TYPE_CHECKING, Any, Generic, TypeAlias, TypeVar, cast

from . import _native
from ._native import Field as Field, Record as Record

if TYPE_CHECKING:
    from .runtime import Connection

Query: TypeAlias = (
    _native.Select | _native.Insert | _native.Update | _native.Delete
    | _native.Merge | _native.RawSQL | _native.Compiled | _native.Pipeline
    | _native.DDL
)

Mapping.register(Record)

T = TypeVar("T", default=Record)
U = TypeVar("U")


# [spec:pgorm:req:bindings.streams+2]
class RowStream(Generic[T]):
    """An async iterator owning an active native query and its connection.

    Use ``async with await pool.stream(query) as rows`` when iteration might
    stop early. EOF releases the lease; early close or cancellation discards
    it. A stream from a Connection reserves that connection until EOF/close,
    and one from a Transaction that transaction, which an early close
    discards with its connection. A model's stream yields each row decoded as
    its query decodes it. The native stream hands over the rows it has
    received in batches, which the stream yields from memory.
    """

    def __init__(
        self, native: _native._Stream, connection: "Connection | None", *, release_connection: bool,
    ) -> None:
        if not isinstance(native, _native._Stream):
            raise TypeError("streams must be opened by a pool, a connection or a transaction")
        self._native = native
        self._connection = connection
        self._release_connection = release_connection
        self._closed = False
        self._decode: Callable[[Record], T] | None = None
        self._batch: list[Any] = []
        self._at = 0

    def _decoding(self, decode: Callable[[Record], U]) -> "RowStream[U]":
        """This stream, each row decoded by ``decode``: a decode that fails
        closes the stream before its error propagates."""
        stream: RowStream[U] = RowStream(
            self._native, self._connection, release_connection=self._release_connection
        )
        stream._decode = decode
        return stream

    @property
    def closed(self) -> bool:
        return self._closed or self._native.closed()

    def __aiter__(self) -> "RowStream[T]":
        return self

    async def __anext__(self) -> T:
        if self._closed:
            raise StopAsyncIteration
        if self._at < len(self._batch) and self._native.pullable():
            record = self._batch[self._at]
            self._at += 1
        else:
            self._batch = []
            pending = self._native.next()
            try:
                batch = await pending
            except BaseException:
                await self.aclose()
                raise
            if batch is None:
                await self._finish()
                raise StopAsyncIteration
            self._batch, self._at = batch, 1
            record = batch[0]
        if self._decode is None:
            return cast(T, record)
        try:
            return self._decode(record)
        except BaseException:
            await self.aclose()
            raise

    async def aclose(self) -> None:
        pending = self._native.close()
        self._closed = True
        self._batch = []
        try:
            await pending
        finally:
            await self._release()

    async def _finish(self) -> None:
        """End the stream at the end its pull found, which released what the
        native stream held, so there is nothing of it to close."""
        self._closed = True
        self._batch = []
        await self._release()

    async def _release(self) -> None:
        if self._release_connection and self._connection is not None:
            await self._connection.close()

    close = aclose

    async def __aenter__(self) -> "RowStream[T]":
        if self.closed:
            raise _native.LifecycleError("the stream is closed")
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.aclose()

    # [spec:pgorm:req:python.truth-testing+5]
    def __bool__(self) -> bool:
        raise _native.ConstructionError("result streams cannot be tested as Python booleans")

    def __repr__(self) -> str:
        return f"RowStream(closed={self.closed})"
