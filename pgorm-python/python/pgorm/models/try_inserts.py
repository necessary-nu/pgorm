"""A model's try-inserts: an INSERT that may write nothing, as pgorm's
``TryInsert`` is, whose terminals say what it did with a ``TryInsertResult``:
``Inserted`` with what it wrote, ``Conflicted`` when its conflict clause
skipped every row, or ``Empty`` when it had nothing to write and sent
nothing."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass, replace
from typing import TYPE_CHECKING, Any, Self

from .. import _native as p
from .queries import ModelRows, written
from .record import Database, ModelRecord
from .writes import ModelInsert, ModelInsertMany

if TYPE_CHECKING:
    from .model import Model


# [spec:pgorm:req:bindings.model-try-inserts]
class TryInsertResult:
    """What a try-insert did, as pgorm's ``TryInsertResult`` says:
    ``Inserted`` with what it wrote, ``Conflicted`` when its conflict clause
    skipped every row, or ``Empty`` when it had nothing to write and sent
    nothing."""

    @dataclass(frozen=True, slots=True)
    class Inserted[T]:
        value: T

    @dataclass(frozen=True, slots=True)
    class Conflicted:
        pass

    @dataclass(frozen=True, slots=True)
    class Empty:
        pass



@dataclass(frozen=True, eq=False, slots=True)
class _Tried[I: (ModelInsert, ModelInsertMany)]:
    """What a model's try-insert of one row and of a batch share: the insert
    it runs, and whether it writes nothing — no rows, or a row that sets no
    field, which pgorm's ``TryInsert`` answers ``Empty`` without sending."""

    insert: I

    def __post_init__(self) -> None:
        if not isinstance(self.insert, (ModelInsert, ModelInsertMany)):
            raise TypeError("a try-insert comes from an insert's on_conflict_do_nothing() or on_empty_do_nothing()")

    @property
    def model(self) -> Model:
        return self.insert.model

    @property
    def statement(self) -> p.Insert:
        return written(self.insert.statement)

    def _empty(self) -> bool:
        return self.insert.statement is None or self.insert.fields == ()

    def inspect(self) -> p.Compiled:
        """The SQL and values the insert runs."""
        return written(self.insert.statement).inspect()

    def on_conflict(self, action: p.Conflict | p.ConflictUpdate) -> Self:
        return replace(self, insert=self.insert.on_conflict(action))

    def add(self, values: Mapping[str, Any]) -> ModelTryInsertMany:
        return ModelTryInsertMany(self.insert.add(values))

    def add_many(self, rows: Sequence[Mapping[str, Any]]) -> ModelTryInsertMany:
        return ModelTryInsertMany(self.insert.add_many(rows))

    async def execute(
        self, db: Database
    ) -> TryInsertResult.Inserted[int] | TryInsertResult.Conflicted | TryInsertResult.Empty:
        """The rows written, as pgorm's ``TryInsert::exec`` answers:
        ``Conflicted`` when a conflict clause wrote none of them, ``Empty``
        for nothing to write."""
        if self._empty():
            return TryInsertResult.Empty()
        count = await db.execute(written(self.insert.statement))
        if count == 0 and self.insert.conflict:
            return TryInsertResult.Conflicted()
        return TryInsertResult.Inserted(count)

    def __bool__(self) -> bool:
        raise p.ConstructionError("model writes cannot be tested as Python booleans")


# [spec:pgorm:req:bindings.model-try-inserts]
@dataclass(frozen=True, eq=False, slots=True)
class ModelTryInsert(_Tried[ModelInsert]):
    """A model's INSERT of one row that may write nothing, made by its
    insert's ``on_conflict_do_nothing()`` or ``on_empty_do_nothing()``:
    Rust's ``TryInsert<A, OneRow>``."""

    def returning(self, *fields: str) -> ModelTryRow:
        """A RETURNING list of ``fields``, every field when it names none,
        whose ``one`` answers with the row written, as pgorm's
        ``exec_returning_model`` does."""
        return ModelTryRow(self.insert.returning(*fields), self._empty(), self.insert.conflict)


# [spec:pgorm:req:bindings.model-try-inserts]
@dataclass(frozen=True, eq=False, slots=True)
class ModelTryInsertMany(_Tried[ModelInsertMany]):
    """A model's INSERT of a batch that may write nothing, made by a batch's
    ``on_conflict_do_nothing()`` or ``on_empty_do_nothing()``, or a
    try-insert's ``add``: Rust's ``TryInsert<A, ManyRows>``."""

    def returning(self, *fields: str) -> ModelTryRows:
        """A RETURNING list of ``fields``, every field when it names none,
        whose ``all`` answers with the rows written, as pgorm's
        ``exec_returning_models`` does."""
        return ModelTryRows(self.insert.returning(*fields), self._empty(), self.insert.conflict)


@dataclass(frozen=True, eq=False, slots=True)
class _TriedRows:
    rows: ModelRows
    empty: bool
    conflict: bool

    def inspect(self) -> p.Compiled:
        return self.rows.inspect()

    def __bool__(self) -> bool:
        raise p.ConstructionError("model queries cannot be tested as Python booleans")


@dataclass(frozen=True, eq=False, slots=True)
class ModelTryRow(_TriedRows):
    """A try-insert of one row with a RETURNING list."""

    async def one(
        self, db: Database
    ) -> TryInsertResult.Inserted[ModelRecord] | TryInsertResult.Conflicted | TryInsertResult.Empty:
        """The row written as a record, ``Conflicted`` when the conflict
        clause held it back, or ``Empty`` when it sets no field; with no
        conflict clause, no row is a ``DecodeError``."""
        if self.empty:
            return TryInsertResult.Empty()
        record = await self.rows.optional(db)
        if record is not None:
            return TryInsertResult.Inserted(record)
        if self.conflict:
            return TryInsertResult.Conflicted()
        raise p.DecodeError("expected the inserted row, and the insert returned none")


@dataclass(frozen=True, eq=False, slots=True)
class ModelTryRows(_TriedRows):
    """A try-insert of a batch with a RETURNING list."""

    async def all(
        self, db: Database
    ) -> TryInsertResult.Inserted[list[ModelRecord]] | TryInsertResult.Conflicted | TryInsertResult.Empty:
        """The rows written as records, in the order the database wrote them,
        ``Conflicted`` when the conflict clause held every one back, or
        ``Empty`` for a batch of none."""
        if self.empty:
            return TryInsertResult.Empty()
        records = await self.rows.all(db)
        if not records and self.conflict:
            return TryInsertResult.Conflicted()
        return TryInsertResult.Inserted(records)
