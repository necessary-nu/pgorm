"""A model query's projections: items of its columns, its joined tables'
columns and expressions, each decoded as its declaration says, and the
partial models that declare a projection once."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, replace
from types import MappingProxyType
from typing import TYPE_CHECKING, Any, Callable, cast

from .. import _native as p
from . import casts
from .columns import Column, ModelColumn
from .clauses import Clauses, Recipe
from .record import Database, decode_all, decode_single, decode_stream

if TYPE_CHECKING:
    from ..results import RowStream
    from ..runtime import Connection, Pool
    from .cursors import Cursor, Paginator
    from .model import Model


# [spec:pgorm:req:bindings.model-projections]
@dataclass(frozen=True, slots=True)
class Item:
    """One item of a projection: the name its value is read under, the
    expression projected, and the declaration its value is decoded by."""

    name: str
    projected: p.Expr | p.Aliased
    declaration: Column

    @staticmethod
    def of(expr: p.Expr, declaration: Column, name: str, aliased: bool) -> Item:
        """``expr`` read under ``name``: aliased when ``aliased`` says so, and
        cast to ``text`` (``text[]``) where the declaration is read from its
        text, as a model's field is."""
        selected = casts.selected(declaration, expr)
        projected: p.Expr | p.Aliased = (
            selected.as_(name) if aliased or selected is not expr else expr
        )
        return Item(name, projected, declaration)

    @staticmethod
    def column(column: ModelColumn, name: str | None, *, always: bool) -> Item:
        """A model column under ``name``, its field's by default: aliased
        always when ``always`` says so, as Rust's ``column_as`` aliases, and
        otherwise only where the name is not the column's."""
        if not isinstance(column, ModelColumn):
            raise TypeError("a projection's column is a model's column, model.col(field)")
        key = column.field_name if name is None else p._identifier(name)
        declaration = column.declaration
        return Item.of(column.expr(), declaration, key, always or key != declaration.name)

    @staticmethod
    def field(model: Model, field: str) -> Item:
        """The model's field as its own query projects it."""
        return Item.column(model.col(field), None, always=False)

    def decoded(self, row: p.Record) -> Any:
        """The item's value in ``row``, held to its declaration."""
        declaration = self.declaration
        value = declaration._decoded(row.tagged(self.name))
        if declaration._read_as_text or declaration.decode is not None:
            return casts.read(declaration, value)
        return row[self.name]


def _expression_declaration(declaration: object) -> Column:
    """``declaration`` as an expression's: a Column that says only how the
    value decodes, its kind, nullability, array and values."""
    if not isinstance(declaration, Column):
        raise TypeError("an expression's declaration is a Column, Column(kind, ..)")
    if (
        declaration.name is not None
        or declaration.primary_key
        or declaration.default
        or declaration.generated is not None
    ):
        raise p.ConstructionError(
            "an expression's declaration says how it decodes: its kind, nullable, array and values"
        )
    return declaration


