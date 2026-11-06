"""Runtime model reads, and the rows a write returns, using native SQL statements."""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import TYPE_CHECKING, cast

from .. import _native as p
from .record import (
    Database,
    ModelRecord,
    Source,
    decode_all,
    decode_count,
    decode_single,
    decode_stream,
    own_source,
)

if TYPE_CHECKING:
    from ..results import RowStream
    from ..runtime import Connection, Pool
    from .cursors import Cursor, Paginator
    from .model import Model
    from .relations import Relation


def predicate_of(predicate: object) -> p.Expr | p.Condition:
    if not isinstance(predicate, (p.Expr, p.Condition)):
        raise TypeError("a condition is an expression or a Condition")
    return predicate


def orderings_of(orderings: tuple[object, ...]) -> tuple[p.OrderBy, ...]:
    checked = tuple(ordering for ordering in orderings if isinstance(ordering, p.OrderBy))
    if len(checked) != len(orderings):
        raise TypeError("an ordering is expr.asc() or expr.desc()")
    return checked


@dataclass(frozen=True, slots=True)
class Joined:
    """A table a model query joins without reading its columns."""

    model: Model
    table: p.Table
    qualifier: str
    on: p.Condition
    kind: p.Join


@dataclass(frozen=True, slots=True)
class Recipe:
    """What a model query reads: its fields, joins, conditions, ordering and
    window, from which each terminal's statement is built."""

    model: Model
    fields: tuple[str, ...]
    joins: tuple[Joined, ...] = ()
    wheres: tuple[p.Expr | p.Condition, ...] = ()
    orders: tuple[p.OrderBy, ...] = ()
    limit: int | None = None
    offset: int | None = None

    def select(
        self,
        orders: tuple[p.OrderBy, ...] | None = None,
        wheres: tuple[p.Expr | p.Condition, ...] = (),
        limit: int | None = None,
    ) -> p.Select:
        """The SELECT, its ordering, further conditions and limit those given."""
        model = self.model
        query = p.Select(
            *(model._projected(name, model.col(name)) for name in self.fields)
        ).from_(model.table)
        for join in self.joins:
            query = query.join(join.table, join.on, kind=join.kind)
        for where in (*self.wheres, *wheres):
            query = query.where(where)
        ordering = self.orders if orders is None else orders
        if ordering:
            query = query.order_by(*ordering)
        return query.limit(limit).offset(self.offset)


