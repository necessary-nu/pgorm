"""What a disjoint model's statements run and are read through: the reads,
their pages and cursor, and a write's terminal.

Every statement is the native shape's; ``_Disjoint`` is what a model and a
registration share, the shape and the view each native row is read through.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from collections.abc import AsyncIterator, Mapping, Sequence
from dataclasses import dataclass, replace
from typing import TYPE_CHECKING, Any, Generic, Literal, TypeVar, cast

from .. import _native as p
from .queries import orderings_of, predicate_of
from .record import Database

if TYPE_CHECKING:
    from ..results import RowStream
    from ..runtime import Connection, Pool

# A row of a disjoint read: the base's record, the kind and the kind's record
# or None, typed by a model or a registration, or by a generated module as a
# union over the kinds.
# [spec:pgorm:req:bindings.codegen-disjoint]
T = TypeVar("T")

# PostgreSQL reads LIMIT and OFFSET as bigint.
ROWS_MAX = 2**63 - 1


def _rows(value: object, what: str) -> int:
    if type(value) is not int:
        raise TypeError(f"{what} is an int")
    if not 0 <= value <= ROWS_MAX:
        raise p.ConstructionError(f"{what} is a non-negative int within bigint")
    return value


class _Disjoint(ABC, Generic[T]):
    """What a disjoint model and a registered one share: the native shape
    and codec, and the view each native row is read through."""

    __slots__ = ("_native",)

    _native: p._Disjoint

    @abstractmethod
    def _row(self, item: Any) -> T:
        """A native read's row as ``(base record, kind, kind record or
        None)``, each record as the model, registration or generated module
        views it."""

    @abstractmethod
    def _key(self, key: Mapping[str, Any]) -> list[Any]:
        """The values of ``key``, in the base's key order, as the native read
        takes them."""

    @abstractmethod
    def _columns(self, fields: tuple[str, ...]) -> list[str]:
        """The base's columns a cursor ordered by ``fields`` orders by."""

    @abstractmethod
    def _boundary(self, fields: tuple[str, ...], values: tuple[Any, ...]) -> list[Any]:
        """A cursor boundary's ``values`` for ``fields``, as the native cursor
        binds them."""

    @property
    def name(self) -> str:
        return cast(str, self._native.describe()["name"])

    # [spec:pgorm:req:bindings.disjoint+2]
    def describe(self) -> dict[str, Any]:
        """The declaration as data: its ``name``, its ``base`` model's name,
        its ``kind_field``, its ``kind_type``'s ``schema`` and ``name``, and its
        ``kinds``, each a ``value`` and its ``model``'s name or ``None``."""
        return self._native.describe()

    def find(self) -> DisjointQuery[T]:
        """Every base row with its kind's row, in one statement."""
        return DisjointQuery(self, self._native.find())

    def find_by_key(self, key: Mapping[str, Any]) -> DisjointQuery[T]:
        """The base row of the key ``key`` gives, every key part and no other
        named."""
        return DisjointQuery(self, self._native.narrow_key(self._native.find(), self._key(key)))



