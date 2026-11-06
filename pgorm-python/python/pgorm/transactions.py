"""Transactions and savepoints, each open for exactly one ``async with`` block
over a native Rust scope."""
from __future__ import annotations

from contextlib import AbstractAsyncContextManager, asynccontextmanager
from collections.abc import AsyncIterator, Awaitable, Callable
from typing import TYPE_CHECKING, Any, Literal, TypeAlias

from . import _native
from .copy import CopyOutStream, CopyRowStream, CopySource, copy_in
from .copy_rows import RowCopies
from .results import Query, Record

if TYPE_CHECKING:
    from .runtime import Connection

TransactionMode: TypeAlias = Literal["default", "read_write", "read_only", "deferrable"]
IsolationLevel: TypeAlias = Literal["read_uncommitted", "read_committed", "repeatable_read", "serializable"]


# [spec:pgorm:req:bindings.transactions]
# [spec:pgorm:req:python.transactions+3]
@asynccontextmanager
async def _scope(
    opening: Callable[[], Awaitable[_native._Transaction]], parent: Connection | Transaction
) -> AsyncIterator[Transaction]:
    """Serve one block's transaction or savepoint, ending it as ``Transaction``
    describes."""
    native = await opening()
    transaction = Transaction(native, parent)
    try:
        yield transaction
    except BaseException as error:
        await _roll_back(native, error)
        raise
    try:
        await native.finish(True)
    except BaseException:
        await native.abort()
        raise


async def _roll_back(native: _native._Transaction, error: BaseException) -> None:
    if not native.closed():
        try:
            await native.finish(False)
        except _native.LifecycleError:
            # The scope refused the rollback (a statement or savepoint still
            # holds it, or it has just closed), so it cannot be recovered: it
            # is discarded, and the block's exception stands.
            await native.abort()
            return
        except _native.PgormError as failure:
            await native.abort()
            # asyncio's timeouts and task groups recognise a cancellation only
            # as the CancelledError itself, so a failed ROLLBACK never
            # replaces one.
            if isinstance(error, Exception):
                # An error a native future delivers is re-raised without
                # implicit chaining, so the context is set here.
                failure.__context__ = error
                raise
            return
        except BaseException:
            await native.abort()
            raise
    await native.wait_closed()


# [spec:pgorm:req:python.transactions+3]
class Transaction(RowCopies):
    """One native transaction or savepoint, open for the ``async with`` block
    that opened it and reserving its parent until that block exits.

    ``connection.transaction()`` and ``pool.transaction()`` open one, and
    ``tx.transaction()`` a real savepoint inside it. A clean exit commits, or
    releases a savepoint; a commit that fails discards the connection and
    raises. An exception rolls back and propagates. A ``ROLLBACK`` that fails
    discards the connection too, and is raised in place of the block's
    exception, which it carries as ``__context__``, unless that exception is a
    cancellation or an interpreter exit, which propagates as itself.
    """

    def __init__(self, native: _native._Transaction, parent: Connection | Transaction) -> None:
        if not isinstance(native, _native._Transaction) or not native.owner_matches(getattr(parent, "_native", None)):
            raise TypeError("transactions must be opened on their actual parent")
        self._native = native
        self._parent = parent

    @property
    def closed(self) -> bool:
        return self._native.closed()

    def transaction(self) -> AbstractAsyncContextManager[Transaction]:
        """Open a savepoint for an ``async with`` block, reserving this
        transaction until the block exits: a clean exit releases it, and an
        exception rolls back to it."""
        return _scope(self._native.begin, self)

    async def execute(self, query: Query) -> int:
        return await self._native.execute(query)

    async def query(self, query: Query) -> list[Record]:
        return await self._native.fetch(query, "all")

    async def one(self, query: Query) -> Record:
        return await self._native.fetch(query, "one")

    async def optional(self, query: Query) -> Record | None:
        return await self._native.fetch(query, "optional")

    async def copy_in(self, statement: _native.CopyFrom[Any], source: CopySource) -> int:
        """Run a COPY FROM inside this transaction; see ``Connection.copy_in``.

        A source that raises aborts the COPY and with it the transaction,
        which then refuses statements until it is rolled back.
        """
        return await copy_in(self._native, statement, source)

    async def copy_out(self, statement: _native.CopyTo[Any]) -> CopyOutStream:
        """Open a COPY TO; closing it before its end discards the transaction."""
        return CopyOutStream(await _native._copy_out(self._native, statement), None)

    async def _copy_holder(self) -> tuple[_native._Transaction, None]:
        return self._native, None

    async def copy_out_rows(self, statement: _native.CopyTo[Literal["binary"]]) -> CopyRowStream:
        """Open a binary COPY TO of rows; closing it before its end discards the transaction."""
        return CopyRowStream(await _native._copy_out(self._native, statement, True), None)

    async def notify(self, channel: str, payload: str) -> None:
        """Send a notification delivered when this transaction commits, never if it rolls back."""
        await self._native.notify(channel, payload)

    def __repr__(self) -> str:
        return f"Transaction(closed={self.closed})"
