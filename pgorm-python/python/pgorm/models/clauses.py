"""What a model query and a projection of one share: the recipe each
terminal's statement is built from, and the clauses that change it."""

from __future__ import annotations

from dataclasses import Field, dataclass, replace
from typing import TYPE_CHECKING, Any, ClassVar, Self

from .. import _native as p

if TYPE_CHECKING:
    from .model import Model
    from .projections import Item
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
class Lock:
    """A model query's row lock, as the statement builder's ``lock`` takes it."""

    strength: p.LockStrength
    wait: p.LockWait | None
    of: tuple[p.Table | p.FromItem, ...]


# [spec:pgorm:req:bindings.model-clauses]
@dataclass(frozen=True, slots=True)
class Recipe:
    """What a model query reads: its fields, or a projection's items; its
    joins, conditions, grouping, DISTINCT, ordering, window and lock, from
    which each terminal's statement is built."""

    model: Model
    fields: tuple[str, ...]
    joins: tuple[Joined, ...] = ()
    wheres: tuple[p.Expr | p.Condition, ...] = ()
    orders: tuple[p.OrderBy, ...] = ()
    limit: int | None = None
    offset: int | None = None
    # ``True`` is DISTINCT, a tuple of columns DISTINCT ON; the later call wins.
    distinct: bool | tuple[p.Expr, ...] = False
    groups: tuple[p.Expr | p.GroupingElement, ...] = ()
    havings: tuple[p.Expr | p.Condition, ...] = ()
    lock: Lock | None = None
    items: tuple[Item, ...] | None = None

    def tables(self) -> tuple[tuple[Model, str], ...]:
        """Each model the query reads, with the name its table answers to."""
        return ((self.model, self.model.qualifier), *((j.model, j.qualifier) for j in self.joins))

    def select(
        self,
        orders: tuple[p.OrderBy, ...] | None = None,
        wheres: tuple[p.Expr | p.Condition, ...] = (),
        limit: int | None = None,
    ) -> p.Select:
        """The SELECT, its ordering, further conditions and limit those given."""
        model = self.model
        if self.items is None:
            projection = tuple(model._projected(name, model.col(name)) for name in self.fields)
        elif not self.items:
            raise p.ConstructionError("a projection selects at least one item")
        else:
            projection = tuple(item.projected for item in self.items)
        query = p.Select(*projection).from_(model.table)
        for join in self.joins:
            query = query.join(join.table, join.on, kind=join.kind)
        for where in (*self.wheres, *wheres):
            query = query.where(where)
        if self.groups:
            query = query.group_by(*self.groups)
        for having in self.havings:
            query = query.having(having)
        if self.distinct is True:
            query = query.distinct()
        elif self.distinct:
            query = query.distinct_on(*self.distinct)
        ordering = self.orders if orders is None else orders
        if ordering:
            query = query.order_by(*ordering)
        query = query.limit(limit).offset(self.offset)
        if self.lock is not None:
            query = query.lock(self.lock.strength, of=self.lock.of, wait=self.lock.wait)
        return query


_JOINS = (p.Join.Inner, p.Join.Left, p.Join.Right, p.Join.Full)