@dataclass(frozen=True, eq=False, slots=True)
class DisjointQuery(Generic[T]):
    """A read of a disjoint model: each base row with its kind's row, one row
    of the statement each, given as ``(base record, kind, kind record or
    None)``."""

    owner: _Disjoint[T]
    select: p.Select

    def where(self, predicate: p.Expr | p.Condition) -> DisjointQuery[T]:
        """Rows passing ``predicate``, an ordinary ``WHERE``: a kind table's
        column is ``NULL`` in every row of another kind."""
        return replace(self, select=self.select.where(predicate_of(predicate)))

    def order_by(self, *orderings: p.OrderBy) -> DisjointQuery[T]:
        return replace(self, select=self.select.order_by(*orderings_of(orderings)))

    def limit(self, rows: int | None) -> DisjointQuery[T]:
        return replace(self, select=self.select.limit(None if rows is None else _rows(rows, "a limit")))

    def offset(self, rows: int | None) -> DisjointQuery[T]:
        return replace(self, select=self.select.offset(None if rows is None else _rows(rows, "an offset")))

    def kind(self, kind: str) -> DisjointQuery[T]:
        """Only base rows of the kind ``kind``."""
        return replace(self, select=self.owner._native.narrow_kind(self.select, kind))

    def kinds(self, kinds: Sequence[str]) -> DisjointQuery[T]:
        """Only base rows of one of ``kinds``."""
        if not isinstance(kinds, (list, tuple)):
            raise TypeError("kinds are a list or tuple of kinds' values")
        return replace(self, select=self.owner._native.narrow_kinds(self.select, list(kinds)))

    def filter_kind(self, kind: str, predicate: p.Expr | p.Condition) -> DisjointQuery[T]:
        """Only base rows of the kind ``kind`` whose row passes
        ``predicate``: the kind is part of the condition, so one a missing row
        satisfies reads no row of another kind."""
        return replace(
            self,
            select=self.owner._native.narrow_filter_kind(self.select, kind, predicate_of(predicate)),
        )

    def inspect(self, *, terminal: Literal["all", "one"] = "all") -> p.Compiled:
        """The SQL and values ``all`` or ``one`` sends."""
        return self.owner._native.inspect(self.select, terminal=terminal)

    async def all(self, db: Database) -> list[T]:
        """Every row; a row the constraints do not admit is a ``DecodeError``."""
        items = await self.owner._native.run(db, self.select, "all")
        return [self.owner._row(item) for item in items]

    async def one(self, db: Database) -> T:
        """The first row of the read narrowed to one, a ``DecodeError`` when
        there is none."""
        return self.owner._row(await self.owner._native.run(db, self.select, "one"))

    async def stream(self, db: Database) -> RowStream[T]:
        """The rows as they arrive; a row that does not decode ends the
        stream as its error."""
        from ..results import RowStream
        from ..runtime import Connection, Pool
        from ..transactions import Transaction

        if isinstance(db, Pool):
            connection = await db.acquire()
            try:
                native = await connection._native.stream(self.select, self.owner._native)
            except BaseException:
                await connection.close()
                raise
            stream: RowStream[Any] = RowStream(native, connection, release_connection=True)
        elif isinstance(db, (Connection, Transaction)):
            native = await db._native.stream(self.select, self.owner._native)
            stream = RowStream(native, None, release_connection=False)
        else:
            raise TypeError("a disjoint read's rows stream from a Pool, a Connection or a Transaction")
        return stream._decoding(self.owner._row)

    async def count(self, db: Database) -> int:
        """The base rows the read matches, counted as pgorm's paginator
        counts them."""
        return await self.owner._native.run(db, self.owner._native.counted(self.select), "count")

    def paginate(self, page_size: int) -> DisjointPaginator[T]:
        """Pages of ``page_size`` rows, numbered from zero."""
        return DisjointPaginator(self, page_size)

    def cursor(self, *fields: str) -> DisjointCursor[T]:
        """A keyset cursor ordered by the base's ``fields``, its boundaries
        bound under their columns' casts."""
        if not fields:
            raise p.ConstructionError("a cursor is ordered by at least one field")
        return DisjointCursor(self, fields)

    def __bool__(self) -> bool:
        raise p.ConstructionError("disjoint queries cannot be tested as Python booleans")


@dataclass(frozen=True, eq=False, slots=True)
class DisjointWrite(Generic[T]):
    """An insert, update or change of kind: one statement, its parts joined
    by data-modifying ``WITH`` clauses, which reads back the written row."""

    owner: _Disjoint[T]
    select: p.Select

    def inspect(self) -> p.Compiled:
        return self.owner._native.inspect(self.select, terminal="write")

    async def returning_model(self, db: Database) -> T:
        """Send the write and read back what a read of the written row gives.
        A write that touches no row — a key no row has, a row of another kind,
        a change to the row's own kind — changes nothing and is a
        ``DecodeError``."""
        return self.owner._row(await self.owner._native.run(db, self.select, "write"))

    def __bool__(self) -> bool:
        raise p.ConstructionError("disjoint writes cannot be tested as Python booleans")


