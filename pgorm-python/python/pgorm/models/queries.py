"""Runtime model reads, and the rows a write returns, using native SQL statements."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, replace
from typing import TYPE_CHECKING, Any, Callable, cast

from .. import _native as p
from .clauses import Clauses, Recipe
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
    from .columns import Column, ModelColumn
    from .cursors import Cursor, Paginator
    from .model import Model
    from .projections import ModelProjection, ModelSelector, PartialModel
    from .relations import Relation


# [spec:pgorm:req:bindings.model-reads]
@dataclass(frozen=True, eq=False, slots=True)
class ModelQuery(Clauses):
    """A model's SELECT: every method returns a new query, and the terminals
    decode each row into a record of the fields it selects."""

    recipe: Recipe

    def __post_init__(self) -> None:
        if not isinstance(self.recipe, Recipe) or self.recipe.items is not None:
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

    async def all(self, db: Database) -> list[ModelRecord]:
        return await decode_all(db, self.statement, self._sources())

    async def one(self, db: Database) -> ModelRecord:
        return cast(ModelRecord, await decode_single(db, self.statement, self._sources(), optional=False))

    async def optional(self, db: Database) -> ModelRecord | None:
        return cast(ModelRecord | None, await decode_single(db, self.statement, self._sources(), optional=True))

    async def stream(self, db: Connection | Pool) -> RowStream[ModelRecord]:
        """The rows, one per pull, over a connection the stream holds until
        it ends."""
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
                recipe.tables(),
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

    def select_only(self) -> ModelProjection:
        """A projection of no items yet, the query's clauses kept: Rust's
        ``select_only``."""
        from .projections import ModelProjection

        return ModelProjection(replace(self.recipe, items=()))

    def _projection(self) -> ModelProjection:
        """A projection of the fields the query selects, each projected as
        the query projects it."""
        from .projections import Item, ModelProjection

        model = self.recipe.model
        items = tuple(Item.field(model, name) for name in self.recipe.fields)
        return ModelProjection(replace(self.recipe, items=items))

    def column(self, column: ModelColumn) -> ModelProjection:
        """A projection of the query's fields and ``column``."""
        return self._projection().column(column)

    def columns(self, *columns: ModelColumn) -> ModelProjection:
        return self._projection().columns(*columns)

    def column_as(self, column: ModelColumn, name: str) -> ModelProjection:
        return self._projection().column_as(column, name)

    def expr_as(self, expr: p.Expr, name: str, declaration: Column) -> ModelProjection:
        return self._projection().expr_as(expr, name, declaration)

    def into_tuple(self) -> ModelSelector[tuple[Any, ...]]:
        """The rows as tuples of the fields' values, in the order selected."""
        return self._projection().into_tuple()

    def into_values(self, *names: str) -> ModelSelector[tuple[Any, ...]]:
        return self._projection().into_values(*names)

    def into_model[T](self, target: Callable[..., T]) -> ModelSelector[T]:
        return self._projection().into_model(target)

    def into_partial_model(self, shape: PartialModel) -> ModelSelector[Mapping[str, Any]]:
        """The rows read as ``shape`` declares them, the query's clauses
        kept: Rust's ``into_partial_model``."""
        from .projections import PartialModel

        if not isinstance(shape, PartialModel):
            raise TypeError("into_partial_model takes a PartialModel")
        return shape._selector(self.recipe)

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
