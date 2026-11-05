"""Typed views over the writes that return a row before and after it."""

from typing import Generic, TypeVar

from .. import _native as native_types

M = TypeVar("M")


class Change(Generic[M]):
    """A row before a write and as the write left it, each a typed model."""

    __slots__ = ("old", "new")

    def __init__(self, old: M, new: M):
        self.old, self.new = old, new


class Inserted(Generic[M]):
    """An upsert's row that was not there, and that the insert wrote."""

    __slots__ = ("model",)
    __match_args__ = ("model",)

    def __init__(self, model: M):
        self.model = model


class Updated(Generic[M]):
    """An upsert's row whose ON CONFLICT DO UPDATE updated the row it met."""

    __slots__ = ("change",)
    __match_args__ = ("change",)

    def __init__(self, change: Change[M]):
        self.change = change


def _upserted(row, model):
    if isinstance(row, native_types.Upserted.Inserted):
        return Inserted(model(row.model))
    return Updated(Change(model(row.change.old), model(row.change.new)))


class UpdateView(Generic[M]):
    __slots__ = ("_update", "_model_class")

    def __init__(self, update: native_types.EntityUpdate, model: type[M]):
        self._update, self._model_class = update, model

    async def returning_change(self, connection) -> Change[M]:
        change = await self._update.returning_change(connection)
        return Change(self._model_class(change.old), self._model_class(change.new))

    # [spec:pgorm:req:python.truth-testing+4]
    def __bool__(self) -> bool:
        return bool(self._update)


class UpdateManyView(Generic[M]):
    __slots__ = ("_update", "_model_class")

    def __init__(self, update: native_types.EntityUpdateMany, model: type[M]):
        self._update, self._model_class = update, model

    def set(self, column, value) -> "UpdateManyView[M]":
        return UpdateManyView(self._update.set(column, value), self._model_class)

    def where(self, predicate) -> "UpdateManyView[M]":
        return UpdateManyView(self._update.where(predicate), self._model_class)

    def all_rows(self) -> "UpdateManyView[M]":
        return UpdateManyView(self._update.all_rows(), self._model_class)

    async def returning_changes(self, connection) -> list[Change[M]]:
        changes = await self._update.returning_changes(connection)
        model = self._model_class
        return [Change(model(change.old), model(change.new)) for change in changes]

    def __bool__(self) -> bool:
        return bool(self._update)


class InsertView(Generic[M]):
    __slots__ = ("_insert", "_model_class")

    def __init__(self, insert: native_types.EntityInsert, model: type[M]):
        self._insert, self._model_class = insert, model

    def on_conflict(self, action) -> "InsertView[M]":
        return InsertView(self._insert.on_conflict(action), self._model_class)

    async def returning_upsert(self, connection):
        row = await self._insert.returning_upsert(connection)
        return None if row is None else _upserted(row, self._model_class)

    def __bool__(self) -> bool:
        return bool(self._insert)


class InsertManyView(Generic[M]):
    __slots__ = ("_insert", "_model_class")

    def __init__(self, insert: native_types.EntityInsertMany, model: type[M]):
        self._insert, self._model_class = insert, model

    def on_conflict(self, action) -> "InsertManyView[M]":
        return InsertManyView(self._insert.on_conflict(action), self._model_class)

    async def returning_upserts(self, connection):
        rows = await self._insert.returning_upserts(connection)
        return [_upserted(row, self._model_class) for row in rows]

    def __bool__(self) -> bool:
        return bool(self._insert)