# [spec:pgorm:req:bindings.model-clauses]
class Clauses:
    """The clauses a model query and a projection of one share: each returns
    a new query over the changed recipe."""

    __slots__ = ()
    # Each subclass is a dataclass over its recipe, which ``_with`` replaces.
    __dataclass_fields__: ClassVar[dict[str, Field[Any]]]
    recipe: Recipe

    def _with(self, recipe: Recipe) -> Self:
        """The query of the same kind over ``recipe``, checked as it is made."""
        return replace(self, recipe=recipe)

    def where(self, predicate: p.Expr | p.Condition) -> Self:
        recipe = self.recipe
        return self._with(replace(recipe, wheres=(*recipe.wheres, predicate_of(predicate))))

    def order_by(self, *ordering: p.OrderBy) -> Self:
        recipe = self.recipe
        return self._with(replace(recipe, orders=(*recipe.orders, *orderings_of(ordering))))

    def limit(self, value: int | None) -> Self:
        return self._with(replace(self.recipe, limit=value))

    def offset(self, value: int | None) -> Self:
        return self._with(replace(self.recipe, offset=value))

    def distinct(self) -> Self:
        """``SELECT DISTINCT``, replacing a ``distinct_on``."""
        return self._with(replace(self.recipe, distinct=True))

    def distinct_on(self, *columns: p.Expr) -> Self:
        """``SELECT DISTINCT ON (..)`` the columns, of the queried model or a
        table it joined, replacing ``distinct``."""
        if not columns:
            raise p.ConstructionError("distinct_on names at least one column")
        if not all(isinstance(column, p.Expr) for column in columns):
            raise TypeError("distinct_on takes columns")
        return self._with(replace(self.recipe, distinct=tuple(columns)))

    def group_by(self, *items: p.Expr | p.GroupingElement) -> Self:
        """Add to the GROUP BY list, as the statement builder's ``group_by``."""
        if not all(isinstance(item, (p.Expr, p.GroupingElement)) for item in items):
            raise TypeError("group_by takes expressions and grouping elements")
        recipe = self.recipe
        return self._with(replace(recipe, groups=(*recipe.groups, *items)))

    def having(self, predicate: p.Expr | p.Condition) -> Self:
        recipe = self.recipe
        return self._with(replace(recipe, havings=(*recipe.havings, predicate_of(predicate))))

    def lock(
        self,
        strength: p.LockStrength,
        *,
        of: list[p.Table | p.FromItem] | tuple[p.Table | p.FromItem, ...] = (),
        wait: p.LockWait | None = None,
    ) -> Self:
        """Lock the rows read, replacing any lock set before, as the statement
        builder's ``lock`` does: Rust's ``lock``, ``lock_shared``,
        ``lock_exclusive`` and ``lock_with_behavior``."""
        if not isinstance(strength, p.LockStrength):
            raise TypeError("a lock's strength is a LockStrength")
        if wait is not None and not isinstance(wait, p.LockWait):
            raise TypeError("a lock's wait is a LockWait")
        if not isinstance(of, (list, tuple)):
            raise TypeError("a lock's of is a list of Table or FromItem")
        lock = Lock(strength, wait, tuple(of))
        # Refused as the statement builder refuses it, as the query is made.
        p.Select().lock(strength, of=lock.of, wait=wait)
        return self._with(replace(self.recipe, lock=lock))

    def join(
        self,
        relation: Relation,
        *,
        kind: p.Join = p.Join.Inner,
        alias: str | None = None,
    ) -> Self:
        """Join the table at the far end of ``relation``, from the query's
        model or a table it joined, without reading its columns."""
        from .relations import Relation, joined_on

        if not isinstance(relation, Relation):
            raise TypeError("a model query joins a Relation")
        start = self._start(relation.from_, "starts", kind)
        end = relation.to
        table, qualifier = _joined_table(end, alias)
        on = joined_on(relation, start, qualifier)
        return self._joined(Joined(end, table, qualifier, on, kind))

    def join_rev(
        self,
        relation: Relation,
        *,
        kind: p.Join = p.Join.Inner,
        alias: str | None = None,
    ) -> Self:
        """Join the table ``relation`` starts at, from its far end, which the
        query's model or a table it joined is: Rust's ``join_rev``, the
        relation's condition as it stands."""
        from .relations import Relation, joined_on

        if not isinstance(relation, Relation):
            raise TypeError("a model query joins a Relation")
        start = self._start(relation.to, "ends", kind)
        end = relation.from_
        table, qualifier = _joined_table(end, alias)
        on = joined_on(relation, qualifier, start)
        return self._joined(Joined(end, table, qualifier, on, kind))

    def _start(self, model: Model, end: str, kind: p.Join) -> str:
        """The name of the table the join starts from, ``model``'s, which the
        relation ``end`` at."""
        from .relations import same_table

        if not any(kind == known for known in _JOINS):
            raise TypeError("a model join is Join.Inner, Join.Left, Join.Right or Join.Full")
        start = next((q for read, q in self.recipe.tables() if same_table(read, model)), None)
        if start is None:
            raise p.ConstructionError(
                f"the relation {end} at {model.table.name}, which the query does not read"
            )
        return start

    def _joined(self, join: Joined) -> Self:
        recipe = self.recipe
        return self._with(replace(recipe, joins=(*recipe.joins, join)))


def _joined_table(model: Model, alias: str | None) -> tuple[p.Table, str]:
    if alias is None:
        return model.table, model.qualifier
    return model.table.as_(alias), p._identifier(alias)
