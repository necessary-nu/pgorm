"""Owned Python views used by generated concrete registration modules."""

from collections.abc import Mapping
from typing import Any, Generic, TypeVar

from .. import _native as native_types
from .raw import RawQueryView
from .versions import (
    InsertManyView,
    InsertView,
    UpdateManyView,
    UpdateView,
)

M = TypeVar("M", bound="ModelView")
A = TypeVar("A", bound="ActiveView")


class ModelView(Mapping[str, Any], Generic[A]):
    __slots__ = ("_model",)
    _entity_name: str
    _active_class: type[A]

    def __init__(self, model: native_types.EntityModel):
        if not isinstance(model, native_types.EntityModel):
            raise TypeError("model view requires a native EntityModel")
        if model.entity_name != self._entity_name:
            raise TypeError(f"the model is {model.entity_name}'s, not {self._entity_name}'s")
        self._model = model

    @property
    def native(self):
        return self._model

    @property
    def entity_name(self) -> str:
        return self._model.entity_name

    def __getitem__(self, key: str) -> Any:
        return self._model[key]

    def __iter__(self):
        return iter(self._model)

    def __len__(self) -> int:
        return len(self._model)

    def tagged(self, key: str) -> native_types.Value:
        return self._model.tagged(key)

    def with_value(self: M, column, value) -> M:
        return type(self)(self._model.with_value(column, value))

    def into_active(self) -> A:
        return self._active_class(self._model.into_active())


class ActiveView(Generic[M]):
    __slots__ = ("_active",)
    _entity_name: str
    _model_class: type[M]

    def __init__(self, active: native_types.ActiveModel):
        if not isinstance(active, native_types.ActiveModel):
            raise TypeError("active view requires a native ActiveModel")
        if active.entity_name != self._entity_name:
            raise TypeError(f"the ActiveModel is {active.entity_name}'s, not {self._entity_name}'s")
        self._active = active

    @property
    def native(self):
        return self._active

    @property
    def entity_name(self) -> str:
        return self._active.entity_name

    def get(self, column):
        return self._active.get(column)

    def set(self: A, column, value) -> A:
        return type(self)(self._active.set(column, value))

    def not_set(self: A, column) -> A:
        return type(self)(self._active.not_set(column))

    def reset(self: A, column) -> A:
        return type(self)(self._active.reset(column))

    def set_if_not_equals(self: A, column, value) -> A:
        return type(self)(self._active.set_if_not_equals(column, value))

    def is_changed(self) -> bool:
        return self._active.is_changed()

    async def insert(self, connection) -> M:
        return self._model_class(await self._active.insert(connection))

    async def update(self, connection) -> M:
        return self._model_class(await self._active.update(connection))

    async def delete(self, connection) -> int:
        return await self._active.delete(connection)


class EntityView(Generic[M, A]):
    __slots__ = ("_entity", "_model_class", "_active_class")

    def __init__(self, entity: native_types.Entity, model: type[M], active: type[A]):
        if entity.name != model._entity_name or entity.name != active._entity_name:
            raise TypeError("generated wrapper types belong to another registration")
        self._entity, self._model_class, self._active_class = entity, model, active

    @property
    def name(self) -> str:
        return self._entity.name

    def describe(self):
        return self._entity.describe()

    def col(self, name):
        return self._entity.col(name)

    def relation(self, name):
        return self._entity.relation(name)

    def find(self) -> "QueryView[M]":
        return QueryView(self._entity.find(), self._model_class)

    def active(self) -> A:
        return self._active_class(self._entity.active())

    def update(self, active) -> "UpdateView[M]":
        return UpdateView(self._entity.update(_native_active(active)), self._model_class)

    def update_many(self) -> "UpdateManyView[M]":
        return UpdateManyView(self._entity.update_many(), self._model_class)

    def insert(self, active) -> "InsertView[M]":
        return InsertView(self._entity.insert(_native_active(active)), self._model_class)

    def insert_many(self, actives) -> "InsertManyView[M]":
        native = [_native_active(active) for active in actives]
        return InsertManyView(self._entity.insert_many(native), self._model_class)


def _native_active(active):
    return active.native if isinstance(active, ActiveView) else active


class QueryView(Generic[M]):
    __slots__ = ("_query", "_model_class")

    def __init__(self, query: native_types.EntityQuery, model: type[M]):
        if query.entity_name != model._entity_name:
            raise TypeError(f"the query is {query.entity_name}'s, not {model._entity_name}'s")
        self._query, self._model_class = query, model

    def where(self, predicate) -> "QueryView[M]":
        return QueryView(self._query.where(predicate), self._model_class)

    def order_by(self, *ordering) -> "QueryView[M]":
        return QueryView(self._query.order_by(*ordering), self._model_class)

    def limit(self, value=None) -> "QueryView[M]":
        return QueryView(self._query.limit(value), self._model_class)

    def offset(self, value=None) -> "QueryView[M]":
        return QueryView(self._query.offset(value), self._model_class)

    def inspect(self, *, terminal="all"):
        return self._query.inspect(terminal=terminal)

    async def all(self, connection) -> list[M]:
        return [self._model_class(row) for row in await self._query.all(connection)]

    async def one(self, connection) -> M:
        return self._model_class(await self._query.one(connection))

    async def optional(self, connection) -> M | None:
        row = await self._query.optional(connection)
        return None if row is None else self._model_class(row)

    def from_raw_sql(self, statement) -> "RawQueryView[M]":
        return RawQueryView(self._query.from_raw_sql(statement), self._model_class)

    def join(self, relation, *, kind=native_types.Join.Inner) -> "QueryView[M]":
        return QueryView(self._query.join(relation, kind=kind), self._model_class)

    def select(self, *columns):
        return self._query.select(*columns)

    async def count(self, connection) -> int:
        return await self._query.count(connection)

    async def stream(self, connection):
        return (await self._query.stream(connection))._decoding(self._model_class)

    def paginate(self, page_size):
        from .reads import PaginatorView

        return PaginatorView(self._query.paginate(page_size), self._model_class)

    def cursor(self, *columns):
        from .reads import CursorView

        return CursorView(self._query.cursor(*columns), self._model_class)

    def __bool__(self) -> bool:
        return bool(self._query)