# [spec:pgorm:req:bindings.model-reads+1]
@dataclass(frozen=True, eq=False, slots=True)
class ModelQuery:
    """A model's SELECT: every method returns a new query, and the terminals
    decode each row into a record of the fields it selects."""

    recipe: Recipe

    def __post_init__(self) -> None:
        if not isinstance(self.recipe, Recipe):
            raise TypeError("a ModelQuery comes from a model's find() or select()")
        # Building the statement refuses what cannot be built as the query is made.
        self.statement

    @property
    def model(self) -> Model:
        return self.recipe.model

    @property
    def selected(self) -> tuple[str, ...]:
        return self.recipe.fields

    @property
    def statement(self) -> p.Select:
        return self.recipe.select(limit=self.recipe.limit)

    def inspect(self, *, terminal: str | None = None) -> p.Compiled:
        """The SQL and values the query runs, or with ``terminal="count"``
        the ones ``count`` runs."""
        if terminal is None:
            return self.statement.inspect()
        if terminal == "count":
            return p._model_count(self.statement).inspect()
        raise TypeError('a model query inspects itself, or its "count"')

    def where(self, predicate: p.Expr | p.Condition) -> ModelQuery:
        recipe = self.recipe
        return ModelQuery(replace(recipe, wheres=(*recipe.wheres, predicate_of(predicate))))

    def order_by(self, *ordering: p.OrderBy) -> ModelQuery:
        recipe = self.recipe
        return ModelQuery(replace(recipe, orders=(*recipe.orders, *orderings_of(ordering))))

    def limit(self, value: int | None) -> ModelQuery:
        return ModelQuery(replace(self.recipe, limit=value))

    def offset(self, value: int | None) -> ModelQuery:
        return ModelQuery(replace(self.recipe, offset=value))

    def join(
        self,
        relation: Relation,
        *,
        kind: p.Join = p.Join.Inner,
        alias: str | None = None,
    ) -> ModelQuery:
        """Join the table at the far end of ``relation``, from the query's
        model or a table it joined, without reading its columns: the first
        table of the relation's model the query reads, or the one its
        ``from_alias`` names."""
        from .relations import Relation, joined_on, same_table

        if not isinstance(relation, Relation):
            raise TypeError("a model query joins a Relation")
        if kind != p.Join.Inner and kind != p.Join.Left:
            raise TypeError("a model join is Join.Inner or Join.Left")
        recipe = self.recipe
        tables = [(recipe.model, recipe.model.qualifier)]
        tables += [(join.model, join.qualifier) for join in recipe.joins]
        if relation.start is None:
            start = next((q for model, q in tables if same_table(model, relation.from_)), None)
            if start is None:
                raise p.ConstructionError(
                    f"the relation starts at {relation.from_.table.name}, which the query does not read"
                )
        else:
            named = next((model for model, q in tables if q == relation.start), None)
            if named is None:
                raise p.ConstructionError(f"the query reads no table called {relation.start!r}")
            if not same_table(named, relation.from_):
                raise p.ConstructionError(
                    f"{relation.start!r} is {named.table.name}, where the relation starts at "
                    f"{relation.from_.table.name}"
                )
            start = relation.start
        end = relation.to
        table = end.table if alias is None else end.table.as_(alias)
        qualifier = end.qualifier if alias is None else p._identifier(alias)
        on = joined_on(relation, start, qualifier)
        join = Joined(end, table, qualifier, on, kind)
        return ModelQuery(replace(recipe, joins=(*recipe.joins, join)))

    async def all(self, db: Database) -> list[ModelRecord]:
        return await decode_all(db, self.statement, self._sources())

    async def one(self, db: Database) -> ModelRecord:
        return cast(ModelRecord, await decode_single(db, self.statement, self._sources(), optional=False))

    async def optional(self, db: Database) -> ModelRecord | None:
        return cast(ModelRecord | None, await decode_single(db, self.statement, self._sources(), optional=True))

    async def stream(self, db: Database) -> RowStream[ModelRecord]:
        """The rows, one per pull, over a connection or a transaction the
        stream holds until it ends."""
        return await decode_stream(db, self.statement, self._sources())

    async def count(self, db: Database) -> int:
        """How many rows the query reads, its limit and offset aside."""
        return await decode_count(db, self.statement)

    def cursor(self, *fields: str) -> Cursor:
        """A keyset cursor over the rows, ordered by ``fields``."""
        from .cursors import Cursor, Keyed

        recipe = self.recipe
        if not fields:
            raise p.ConstructionError("a cursor orders by at least one field")
        order = tuple(recipe.model.col(name) for name in fields)
        return Cursor(
            Keyed(
                lambda orders, wheres, limit: recipe.select(orders, wheres, limit),
                recipe.limit,
                self._sources(),
                order,
                (),
            )
        )

    def paginate(self, page_size: int) -> Paginator:
        """Pages of ``page_size`` rows."""
        from .cursors import Pageable, Paginator

        recipe = self.recipe
        return Paginator(
            Pageable(
                lambda limit, offset: replace(recipe, offset=offset).select(limit=limit),
                p._model_count(self.statement),
                self._sources(),
            ),
            page_size,
        )

    def _sources(self) -> tuple[Source, ...]:
        return (own_source(self.recipe.model, self.recipe.fields),)

    def __bool__(self) -> bool:
        raise p.ConstructionError("model queries cannot be tested as Python booleans")


@dataclass(frozen=True, eq=False, slots=True)
class ModelRows:
    """A write with a RETURNING list of a model's fields, whose terminals
    decode each returned row; an insert of no rows has no statement and
    returns none."""

    model: Model
    statement: p.Insert | p.Update | p.Delete | None
    selected: tuple[str, ...]

    def __post_init__(self) -> None:
        from .model import Model

        if not isinstance(self.model, Model) or not isinstance(
            self.statement, (p.Insert, p.Update, p.Delete, type(None))
        ):
            raise TypeError("model rows are a model's native write statement")
        object.__setattr__(
            self, "selected", self.model._selection(tuple(self.selected))
        )

    def inspect(self) -> p.Compiled:
        return written(self.statement).inspect()

    async def all(self, db: Database) -> list[ModelRecord]:
        if self.statement is None:
            return []
        return await decode_all(db, self.statement, self._sources())

    async def one(self, db: Database) -> ModelRecord:
        if self.statement is None:
            raise p.DecodeError("expected exactly one row, and the insert wrote none")
        return cast(ModelRecord, await decode_single(db, self.statement, self._sources(), optional=False))

    async def optional(self, db: Database) -> ModelRecord | None:
        if self.statement is None:
            return None
        return cast(ModelRecord | None, await decode_single(db, self.statement, self._sources(), optional=True))

    def _sources(self) -> tuple[Source, ...]:
        return (own_source(self.model, self.selected),)

    def __bool__(self) -> bool:
        raise p.ConstructionError("model queries cannot be tested as Python booleans")


def written[S: p.Insert | p.Update | p.Delete](statement: S | None) -> S:
    """The statement a write runs; an insert of no rows has none."""
    if statement is None:
        raise p.ConstructionError("an insert of no rows writes nothing and has no statement")
    return statement
