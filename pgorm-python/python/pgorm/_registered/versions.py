"""Typed views over the writes that return a row before and after it."""

from typing import Generic, TypeVar

from .. import _native as native_types

M = TypeVar("M")


def _upserted(row, model):
    """``row``, a registered entity's upsert, read as the view's model."""
    if isinstance(row, native_types.Upserted.Inserted):
        return native_types._inserted(model(row.model))
    change = row.change
    return native_types._updated(native_types._change(model(change.old), model(change.new)))


class UpdateView(Generic[M]):
    __slots__ = ("_update", "_model_class")

    def __init__(self, update: native_types.EntityUpdate, model: type[M]):
        self._update, self._model_class = update, model

    async def returning_change(self, connection) -> native_types.Change[M]:
        change = await self._update.returning_change(connection)
        return native_types._change(self._model_class(change.old), self._model_class(change.new))

    # [spec:pgorm:req:python.truth-testing+5]
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

    async def returning_changes(self, connection) -> list[native_types.Change[M]]:
        changes = await self._update.returning_changes(connection)
        model = self._model_class
        return [native_types._change(model(change.old), model(change.new)) for change in changes]

    def __bool__(self) -> bool:
        return bool(self._update)


def _native_actives(actives):
    from .entities import _native_active

    return [_native_active(active) for active in actives]


class _InsertView(Generic[M]):
    """What a typed insert of one row and of a batch share: the native insert,
    the model class each written row is read as, and the terminals both kinds
    have."""

    __slots__ = ("_insert", "_model_class")

    def __init__(self, insert, model: type[M]):
        self._insert, self._model_class = insert, model

    def add(self, active) -> "InsertManyView[M]":
        return InsertManyView(self._insert.add_many(_native_actives([active])), self._model_class)

    def add_many(self, actives) -> "InsertManyView[M]":
        return InsertManyView(self._insert.add_many(_native_actives(actives)), self._model_class)

    async def execute(self, connection) -> int:
        return await self._insert.execute(connection)

    def __bool__(self) -> bool:
        return bool(self._insert)


class InsertView(_InsertView[M]):
    __slots__ = ()

    def __init__(self, insert: native_types.EntityInsert, model: type[M]):
        super().__init__(insert, model)

    def on_conflict(self, action) -> "InsertView[M]":
        return InsertView(self._insert.on_conflict(action), self._model_class)

    async def returning_pk(self, connection):
        return await self._insert.returning_pk(connection)

    async def returning_model(self, connection) -> M:
        return self._model_class(await self._insert.returning_model(connection))

    async def returning_upsert(self, connection):
        row = await self._insert.returning_upsert(connection)
        return None if row is None else _upserted(row, self._model_class)


class InsertManyView(_InsertView[M]):
    __slots__ = ()

    def __init__(self, insert: native_types.EntityInsertMany, model: type[M]):
        super().__init__(insert, model)

    def on_conflict(self, action) -> "InsertManyView[M]":
        return InsertManyView(self._insert.on_conflict(action), self._model_class)

    async def returning_pks(self, connection) -> list:
        return await self._insert.returning_pks(connection)

    async def returning_models(self, connection) -> list[M]:
        model = self._model_class
        return [model(row) for row in await self._insert.returning_models(connection)]

    async def returning_upserts(self, connection):
        rows = await self._insert.returning_upserts(connection)
        return [_upserted(row, self._model_class) for row in rows]