@dataclass(frozen=True, eq=False, slots=True)
class DisjointPaginator(Generic[T]):
    """Pages of a disjoint read, each the read with pgorm's paginator's limit
    and offset, replacing its own."""

    query: DisjointQuery[T]
    page_size: int

    def __post_init__(self) -> None:
        if _rows(self.page_size, "a page size") < 1:
            raise p.ConstructionError("a page size is at least 1")

    def _page(self, page: int) -> p.Select:
        native = self.query.owner._native
        return native.page(self.query.select, self.page_size, _rows(page, "a page number"))

    def inspect(self, page: int | str) -> p.Compiled:
        """The SQL and values reading page ``page``, or with ``"count"`` the
        ones counting the rows."""
        native = self.query.owner._native
        if page == "count":
            return native.inspect(native.counted(self.query.select))
        if isinstance(page, str):
            raise TypeError('a paginator inspects a page, or its "count"')
        return native.inspect(self._page(page))

    async def fetch_page(self, db: Database, page: int) -> list[T]:
        items = await self.query.owner._native.run(db, self._page(page), "all")
        return [self.query.owner._row(item) for item in items]

    async def num_items(self, db: Database) -> int:
        return await self.query.count(db)

    async def num_pages(self, db: Database) -> int:
        return (await self.num_items_and_pages(db))[1]

    async def num_items_and_pages(self, db: Database) -> tuple[int, int]:
        items = await self.num_items(db)
        return items, -(-items // self.page_size)

    async def pages(self, db: Database) -> AsyncIterator[list[T]]:
        """Each page in turn from the first, ending at the first empty one."""
        page = 0
        while rows := await self.fetch_page(db, page):
            yield rows
            page += 1

    def __bool__(self) -> bool:
        raise p.ConstructionError("disjoint paginators cannot be tested as Python booleans")


@dataclass(frozen=True, eq=False, slots=True)
class DisjointCursor(Generic[T]):
    """A keyset cursor over a disjoint read, as pgorm's ``Cursor`` pages
    one: ordered by base fields, each base row one row of the read, so the
    order fields are the whole keyset. Every method returns a new cursor."""

    query: DisjointQuery[T]
    fields: tuple[str, ...]
    after_values: tuple[Any, ...] | None = None
    before_values: tuple[Any, ...] | None = None
    window: tuple[bool, int] | None = None
    descending: bool = False

    def _values(self, values: tuple[Any, ...]) -> list[Any]:
        if len(values) != len(self.fields):
            raise p.ConstructionError(
                f"a cursor boundary of {len(values)} values does not match "
                f"{len(self.fields)} order field(s)"
            )
        return self.query.owner._boundary(self.fields, values)

    def after(self, *values: Any) -> DisjointCursor[T]:
        return replace(self, after_values=tuple(self._values(values)))

    def before(self, *values: Any) -> DisjointCursor[T]:
        return replace(self, before_values=tuple(self._values(values)))

    def first(self, rows: int) -> DisjointCursor[T]:
        return replace(self, window=(False, _rows(rows, "a window's rows")))

    def last(self, rows: int) -> DisjointCursor[T]:
        return replace(self, window=(True, _rows(rows, "a window's rows")))

    def asc(self) -> DisjointCursor[T]:
        return replace(self, descending=False)

    def desc(self) -> DisjointCursor[T]:
        return replace(self, descending=True)

    def _statement(self) -> tuple[p.Select, bool]:
        return self.query.owner._native.cursor(
            self.query.select,
            self.query.owner._columns(self.fields),
            after=None if self.after_values is None else list(self.after_values),
            before=None if self.before_values is None else list(self.before_values),
            window=self.window,
            descending=self.descending,
        )

    def inspect(self) -> p.Compiled:
        return self.query.owner._native.inspect(self._statement()[0])

    async def all(self, db: Database) -> list[T]:
        """The window's rows in the cursor's order: a ``last`` window is read
        backwards and returned forwards."""
        statement, reversed_ = self._statement()
        items = await self.query.owner._native.run(db, statement, "all")
        rows = [self.query.owner._row(item) for item in items]
        return rows[::-1] if reversed_ else rows

    def __bool__(self) -> bool:
        raise p.ConstructionError("disjoint cursors cannot be tested as Python booleans")
