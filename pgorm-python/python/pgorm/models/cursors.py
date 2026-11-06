"""Keyset cursors and pages over a model's query or a graph, as pgorm's
``Cursor`` and ``Paginator`` read them: a cursor orders by its columns and
tiebreaks and resumes past or short of a boundary row's key; a paginator
reads a page at a time by limit and offset and counts the rows it pages."""

from __future__ import annotations

from collections.abc import AsyncIterator, Callable
from dataclasses import dataclass, replace
from typing import TYPE_CHECKING, Any

from .. import _native as p
from .columns import ModelColumn
from .record import Database, Row, Source, decode_all

if TYPE_CHECKING:
    from ..results import Record

# PostgreSQL reads LIMIT and OFFSET as bigint.
ROWS_MAX = 2**63 - 1


def _rows(value: object, what: str) -> int:
    if type(value) is not int:
        raise TypeError(f"{what} is an int")
    if not 0 <= value <= ROWS_MAX:
        raise p.ConstructionError(f"{what} is a non-negative int within bigint")
    return value


@dataclass(frozen=True, slots=True)
class Keyed:
    """What a cursor pages over: the statement it composes with an ordering,
    further conditions and a limit; the query's own limit, which a window
    replaces; the sources its rows decode as; its order columns; and the
    tiebreaks after them."""

    build: Callable[
        [tuple[p.OrderBy, ...], tuple[p.Expr | p.Condition, ...], int | None], p.Select
    ]
    limit: int | None
    sources: tuple[Source, ...]
    order: tuple[ModelColumn, ...]
    tiebreaks: tuple[ModelColumn, ...]


@dataclass(frozen=True, slots=True)
class Position:
    after: tuple[p.Value, ...] | None = None
    before: tuple[p.Value, ...] | None = None
    last: bool = False
    rows: int | None = None
    ascending: bool = True


# [spec:pgorm:req:bindings.cursors+1]
@dataclass(frozen=True, eq=False, slots=True)
class Cursor:
    """A keyset cursor. Every method returns a new cursor; ``all`` reads the
    window it describes, in the cursor's order."""

    keyed: Keyed
    position: Position = Position()

    def _keyset(self) -> tuple[ModelColumn, ...]:
        return (*self.keyed.order, *self.keyed.tiebreaks)

    def _boundary(self, values: tuple[Any, ...], extended: bool) -> tuple[p.Value, ...]:
        keyset = self._keyset()
        primary = len(self.keyed.order)
        if len(values) != primary and not (extended and len(values) == len(keyset)):
            expected = f"{primary} or {len(keyset)}" if extended and len(keyset) > primary else f"{primary}"
            raise p.ConstructionError(
                f"a cursor boundary of {len(values)} values does not match {expected} key column(s)"
            )
        return tuple(column.declaration._value(value) for column, value in zip(keyset, values))

    def after(self, *values: Any) -> Cursor:
        """Rows past the one whose order-field values these are."""
        return replace(self, position=replace(self.position, after=self._boundary(values, False)))

    def before(self, *values: Any) -> Cursor:
        """Rows short of the one whose order-field values these are."""
        return replace(self, position=replace(self.position, before=self._boundary(values, False)))

    def after_with(self, *values: Any) -> Cursor:
        """Rows past the one whose order fields' or whole key's values these
        are, so a page that ended inside a run of equal order values resumes
        inside it."""
        return replace(self, position=replace(self.position, after=self._boundary(values, True)))

    def before_with(self, *values: Any) -> Cursor:
        return replace(self, position=replace(self.position, before=self._boundary(values, True)))

    # [spec:pgorm:req:bindings.cursor-secondary-order]
    def add_secondary_order_by(self, column: ModelColumn) -> Cursor:
        """The cursor with ``column``, a model's column qualified as the
        query names its table, appended to its ordering after the key, as
        pgorm's ``add_secondary_order_by`` appends one: ``after_with`` and
        ``before_with`` then take its value after the others', while
        ``after`` and ``before`` still take the order fields' alone."""
        if not isinstance(column, ModelColumn):
            raise TypeError("a cursor's secondary ordering is a model's column")
        keyed = self.keyed
        return replace(self, keyed=replace(keyed, tiebreaks=(*keyed.tiebreaks, column)))

    def first(self, rows: int) -> Cursor:
        """The first ``rows`` rows in the cursor's order, replacing any window."""
        return replace(self, position=replace(self.position, last=False, rows=_rows(rows, "a window's rows")))

    def last(self, rows: int) -> Cursor:
        """The last ``rows`` rows in the cursor's order, replacing any window."""
        return replace(self, position=replace(self.position, last=True, rows=_rows(rows, "a window's rows")))

    def asc(self) -> Cursor:
        return replace(self, position=replace(self.position, ascending=True))

    def desc(self) -> Cursor:
        return replace(self, position=replace(self.position, ascending=False))

    def _compared(self, values: tuple[p.Value, ...], past: bool) -> p.Condition:
        """The key compared with ``values``, ``(c1, .., cn) > (v1, .., vn)``
        written out as n disjuncts, the k-th holding the first k-1 columns
        equal and comparing the k-th."""
        columns = [column.expr() for column in self._keyset()]
        disjuncts = [
            p.Condition.all(
                *(
                    column == value
                    if index + 1 < reach
                    else (column > value if past else column < value)
                    for index, (column, value) in enumerate(zip(columns[:reach], values))
                )
            )
            for reach in range(len(values), 0, -1)
        ]
        return p.Condition.any(*disjuncts)

    @property
    def statement(self) -> p.Select:
        """The SELECT the cursor runs: window, order and boundaries applied."""
        position, keyed = self.position, self.keyed
        descending = position.ascending == position.last
        orders = tuple(
            (column.desc() if descending else column.asc())
            for column in (key.expr() for key in self._keyset())
        )
        wheres = []
        if position.after is not None:
            wheres.append(self._compared(position.after, position.ascending))
        if position.before is not None:
            wheres.append(self._compared(position.before, not position.ascending))
        limit = keyed.limit if position.rows is None else position.rows
        return keyed.build(orders, tuple(wheres), limit)

    def inspect(self) -> p.Compiled:
        return self.statement.inspect()

    async def all(self, db: Database) -> list[Row]:
        """The window's rows in the cursor's order: a ``last`` window is read
        backwards and returned forwards."""
        rows = await decode_all(db, self.statement, self.keyed.sources)
        return rows[::-1] if self.position.last else rows

    def __bool__(self) -> bool:
        raise p.ConstructionError("model cursors cannot be tested as Python booleans")


