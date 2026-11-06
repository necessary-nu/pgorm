"""Generated views of a registered read's pages and keyset cursor, each row
decoded into the generated model views as the native rows arrive."""

from collections.abc import AsyncIterator
from typing import Any, Callable, Generic, TypeVar

from .. import _native as native

R = TypeVar("R")


class PaginatorView(Generic[R]):
    __slots__ = ("_paginator", "_decode")

    def __init__(self, paginator: native.EntityPaginator[Any], decode: Callable[..., R]):
        self._paginator, self._decode = paginator, decode

    async def fetch_page(self, connection, page) -> list[R]:
        return [self._decode(row) for row in await self._paginator.fetch_page(connection, page)]

    async def num_items(self, connection) -> int:
        return await self._paginator.num_items(connection)

    async def num_pages(self, connection) -> int:
        return await self._paginator.num_pages(connection)

    async def num_items_and_pages(self, connection) -> tuple[int, int]:
        return await self._paginator.num_items_and_pages(connection)

    async def pages(self, connection) -> AsyncIterator[list[R]]:
        async for page in self._paginator.pages(connection):
            yield [self._decode(row) for row in page]

    def inspect(self, page):
        return self._paginator.inspect(page)

    def __bool__(self) -> bool:
        return bool(self._paginator)


class CursorView(Generic[R]):
    __slots__ = ("_cursor", "_decode")

    def __init__(self, cursor: native.EntityCursor, decode: Callable[..., R]):
        self._cursor, self._decode = cursor, decode

    def before(self, *values) -> "CursorView[R]":
        return CursorView(self._cursor.before(*values), self._decode)

    def after(self, *values) -> "CursorView[R]":
        return CursorView(self._cursor.after(*values), self._decode)

    def first(self, count) -> "CursorView[R]":
        return CursorView(self._cursor.first(count), self._decode)

    def last(self, count) -> "CursorView[R]":
        return CursorView(self._cursor.last(count), self._decode)

    def asc(self) -> "CursorView[R]":
        return CursorView(self._cursor.asc(), self._decode)

    def desc(self) -> "CursorView[R]":
        return CursorView(self._cursor.desc(), self._decode)

    def inspect(self):
        return self._cursor.inspect()

    async def all(self, connection) -> list[R]:
        return [self._decode(row) for row in await self._cursor.all(connection)]

    def __bool__(self) -> bool:
        return bool(self._cursor)