# [spec:pgorm:req:bindings.model-projections]
@dataclass(frozen=True, eq=False, slots=True)
class ModelProjection(Clauses):
    """A model query's projection: items chosen by the program, read once
    ``into_tuple``, ``into_values`` or ``into_model`` says what each row
    becomes. Every method returns a new projection."""

    recipe: Recipe

    def __post_init__(self) -> None:
        if not isinstance(self.recipe, Recipe) or self.recipe.items is None:
            raise TypeError("a ModelProjection comes from a model query's select_only()")

    @property
    def items(self) -> tuple[Item, ...]:
        return cast(tuple[Item, ...], self.recipe.items)

    @property
    def statement(self) -> p.Select:
        return self.recipe.select(limit=self.recipe.limit)

    def inspect(self) -> p.Compiled:
        return self.statement.inspect()

    def _added(self, *items: Item) -> ModelProjection:
        names = [item.name for item in self.items]
        for item in items:
            if item.name in names:
                raise p.ConstructionError(f"two items of the projection are named {item.name!r}")
            names.append(item.name)
        return ModelProjection(replace(self.recipe, items=(*self.items, *items)))

    def column(self, column: ModelColumn) -> ModelProjection:
        """A model's column, the queried one's or a joined table's, read under
        its field's name and decoded as its field declares."""
        return self._added(Item.column(column, None, always=False))

    def columns(self, *columns: ModelColumn) -> ModelProjection:
        return self._added(*(Item.column(column, None, always=False) for column in columns))

    def column_as(self, column: ModelColumn, name: str) -> ModelProjection:
        """A model's column under ``name``, decoded as its field declares."""
        return self._added(Item.column(column, name, always=True))

    def expr_as(self, expr: p.Expr, name: str, declaration: Column) -> ModelProjection:
        """An expression under ``name``, decoded as ``declaration`` says: an
        aggregate's or a function's result, whose type the program states as
        Rust states it in the type ``into_tuple`` decodes into."""
        if not isinstance(expr, p.Expr):
            raise TypeError("expr_as projects an expression")
        declared = _expression_declaration(declaration)
        return self._added(Item.of(expr, declared, p._identifier(name), True))

    def _selector[R](self, read: Callable[[tuple[Any, ...]], R]) -> ModelSelector[R]:
        items = self.items
        if not items:
            raise p.ConstructionError("a projection selects at least one item")
        return ModelSelector(self.recipe, read)

    def into_tuple(self) -> ModelSelector[tuple[Any, ...]]:
        """Each row as a tuple of the items' values, in projection order."""
        return self._selector(lambda values: values)

    def into_values(self, *names: str) -> ModelSelector[tuple[Any, ...]]:
        """Each row as a tuple of the values of the items ``names`` names, in
        that order: Rust's ``into_values``, which reads by a ``Column``
        enum's names."""
        known = [item.name for item in self.items]
        if not names:
            raise p.ConstructionError("into_values names at least one item")
        positions: list[int] = []
        for name in names:
            if type(name) is not str:
                raise TypeError("into_values names items by string")
            if name not in known:
                raise p.ConstructionError(f"the projection has no item {name!r}")
            if known.index(name) in positions:
                raise p.ConstructionError(f"into_values names {name!r} twice")
            positions.append(known.index(name))
        return self._selector(lambda values: tuple(values[at] for at in positions))

    def into_model[R](self, target: Callable[..., R]) -> ModelSelector[R]:
        """Each row as ``target`` called with each item's value as the
        keyword argument of its name: a dataclass named for the items, as
        Rust's ``into_model`` reads a ``FromQueryResult`` struct by name."""
        if not callable(target):
            raise TypeError("into_model takes a callable such as a dataclass")
        names = [item.name for item in self.items]
        return self._selector(lambda values: target(**dict(zip(names, values))))

    def __bool__(self) -> bool:
        raise p.ConstructionError("model projections cannot be tested as Python booleans")


# [spec:pgorm:req:bindings.model-projections]
@dataclass(frozen=True, eq=False, slots=True)
class ModelSelector[T]:
    """A projection whose rows are read: Rust's ``Selector``. Its terminals
    decode each row's items as declared, a column of another kind, a NULL
    where an item is not nullable, or a label it does not list being a
    ``DecodeError``, and make each row what the projection chose."""

    recipe: Recipe
    read: Callable[[tuple[Any, ...]], T]

    @property
    def statement(self) -> p.Select:
        return self.recipe.select(limit=self.recipe.limit)

    def inspect(self, *, terminal: str | None = None) -> p.Compiled:
        """The SQL and values the rows are read with, or with
        ``terminal="count"`` the ones ``count`` runs."""
        if terminal is None:
            return self.statement.inspect()
        if terminal == "count":
            return p._model_count(self.statement).inspect()
        raise TypeError('a model selector inspects itself, or its "count"')

    def _decode(self, row: p.Record) -> T:
        items = cast(tuple[Item, ...], self.recipe.items)
        names = tuple(item.name for item in items)
        if row.keys() != names:
            raise p.DecodeError(
                f"the result's columns {list(row.keys())} are not the projection's {list(names)}"
            )
        return self.read(tuple(item.decoded(row) for item in items))

    async def all(self, db: Database) -> list[T]:
        return await decode_all(db, self.statement, self._decode)

    async def one(self, db: Database) -> T:
        return cast(T, await decode_single(db, self.statement, self._decode, optional=False))

    async def optional(self, db: Database) -> T | None:
        return cast(T | None, await decode_single(db, self.statement, self._decode, optional=True))

    async def stream(self, db: Connection | Pool) -> RowStream[T]:
        return await decode_stream(db, self.statement, self._decode)

    async def count(self, db: Database) -> int:
        """How many rows the projection reads, its limit and offset aside."""
        return int((await db.one(p._model_count(self.statement)))["num_items"])

    def cursor(self, *fields: str) -> Cursor:
        """A keyset cursor over the rows, ordered by ``fields`` of the queried
        model, each row read as the selector reads it."""
        from .cursors import Cursor, Keyed

        recipe = self.recipe
        if not fields:
            raise p.ConstructionError("a cursor orders by at least one field")
        model = recipe.model
        order = tuple((model, model.qualifier, model.col(name).field_name) for name in fields)
        return Cursor(
            Keyed(
                lambda orders, wheres, limit: recipe.select(orders, wheres, limit),
                recipe.limit,
                self._decode,
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
                self._decode,
            ),
            page_size,
        )

    def __bool__(self) -> bool:
        raise p.ConstructionError("model selectors cannot be tested as Python booleans")