@dataclass(frozen=True, slots=True)
class Pageable:
    """What a paginator pages: the statement of one page, the statement whose
    rows it counts, and the sources each row decodes as, or with ``decode``
    what decodes each row in their place, a raw statement's by name, and with
    neither the rows as they are."""

    page: Callable[[int, int], p.Select]
    counted: p.Select
    sources: tuple[Source, ...]
    decode: Callable[[Record], Row] | None = None


# [spec:pgorm:req:bindings.pagination]
@dataclass(frozen=True, eq=False, slots=True)
class Paginator:
    """Pages of a query's rows, ``page_size`` at a time, numbered from zero:
    each a limit and offset over the query, which should be ordered for its
    pages to be stable."""

    pageable: Pageable
    page_size: int

    def __post_init__(self) -> None:
        if _rows(self.page_size, "a page size") < 1:
            raise p.ConstructionError("a page size is at least 1")

    def _page(self, page: int) -> p.Select:
        offset = self.page_size * _rows(page, "a page number")
        if offset > ROWS_MAX:
            raise p.ConstructionError(f"page {page} of {self.page_size} rows is past the offsets bigint holds")
        return self.pageable.page(self.page_size, offset)

    def inspect(self, page: int | str) -> p.Compiled:
        """The SQL and values reading page ``page``, or with ``"count"`` the
        ones counting the rows."""
        if page == "count":
            return self.pageable.counted.inspect()
        if isinstance(page, str):
            raise TypeError('a paginator inspects a page, or its "count"')
        return self._page(page).inspect()

    async def fetch_page(self, db: Database, page: int) -> list[Row]:
        statement = self._page(page)
        decode = self.pageable.decode
        if decode is not None:
            return [decode(row) for row in await db.query(statement)]
        if not self.pageable.sources:
            return list(await db.query(statement))
        return await decode_all(db, statement, self.pageable.sources)

    async def num_items(self, db: Database) -> int:
        """How many rows the pages hold."""
        return int((await db.one(self.pageable.counted))["num_items"])

    async def num_pages(self, db: Database) -> int:
        return (await self.num_items_and_pages(db))[1]

    async def num_items_and_pages(self, db: Database) -> tuple[int, int]:
        """The rows the pages hold and the pages, the last one rounded up."""
        items = await self.num_items(db)
        return items, -(-items // self.page_size)

    async def pages(self, db: Database) -> AsyncIterator[list[Row]]:
        """Each page in turn from the first, ending at the first empty one."""
        page = 0
        while rows := await self.fetch_page(db, page):
            yield rows
            page += 1

    def __bool__(self) -> bool:
        raise p.ConstructionError("model paginators cannot be tested as Python booleans")

