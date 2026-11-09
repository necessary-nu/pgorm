"""Generated graph views retain the native tuple's model and optional slots."""

from typing import Callable, Generic, TypeVar
from .. import _native as native
from .entities import ModelView

R = TypeVar("R")


def wrap_row(row, models: tuple[type[ModelView], ...], optional: tuple[bool, ...]):
    if len(models) == 1:
        return models[0](row)
    if not isinstance(row, tuple) or len(row) != len(models):
        raise native.InternalError(
            "native graph row differs from the generated tuple shape"
        )
    result = []
    for value, model, may_be_absent in zip(row, models, optional, strict=True):
        if value is None:
            if not may_be_absent:
                raise native.InternalError("required native graph source is absent")
            result.append(None)
        else:
            result.append(model(value))
    return tuple(result)


class GraphView(Generic[R]):
    __slots__ = ("_graph", "_decode")

    def __init__(self, graph: native.EntityGraph, decode: Callable[..., R]):
        self._graph, self._decode = graph, decode

    @property
    def name(self) -> str:
        return self._graph.name

    def describe(self):
        return self._graph.describe()

    def find(self, *, aliases=None) -> "GraphQueryView[R]":
        return GraphQueryView(self._graph.find(aliases=aliases), self._decode)


class GraphQueryView(Generic[R]):
    __slots__ = ("_query", "_decode")

    def __init__(self, query: native.EntityGraphQuery, decode: Callable[..., R]):
        self._query, self._decode = query, decode

    def col(self, source, column):
        return self._query.col(source, column)

    def where(self, predicate) -> "GraphQueryView[R]":
        return GraphQueryView(self._query.where(predicate), self._decode)

    def order_by(self, *ordering) -> "GraphQueryView[R]":
        return GraphQueryView(self._query.order_by(*ordering), self._decode)

    def inspect(self, *, terminal="all"):
        return self._query.inspect(terminal=terminal)

    def cursor(self, *columns) -> "GraphCursorView[R]":
        return GraphCursorView(self._query.cursor(*columns), self._decode)

    def cursor_on(self, source, *columns) -> "GraphCursorView[R]":
        return GraphCursorView(self._query.cursor_on(source, *columns), self._decode)

    async def count(self, connection) -> int:
        return await self._query.count(connection)

    def paginate(self, page_size):
        from .reads import PaginatorView

        return PaginatorView(self._query.paginate(page_size), self._decode)

    async def stream(self, connection):
        return (await self._query.stream(connection))._decoding(self._decode)

    async def all_grouped(self, connection):
        """Each root with its slot's models, decoded as the generated row
        type decodes the root with no slot and the root with each slot."""
        grouped = []
        for root, slots in await self._query.all_grouped(connection):
            decoded_root = self._decode((root, None))[0]
            grouped.append((decoded_root, [self._decode((root, slot))[1] for slot in slots]))
        return grouped

    async def all(self, connection) -> list[R]:
        return [self._decode(row) for row in await self._query.all(connection)]

    async def optional(self, connection) -> R | None:
        row = await self._query.optional(connection)
        return None if row is None else self._decode(row)

    def __bool__(self) -> bool:
        return bool(self._query)


class GraphCursorView(Generic[R]):
    __slots__ = ("_cursor", "_decode")

    def __init__(self, cursor: native.EntityGraphCursor, decode: Callable[..., R]):
        self._cursor, self._decode = cursor, decode

    def before(self, *values) -> "GraphCursorView[R]":
        return GraphCursorView(self._cursor.before(*values), self._decode)

    def after(self, *values) -> "GraphCursorView[R]":
        return GraphCursorView(self._cursor.after(*values), self._decode)

    def before_with(self, *values) -> "GraphCursorView[R]":
        return GraphCursorView(self._cursor.before_with(*values), self._decode)

    def after_with(self, *values) -> "GraphCursorView[R]":
        return GraphCursorView(self._cursor.after_with(*values), self._decode)

    def first(self, count) -> "GraphCursorView[R]":
        return GraphCursorView(self._cursor.first(count), self._decode)

    def last(self, count) -> "GraphCursorView[R]":
        return GraphCursorView(self._cursor.last(count), self._decode)

    def asc(self) -> "GraphCursorView[R]":
        return GraphCursorView(self._cursor.asc(), self._decode)

    def desc(self) -> "GraphCursorView[R]":
        return GraphCursorView(self._cursor.desc(), self._decode)

    def inspect(self):
        return self._cursor.inspect()

    async def all(self, connection) -> list[R]:
        return [self._decode(row) for row in await self._cursor.all(connection)]

    def __bool__(self) -> bool:
        return bool(self._cursor)
