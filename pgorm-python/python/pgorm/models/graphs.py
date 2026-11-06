"""A model's graph: its rows with the rows its relations reach, each joined
source decoded as a record of its own, as pgorm's ``SelectGraph`` reads them.

The slot kind is the join type is the decode shape: ``join_one`` is an INNER
JOIN decoded as a record, ``join_maybe`` a LEFT JOIN decoded as a record or
None, and ``via`` a LEFT JOIN through a table no record reads. Every decoded
source is projected under its own prefix, ``s0_`` for the root and ``s{i}_``
for the i-th slot, by the composition pgorm's graph writer uses."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, replace
from typing import TYPE_CHECKING, Any, Generic, Literal, TypeVar

from .. import _native as p
from .clauses import orderings_of, predicate_of
from .record import (
    Database,
    ModelRecord,
    Row,
    Source,
    decode_all,
    decode_count,
    decode_single,
    decode_stream,
)
from .relations import Related, Relation, answering, condition_of, hashable, joined_on, same_table

if TYPE_CHECKING:
    from ..results import RowStream
    from ..runtime import Connection, Pool
    from .columns import ModelColumn
    from .cursors import Cursor, Paginator
    from .model import Model


# Whether ``limit`` or ``offset`` has cut the graph's read to a window of
# rows: False until one is called, True after, whatever value it was given.
S = TypeVar("S", bound=bool, default=Literal[False])

# Why each read that decides the row range itself refuses a sliced graph.
REFUSALS = {
    "all_grouped": "a window of rows can end part-way through a root's rows, and its group would look complete",
    "count": "the count would ignore the window",
    "cursor": "its first and last would replace the window, and an offset would skip rows past each boundary",
    "cursor_on": "its first and last would replace the window, and an offset would skip rows past each boundary",
    "paginate": "its pages would replace the window",
}


@dataclass(frozen=True, slots=True)
class Table:
    """A table the graph reads: the root, a decoded slot, or a hop."""

    model: Model
    qualifier: str
    role: Literal["root", "slot", "hop"]
    optional: bool


@dataclass(frozen=True, slots=True)
class Edge:
    table: p.Table
    on: p.Condition
    kind: p.Join


@dataclass(frozen=True, slots=True)
class State:
    tables: tuple[Table, ...]
    joins: tuple[Edge, ...] = ()
    wheres: tuple[p.Expr | p.Condition, ...] = ()
    orders: tuple[p.OrderBy, ...] = ()
    limit: int | None = None
    offset: int | None = None
    # Set by limit or offset, whatever value they were given: the caller has
    # taken charge of the row range.
    sliced: bool = False

    def decoded(self) -> tuple[Table, ...]:
        return tuple(table for table in self.tables if table.role != "hop")

    def sources(self) -> tuple[Source, ...]:
        return tuple(
            Source(
                table.model,
                tuple(table.model.columns),
                tuple(
                    p._model_result_names(
                        f"s{index}_", [table.model._sql(name) for name in table.model.columns]
                    )
                ),
                table.optional,
            )
            for index, table in enumerate(self.decoded())
        )

    def select(
        self,
        orders: tuple[p.OrderBy, ...] | None = None,
        wheres: tuple[p.Expr | p.Condition, ...] = (),
        limit: int | None = None,
        offset: int | None = None,
    ) -> p.Select:
        """The graph's SELECT, its ordering, further conditions and window
        those given."""
        projection = [
            table.model._projected(
                field, table.model._column(field, table.qualifier), name
            )
            for table, source in zip(self.decoded(), self.sources())
            for field, name in zip(source.fields, source.names)
        ]
        query = p.Select(*projection).from_(self.tables[0].model.table)
        for join in self.joins:
            query = query.join(join.table, join.on, kind=join.kind)
        for where in (*self.wheres, *wheres):
            query = query.where(where)
        ordering = self.orders if orders is None else orders
        if ordering:
            query = query.order_by(*ordering)
        return query.limit(limit).offset(offset)


# [spec:pgorm:req:bindings.graphs+2]
@dataclass(frozen=True, eq=False, slots=True)
class Graph(Generic[S]):
    """A model's rows with the rows its relations reach. Every method returns
    a new graph; the terminals decode each row as the root's record when no
    slot is joined, and otherwise as a tuple of the root's and each slot's.

    ``S`` records whether ``limit`` or ``offset`` has cut the read to a
    window of rows: ``Literal[False]`` until one is called, ``Literal[True]``
    after, which ``all_grouped``, ``count``, ``cursor``, ``cursor_on`` and
    ``paginate`` refuse, as pgorm's ``Sliced`` graph has none of them."""

    state: State

    def __post_init__(self) -> None:
        if not isinstance(self.state, State):
            raise TypeError("a Graph comes from a model's graph()")
        # Building the statement refuses what cannot be built as the graph is made.
        self.statement

    def _edge(
        self,
        relation: Relation,
        role: Literal["slot", "hop"],
        optional: bool,
        alias: str | None,
        from_: int | None,
        extra: Callable[[Any, Any], Any] | None = None,
        kind: p.Join | None = None,
    ) -> Graph[S]:
        if not isinstance(relation, Relation):
            raise TypeError("a graph joins along a Relation")
        state = self.state
        start, end = relation.from_, relation.to
        if relation.start is not None:
            if from_ is not None:
                raise p.ConstructionError(
                    "a relation from an alias names where it starts: it takes no from_"
                )
            left = next((table for table in state.tables if table.qualifier == relation.start), None)
            if left is None:
                raise p.ConstructionError(f"the graph reads no table called {relation.start!r}")
            if not same_table(left.model, start):
                raise p.ConstructionError(
                    f"{relation.start!r} is {left.model.table.name}, where the relation starts at "
                    f"{start.table.name}"
                )
        elif from_ is None:
            left = next((table for table in state.tables if same_table(table.model, start)), None)
        else:
            if type(from_) is not int:
                raise TypeError("a graph's from_ is a decoded source's index")
            decoded = state.decoded()
            left = decoded[from_] if 0 <= from_ < len(decoded) else None
            if left is not None and not same_table(left.model, start):
                raise p.ConstructionError(
                    f"source {from_} is {left.model.table.name}, where the relation starts at {start.table.name}"
                )
        if left is None:
            raise p.ConstructionError(
                f"the relation starts at {start.table.name}, which the graph does not read"
            )
        qualifier = end.qualifier if alias is None else p._identifier(alias)
        if any(table.qualifier == qualifier for table in state.tables):
            raise p.ConstructionError(
                f"the graph already reads a table called {qualifier!r}: give this one an alias"
            )
        table = end.table if alias is None else end.table.as_(alias)
        if kind is None:
            kind = p.Join.Inner if role == "slot" and not optional else p.Join.Left
        on = joined_on(relation, left.qualifier, qualifier)
        if extra is not None:
            narrowed = extra(answering(start, left.qualifier), answering(end, qualifier))
            on = p.Condition.all(on, condition_of(narrowed, "a join's condition"))
        return Graph(
            replace(
                state,
                tables=(*state.tables, Table(end, qualifier, role, optional)),
                joins=(*state.joins, Edge(table, on, kind)),
            )
        )

    def join_one(
        self, relation: Relation, *, alias: str | None = None, from_: int | None = None
    ) -> Graph[S]:
        """A slot that must match: an INNER JOIN of the relation's far end,
        decoded as its record."""
        return self._edge(relation, "slot", False, alias, from_)

    def join_maybe(
        self, relation: Relation, *, alias: str | None = None, from_: int | None = None
    ) -> Graph[S]:
        """A slot that may be absent: a LEFT JOIN of the relation's far end,
        decoded as its record, or None where the join matched nothing."""
        return self._edge(relation, "slot", True, alias, from_)

    def join_maybe_filtered(
        self,
        relation: Relation,
        condition: Callable[[Model, Model], p.Expr | p.Condition],
        *,
        alias: str | None = None,
        from_: int | None = None,
    ) -> Graph[S]:
        """``join_maybe`` with ``condition``, given the two ends as the join
        names them, ANDed into the join's ``ON``, as pgorm's
        ``join_maybe_filtered`` adds it: it narrows which rows match, and a
        root with none keeps its row with the slot None."""
        if not callable(condition):
            raise TypeError("a join's condition is a function of the two ends")
        return self._edge(relation, "slot", True, alias, from_, condition)

    def via(
        self, relation: Relation, *, alias: str | None = None, from_: int | None = None
    ) -> Graph[S]:
        """A hop never decoded — a junction table, a step of a chain — LEFT
        JOINed, so a missing middle cannot drop a root row by itself."""
        return self._edge(relation, "hop", True, alias, from_)

    # [spec:pgorm:req:bindings.related]
    def related_one(self, path: Relation | Related) -> Graph[S]:
        """A slot that must match, along ``path``: a relation, as ``join_one``
        joins it, or a related path, its junction INNER JOINed as a hop and
        its far end as the slot, as pgorm's ``related_one`` folds a
        ``Related`` path in."""
        if isinstance(path, Related):
            return self._edge(path.via, "hop", True, None, None, kind=p.Join.Inner).join_one(
                path.relation
            )
        return self.join_one(path)

    def related_maybe(self, path: Relation | Related) -> Graph[S]:
        """A slot that may be absent, along ``path``: a relation, as
        ``join_maybe`` joins it, or a related path, its junction a ``via``
        hop and its far end the slot, as pgorm's ``related_maybe`` folds a
        ``Related`` path in."""
        if isinstance(path, Related):
            return self.via(path.via).join_maybe(path.relation)
        return self.join_maybe(path)

    def col(self, source: int, field: str) -> ModelColumn:
        """A field of decoded source ``source`` — 0 the root, i the i-th slot
        — qualified as that source is named."""
        decoded = self.state.decoded()
        if type(source) is not int:
            raise TypeError("a graph's source is its index")
        if not 0 <= source < len(decoded):
            raise p.ConstructionError(f"the graph has no source {source}")
        table = decoded[source]
        return table.model._column(field, table.qualifier)

    def where(self, predicate: p.Expr | p.Condition) -> Graph[S]:
        state = self.state
        return Graph(replace(state, wheres=(*state.wheres, predicate_of(predicate))))

    def order_by(self, *ordering: p.OrderBy) -> Graph[S]:
        state = self.state
        return Graph(replace(state, orders=(*state.orders, *orderings_of(ordering))))

    # [spec:pgorm:req:bindings.graphs+2]    limit and offset slice the graph whatever their value
    def limit(self, value: int | None) -> Graph[Literal[True]]:
        """Read at most ``value`` rows of the join, not roots, or with None
        no limit; either way the graph is sliced."""
        return Graph(replace(self.state, limit=value, sliced=True))

    def offset(self, value: int | None) -> Graph[Literal[True]]:
        """Skip the first ``value`` rows of the join, or with None none;
        either way the graph is sliced."""
        return Graph(replace(self.state, offset=value, sliced=True))

    def _unsliced(self, terminal: str) -> None:
        """Refuse a read that decides the row range itself on a graph whose
        limit or offset already decided it."""
        if self.state.sliced:
            raise p.ConstructionError(
                f"a graph cut by limit or offset has no {terminal}: {REFUSALS[terminal]}; "
                "read the first roots with find().limit(..) and load their related rows"
            )

    @property
    def statement(self) -> p.Select:
        state = self.state
        return state.select(limit=state.limit, offset=state.offset)

    def inspect(self, *, terminal: str | None = None) -> p.Compiled:
        """The SQL and values the graph runs, or the ones ``count`` or
        ``all_grouped`` runs."""
        if terminal is None:
            return self.statement.inspect()
        if terminal == "count":
            self._unsliced("count")
            return p._model_count(self.statement).inspect()
        if terminal == "all_grouped":
            return self._grouped().inspect()
        raise TypeError('a graph inspects itself, its "count" or its "all_grouped"')

    async def count(self: Graph[Literal[False]], db: Database) -> int:
        """How many rows the graph reads."""
        self._unsliced("count")
        return await decode_count(db, self.statement)

    async def all(self, db: Database) -> list[Row]:
        return await decode_all(db, self.statement, self.state.sources())

    async def one(self, db: Database) -> Row:
        row: Row = await decode_single(db, self.statement, self.state.sources(), optional=False)
        return row

    async def optional(self, db: Database) -> Row | None:
        row: Row | None = await decode_single(db, self.statement, self.state.sources(), optional=True)
        return row

    async def stream(self, db: Database) -> RowStream[Row]:
        return await decode_stream(db, self.statement, self.state.sources())

    def _root_key(self) -> tuple[tuple[Model, str, str], ...]:
        root = self.state.tables[0]
        return tuple((root.model, root.qualifier, field) for field in root.model.primary_key)

    def _grouped(self) -> p.Select:
        """The SELECT ``all_grouped`` runs: the root's key ordered behind the
        graph's ordering."""
        state = self.state
        self._unsliced("all_grouped")
        if len(state.decoded()) != 2:
            raise p.ConstructionError("all_grouped reads a graph of exactly one slot")
        key = self._root_key()
        if not key:
            raise p.ConstructionError(
                "all_grouped groups by the root's primary key, which it has none of"
            )
        orders = (*state.orders, *(model._column(field, q).asc() for model, q, field in key))
        return state.select(orders)

    async def all_grouped(
        self: Graph[Literal[False]], db: Database
    ) -> list[tuple[ModelRecord, list[ModelRecord]]]:
        """Each root with the records its one slot holds for it, as pgorm's
        ``all_grouped`` reads them, in the order roots first appear."""
        rows = await decode_all(db, self._grouped(), self.state.sources())
        groups: dict[Any, tuple[ModelRecord, list[ModelRecord]]] = {}
        for root, slot in rows:
            key = hashable(tuple(root[field] for field in root.model.primary_key))
            group = groups.setdefault(key, (root, []))
            if slot is not None:
                group[1].append(slot)
        return list(groups.values())

    def cursor(self: Graph[Literal[False]], *fields: str) -> Cursor:
        """A keyset cursor ordered by ``fields`` of the root, with the root's
        primary key and each slot's as tiebreaks behind them, in declaration
        order, so a join that repeats a root row still has a total order."""
        self._unsliced("cursor")
        return self._cursor(0, fields)

    # [spec:pgorm:req:bindings.cursors+1]
    def cursor_on(self: Graph[Literal[False]], source: int, *fields: str) -> Cursor:
        """A keyset cursor ordered by ``fields`` of slot ``source``, the
        first slot ``1``, as pgorm's ``cursor_by_on`` orders: the slot's
        primary key, then the root's, then every other slot's, in
        declaration order, are its tiebreaks."""
        self._unsliced("cursor_on")
        if type(source) is not int:
            raise TypeError("a graph's source is its index")
        if source == 0:
            raise p.ConstructionError(
                "cursor_on orders by a slot's fields, the first slot 1; cursor orders by the root's"
            )
        return self._cursor(source, fields)

    def _cursor(self, source: int, fields: tuple[str, ...]) -> Cursor:
        from .cursors import Cursor, Keyed

        if not fields:
            raise p.ConstructionError("a cursor orders by at least one field")
        state = self.state
        decoded = state.decoded()
        if not 0 <= source < len(decoded):
            raise p.ConstructionError(f"the graph has no source {source}")
        ordered = decoded[source]
        names = tuple(ordered.model.col(name).field_name for name in fields)
        others = (decoded[0], *(slot for index, slot in enumerate(decoded) if index not in (0, source)))
        tiebreaks = (
            *(
                ordered.model._column(field, ordered.qualifier)
                for field in ordered.model.primary_key
                if field not in names
            ),
            *(
                table.model._column(field, table.qualifier)
                for table in (others if source else others[1:])
                for field in table.model.primary_key
            ),
        )
        return Cursor(
            Keyed(
                lambda orders, wheres, limit: state.select(orders, wheres, limit),
                None,
                state.sources(),
                tuple(ordered.model._column(field, ordered.qualifier) for field in names),
                tiebreaks,
                tuple((table.model, table.qualifier) for table in state.tables),
            )
        )

    def paginate(self: Graph[Literal[False]], page_size: int) -> Paginator:
        """Pages of ``page_size`` rows: a page boundary falls between rows, so a
        root with several slot rows can span pages, as the SQL does."""
        from .cursors import Pageable, Paginator

        self._unsliced("paginate")
        state = self.state
        return Paginator(
            Pageable(
                lambda limit, offset: state.select(limit=limit, offset=offset),
                p._model_count(self.statement),
                state.sources(),
            ),
            page_size,
        )

    def __bool__(self) -> bool:
        raise p.ConstructionError("model graphs cannot be tested as Python booleans")
