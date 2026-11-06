"""The generated view of a registered entity's raw read: a statement whose
rows decode into the generated model views, as Rust's ``Select::from_raw_sql``
decodes them."""

from typing import Any, Generic, TypeVar

from .. import _native as native_types

M = TypeVar("M", bound=Any)


class RawQueryView(Generic[M]):
    """A raw statement whose rows decode into the registration's models, as
    Rust's ``Select::from_raw_sql`` decodes them."""

    __slots__ = ("_query", "_model_class")

    def __init__(self, query: native_types.EntityRawQuery, model: type[M]):
        if query.entity_name != model._entity_name:
            raise TypeError(f"the query is {query.entity_name}'s, not {model._entity_name}'s")
        self._query, self._model_class = query, model

    def inspect(self):
        return self._query.inspect()

    async def all(self, connection) -> list[M]:
        return [self._model_class(row) for row in await self._query.all(connection)]

    async def one(self, connection) -> M:
        return self._model_class(await self._query.one(connection))

    async def optional(self, connection) -> M | None:
        row = await self._query.optional(connection)
        return None if row is None else self._model_class(row)

    def __bool__(self) -> bool:
        return bool(self._query)