# [spec:pgorm:req:bindings.partial-models]
@dataclass(frozen=True, slots=True, init=False)
class FromExpr:
    """A partial model's field read from an expression, decoded as its
    declaration says: Rust's ``from_expr``."""

    expr: p.Expr
    declaration: Column

    def __init__(self, expr: p.Expr, declaration: Column) -> None:
        if not isinstance(expr, p.Expr):
            raise TypeError("FromExpr reads an expression")
        object.__setattr__(self, "expr", expr)
        object.__setattr__(self, "declaration", _expression_declaration(declaration))


# [spec:pgorm:req:bindings.partial-models]
@dataclass(frozen=True, slots=True)
class Skip:
    """A partial model's field read from no column, holding ``default`` in
    every row: Rust's ``skip``, which takes the type's ``Default``."""

    default: Any = None


# [spec:pgorm:req:bindings.partial-models]
@dataclass(frozen=True, eq=False, slots=True, init=False)
class PartialModel:
    """A read shape over a model: each field one of the model's columns (Rust's
    ``from_col``, a field named for its column being Rust's plain field), an
    expression (``FromExpr``) or nothing (``Skip``). ``into_partial_model``
    selects what it declares and decodes each row into a read-only mapping of
    its fields, in declaration order."""

    model: Model
    fields: Mapping[str, ModelColumn | FromExpr | Skip]

    def __init__(self, model: Model, fields: Mapping[str, ModelColumn | FromExpr | Skip]) -> None:
        from .model import Model

        if not isinstance(model, Model):
            raise TypeError("a partial model is a read shape over a Model")
        if not isinstance(fields, Mapping):
            raise TypeError("a partial model's fields are a mapping of names to fields")
        declared: dict[str, ModelColumn | FromExpr | Skip] = {}
        for name, field in fields.items():
            name = p._identifier(name)
            if not isinstance(field, (ModelColumn, FromExpr, Skip)):
                raise TypeError(
                    f"{name} is a model column, a FromExpr or a Skip"
                )
            declared[name] = field
        if not any(not isinstance(field, Skip) for field in declared.values()):
            raise p.ConstructionError("a partial model reads at least one field")
        object.__setattr__(self, "model", model)
        object.__setattr__(self, "fields", MappingProxyType(declared))

    def _items(self) -> tuple[Item, ...]:
        items = []
        for name, field in self.fields.items():
            if isinstance(field, ModelColumn):
                items.append(Item.column(field, name, always=False))
            elif isinstance(field, FromExpr):
                items.append(Item.of(field.expr, field.declaration, name, True))
        return tuple(items)

    def _selector(self, recipe: Recipe) -> ModelSelector[Mapping[str, Any]]:
        """``recipe``'s query read as this shape: the model's own, refused
        for another."""
        from .relations import same_table

        if not same_table(recipe.model, self.model):
            raise p.ConstructionError(
                f"the partial model reads {self.model.table.name}, and the query {recipe.model.table.name}"
            )
        fields = tuple(self.fields.items())

        def read(values: tuple[Any, ...]) -> Mapping[str, Any]:
            found = iter(values)
            return MappingProxyType(
                {
                    name: field.default if isinstance(field, Skip) else next(found)
                    for name, field in fields
                }
            )

        return ModelSelector(replace(recipe, items=self._items()), read)
