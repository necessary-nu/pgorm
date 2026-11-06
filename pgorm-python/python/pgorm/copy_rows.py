"""A binary COPY's rows one at a time: a sink a program sends rows and a
model's records into, and the description of a COPY's columns, which a pool,
a connection and a transaction share."""
from __future__ import annotations

from abc import ABC, abstractmethod
from types import TracebackType
from typing import TYPE_CHECKING, Any, Literal, overload

from . import _native
from .copy import CopyRows, copy_in_rows

if TYPE_CHECKING:
    from .models.record import ModelRecord
    from .runtime import Connection


# [spec:pgorm:req:bindings.copy-rows+2]
class CopyRowSink:
    """A binary COPY FROM taking rows one at a time, as Rust's ``CopyRowSink``.

    ``columns`` are the ones the server described before the COPY started.
    ``send`` writes one row, a sequence of its columns' values; ``send_model``
    writes a model's record, each column taken from the record's field of that
    column. A row of another width, or a value its column's type does not
    take, raises ``ConstructionError`` naming the column before anything of
    it is written, and the sink goes on taking rows. ``finish`` completes the
    COPY and returns the rows written. Leaving the ``async with`` block, or
    ``close``, without ``finish`` abandons the COPY: nothing is written.
    """

    def __init__(self, native: _native._CopyIn, connection: Connection | None) -> None:
        if not isinstance(native, _native._CopyIn):
            raise TypeError("copy sinks must be opened by a pool, connection or transaction")
        self._native = native
        self._connection = connection
        self._columns = tuple(native.described())

    @property
    def columns(self) -> tuple[_native.CopyColumn, ...]:
        """The COPY's columns, in order, with their types."""
        return self._columns

    @property
    def closed(self) -> bool:
        return self._native.closed()

    async def send(self, row: object) -> None:
        """Write one row, a sequence of its columns' values."""
        if not await self._native.write_rows([row]):
            await self.finish()
            raise _native.LifecycleError("COPY has ended")

    async def send_model(self, record: ModelRecord) -> None:
        """Write ``record``, each of the COPY's columns taken from the record's
        field whose column it is; a column no field reads raises
        ``ConstructionError``."""
        from .models.record import ModelRecord

        if not isinstance(record, ModelRecord):
            raise TypeError("send_model writes a model's record")
        fields = dict(zip(record.columns, record.selected))
        values: list[object] = []
        for column in self._columns:
            field = fields.get(column.name)
            if field is None:
                raise _native.ConstructionError(
                    f"the record has no field for the COPY's column {column.name!r}"
                )
            values.append(record.tagged(field))
        await self.send(values)

    async def finish(self) -> int:
        """Complete the COPY and return the rows it wrote."""
        try:
            return await self._native.finish()
        finally:
            await self._release()

    async def aclose(self) -> None:
        """Abandon the COPY unless it finished; nothing of it is written."""
        try:
            if not self._native.closed():
                await self._native.abort()
        finally:
            await self._release()

    close = aclose

    async def _release(self) -> None:
        if self._connection is not None:
            connection, self._connection = self._connection, None
            await connection.close()

    async def __aenter__(self) -> CopyRowSink:
        if self.closed:
            raise _native.LifecycleError("copy sink is closed")
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        await self.aclose()

    # [spec:pgorm:req:python.truth-testing+5]
    def __bool__(self) -> bool:
        raise _native.ConstructionError("copy sinks cannot be tested as Python booleans")

    def __repr__(self) -> str:
        return f"CopyRowSink(columns={[column.name for column in self._columns]!r}, closed={self.closed})"


class RowCopies(ABC):
    """The COPY methods a pool, a connection and a transaction share: each
    says through ``_copy_holder`` what runs them, a pool lending a connection
    of their own that is closed once they end."""

    @abstractmethod
    async def _copy_holder(self) -> tuple[_native._Connection | _native._Transaction, Connection | None]:
        """What runs the COPY, and the connection lent for it alone, if any."""

    @overload
    async def copy_in_rows(self, statement: _native.CopyFrom[Literal["binary"]]) -> CopyRowSink: ...

    @overload
    async def copy_in_rows(self, statement: _native.CopyFrom[Literal["binary"]], rows: CopyRows) -> int: ...

    # [spec:pgorm:req:bindings.copy-rows+2]
    async def copy_in_rows(
        self, statement: _native.CopyFrom[Literal["binary"]], rows: CopyRows | None = None
    ) -> int | CopyRowSink:
        """Run a binary COPY FROM whose rows are sequences of values.

        With ``rows``, a sync or async iterable of them, pull them as the COPY
        takes data and return the rows written; a row of another width, or a
        value its column's type does not take, raises a construction error and
        aborts the COPY, as anything raised while the data is sent does, so
        nothing is written. Without, return a ``CopyRowSink`` the program
        sends rows into. The columns are the ones the server describes for the
        statement, before the COPY starts; a description the server refuses
        raises its database error.
        """
        native, owned = await self._copy_holder()
        if rows is None:
            try:
                return CopyRowSink(await _native._copy_in(native, statement, True), owned)
            except BaseException:
                if owned is not None:
                    await owned.close()
                raise
        try:
            return await copy_in_rows(native, statement, rows)
        finally:
            if owned is not None:
                await owned.close()

    async def copy_in_columns(self, statement: _native.CopyFrom[Any]) -> list[_native.CopyColumn]:
        """The columns a COPY FROM writes, with their types, described by the
        server without starting it."""
        return await self._described(statement, False)

    async def copy_out_columns(self, statement: _native.CopyTo[Any]) -> list[_native.CopyColumn]:
        """The columns a COPY TO reads, with their types, described by the
        server without starting it."""
        return await self._described(statement, True)

    async def _described(
        self, statement: _native.CopyFrom[Any] | _native.CopyTo[Any], out: bool
    ) -> list[_native.CopyColumn]:
        native, owned = await self._copy_holder()
        try:
            return await _native._copy_columns(native, statement, out)
        finally:
            if owned is not None:
                await owned.close()
