"""The streams of registered reads: a statement's rows, each decoded natively
as the read's Rust selector decodes it, over a connection the stream holds."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from .._native import Compiled, _RowDecoder
    from ..results import RowStream


# [spec:pgorm:req:bindings.streams]
async def open_stream(db: Any, statement: Compiled, decoder: _RowDecoder | None) -> RowStream[Any]:
    """The rows of ``statement`` on a pool's connection of its own, released
    when the stream ends, or on ``db``'s connection, which it holds until
    then."""
    from ..results import RowStream
    from ..runtime import Connection, Pool

    if isinstance(db, Pool):
        connection = await db.acquire()
        try:
            native = await connection._native.stream(statement, decoder)
        except BaseException:
            await connection.close()
            raise
        return RowStream(native, connection, release_connection=True)
    if isinstance(db, Connection):
        return RowStream(await db._native.stream(statement, decoder), db, release_connection=False)
    raise TypeError("a registered read's rows stream from a Pool or a Connection")
