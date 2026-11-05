"""Keyset cursors and pages over a model's query or a graph, as pgorm's
``Cursor`` and ``Paginator`` read them: a cursor orders by its columns and
tiebreaks and resumes past or short of a boundary row's key; a paginator
reads a page at a time by limit and offset and counts the rows it pages."""

from __future__ import annotations

from collections.abc import AsyncIterator, Callable
from dataclasses import dataclass, replace
from typing import TYPE_CHECKING, Any

from .. import _native as p
from .record import Database, Decode, Row, decode_all

if TYPE_CHECKING:
    from .model import Model

# One column of a cursor's key: its model, the name its table answers to, and
# the field.
KeyColumn = tuple["Model", str, str]

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
    replaces; how its rows decode; its order columns; the tiebreaks after
    them; and each model the query reads, with the name its table answers
    to, which a secondary ordering may name."""

    build: Callable[
        [tuple[p.OrderBy, ...], tuple[p.Expr | p.Condition, ...], int | None], p.Select
    ]
    limit: int | None
    sources: Decode
    order: tuple[KeyColumn, ...]
    tiebreaks: tuple[KeyColumn, ...]
    tables: tuple[tuple[Model, str], ...]


@dataclass(frozen=True, slots=True)
class Position:
    after: tuple[p.Value, ...] | None = None
    before: tuple[p.Value, ...] | None = None
    last: bool = False
    rows: int | None = None
    ascending: bool = True


# [spec:pgorm:req:bindings.cursors]
@dataclass(frozen=True, eq=False, slots=True)
class Cursor:
    """A keyset cursor. Every method returns a new cursor; ``all`` reads the
    window it describes, in the cursor's order."""

    keyed: Keyed
    position: Position = Position()

    def _keyset(self) -> tuple[KeyColumn, ...]:
        return (*self.keyed.order, *self.keyed.tiebreaks)

    def _boundary(self, values: tuple[Any, ...], extended: bool) -> tuple[p.Value, ...]:
        keyset = self._keyset()
        primary = len(self.keyed.order)
        if len(values) != primary and not (extended and len(values) == len(keyset)):
            expected = f"{primary} or {len(keyset)}" if extended and len(keyset) > primary else f"{primary}"
            raise p.ConstructionError(
                f"a cursor boundary of {len(values)} values does not match {expected} key column(s)"
            )
        return tuple(
            model.columns[field]._value(value)
            for (model, _, field), value in zip(keyset, values)
        )

    # [spec:pgorm:req:bindings.cursor-secondary-order]
    def add_secondary_order_by(self, model: Model, field: str) -> Cursor:
        """The cursor ordered by ``field`` of ``model`` after its key and the
        entries before it: a column of the queried model or a table it
        joined, ``model`` named as the query names it, its alias included, as
        Rust's ``add_secondary_order_by(table, column)``. ``after`` and
        ``before`` still take the key alone, so they skip every row sharing
        the boundary's key; ``after_with`` and ``before_with`` take the whole
        ordering and resume from exactly one row."""
        from .model import Model
        from .relations import same_table

        if not isinstance(model, Model):
            raise TypeError("a secondary ordering names a Model and one of its fields")
        name = model.col(field).field_name
        read = any(
            same_table(table, model) and qualifier == model.qualifier
            for table, qualifier in self.keyed.tables
        )
        if not read:
            raise p.ConstructionError(
                f"the cursor's query reads no table named {model.qualifier}"
            )
        keyed = self.keyed
        entry = (model, model.qualifier, name)
        return replace(self, keyed=replace(keyed, tiebreaks=(*keyed.tiebreaks, entry)))

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
        columns = [p.col(model._sql(field), table=q) for model, q, field in self._keyset()]
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
            for column in (p.col(model._sql(field), table=q) for model, q, field in self._keyset())
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
    rows it counts, and how each row decodes."""

    page: Callable[[int, int], p.Select]
    counted: p.Select
    sources: Decode


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
        return await decode_all(db, self._page(page), self.pageable.sources)

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

