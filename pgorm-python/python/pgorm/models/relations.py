"""Relations between models: fields of one model's table equal to fields of
another's, which a query joins by, a graph decodes along, and ``find`` and
``load`` read the far end of; and the paths of more than one relation, a
junction's (``Related``) and a chain's (``Linked``)."""

from __future__ import annotations

import json
from collections.abc import Callable, Hashable, Mapping, Sequence
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, Literal, TypeAlias

from .. import _native as p
from .record import Source, decode_all

if TYPE_CHECKING:
    from .model import Model
    from .queries import Joined, ModelQuery, Recipe
    from .record import Database, ModelRecord

KINDS = ("belongs_to", "has_one", "has_many")

ConditionType: TypeAlias = Literal["all", "any"]
OnCondition: TypeAlias = "Callable[[Model, Model], p.Expr | p.Condition]"

# The name a load joins its starting model back under, pgorm's loader's for
# the input entity; a find joins it under its own name unless that is taken.
SOURCE = "pgorm_loader_src"


def _fields(model: Model, names: object, side: str) -> tuple[str, ...]:
    listed = (names,) if isinstance(names, str) else names
    if not isinstance(listed, (list, tuple)) or not all(isinstance(name, str) for name in listed):
        raise TypeError(f"a relation's {side} is a field or a list of fields")
    if not listed:
        raise p.ConstructionError(f"a relation's {side} names at least one field")
    fields = tuple(model.col(name).field_name for name in listed)
    if len(set(fields)) != len(fields):
        raise p.ConstructionError(f"a relation's {side} names each field once")
    return fields


# [spec:pgorm:req:bindings.relations+1]
@dataclass(frozen=True, eq=False, slots=True, init=False)
class Relation:
    """A relation from one model to another: the fields of each that are
    equal in a related pair of rows, matched in order, and a condition of
    its own it adds, as pgorm's ``on_condition`` does. A ``belongs_to`` or
    ``has_one`` relation reads at most one row at its far end, a
    ``has_many`` any number."""

    kind: Literal["belongs_to", "has_one", "has_many"]
    from_: Model
    to: Model
    from_fields: tuple[str, ...]
    to_fields: tuple[str, ...]
    on_condition: OnCondition | None
    condition_type: ConditionType
    start: str | None

    def __init__(
        self,
        kind: Literal["belongs_to", "has_one", "has_many"],
        origin: Model,
        target: Model,
        from_: str | Sequence[str],
        to: str | Sequence[str],
        on_condition: OnCondition | None = None,
        condition_type: ConditionType = "all",
    ) -> None:
        from .model import Model

        if kind not in KINDS:
            raise TypeError("a relation is belongs_to, has_one or has_many")
        if not isinstance(origin, Model) or not isinstance(target, Model):
            raise TypeError("a relation joins two models")
        if on_condition is not None and not callable(on_condition):
            raise TypeError(
                "a relation's on_condition is a function of the two ends, giving a condition"
            )
        if condition_type not in ("all", "any"):
            raise TypeError('a relation\'s condition_type is "all" or "any"')
        from_fields = _fields(origin, from_, "from_")
        to_fields = _fields(target, to, "to")
        if len(from_fields) != len(to_fields):
            raise p.ConstructionError("a relation pairs as many fields at each end")
        for name, value in (
            ("kind", kind),
            ("from_", origin),
            ("to", target),
            ("from_fields", from_fields),
            ("to_fields", to_fields),
            ("on_condition", on_condition),
            ("condition_type", condition_type),
            ("start", None),
        ):
            object.__setattr__(self, name, value)

    @property
    def cardinality(self) -> Literal["one", "many"]:
        """Whether a row reads at most one row at the far end, or any number."""
        return "many" if self.kind == "has_many" else "one"

    def from_alias(self, alias: str) -> Relation:
        """The relation starting from the table a query or a graph reads
        under ``alias``, as pgorm's ``RelationDef::from_alias`` points it:
        how a path goes on from a second visit to one table."""
        moved = object.__new__(Relation)
        for name in ("kind", "from_", "to", "from_fields", "to_fields", "on_condition", "condition_type"):
            object.__setattr__(moved, name, getattr(self, name))
        object.__setattr__(moved, "start", p._identifier(alias))
        return moved

    def find(self, row: Mapping[str, Any]) -> ModelQuery:
        """The rows at the far end related to ``row``, a record of the model
        the relation starts at: none when one of its key fields is None. A
        relation with an ``on_condition`` joins the model it starts at back,
        so the condition reads both ends, keyed by its primary key, as
        pgorm's ``find_related`` does."""
        from .queries import ModelQuery, Recipe

        to = self.to
        if self.on_condition is None:
            values = _key_values(self.from_fields, row)
            found = to.find()
            if values is None:
                return found.where(p.Condition.any())
            return found.where(
                p.Condition.all(
                    *(to.col(field).eq(value) for field, value in zip(self.to_fields, values))
                )
            )
        start = _free(self.from_.qualifier, {to.qualifier})
        recipe = Recipe(to, to._selection(()), joins=(_back(self, start, to.qualifier, p.Join.Inner),))
        return _keyed(ModelQuery(recipe), self.from_, start, _start_key(self.from_, self.from_fields), row)

    async def load(
        self,
        db: Database,
        rows: Sequence[Mapping[str, Any]],
        query: ModelQuery | None = None,
    ) -> list[Any]:
        """The far end of each of ``rows``, read in one query, as pgorm's
        ``load_one`` and ``load_many`` read it: ``query``, a query of the
        far end's model, filtered and ordered as it says, with the model the
        relation starts at joined back, so the relation is read whole, its
        ``on_condition`` included. A list of records per row for
        ``has_many``, in the query's order; otherwise a record per row, or
        None where nothing matched or the row's key holds a None. Two
        different rows for one key of a ``has_one`` or ``belongs_to``
        relation are a ``DecodeError``."""
        return await _load(db, rows, self, (), query, self.cardinality == "many")


# [spec:pgorm:req:bindings.related]
@dataclass(frozen=True, eq=False, slots=True, init=False)
class Related:
    """The path from one model to another through a junction table, as a
    Rust ``Related`` implementation with a ``via`` declares it: ``via`` from
    the model to the junction, then ``relation`` from the junction to the far
    end, which reaches at most one row from each junction row."""

    via: Relation
    relation: Relation

    def __init__(self, origin: Model, relation: Relation, via: Relation) -> None:
        from .model import Model

        if not isinstance(origin, Model):
            raise TypeError("a related path starts at a model")
        if not isinstance(relation, Relation) or not isinstance(via, Relation):
            raise TypeError("a related path is a Relation through a via Relation")
        if relation.start is not None or via.start is not None:
            raise p.ConstructionError("a related path's relations start from their own tables")
        if not same_table(via.from_, origin):
            raise p.ConstructionError(
                f"the via relation starts at {via.from_.table.name}, not {origin.table.name}"
            )
        if not same_table(relation.from_, via.to):
            raise p.ConstructionError(
                f"the relation starts at {relation.from_.table.name}, where the via relation "
                f"reaches {via.to.table.name}"
            )
        if relation.cardinality != "one":
            raise p.ConstructionError(
                "a junction's relation to the far end reaches one row, a belongs_to or a has_one"
            )
        object.__setattr__(self, "via", via)
        object.__setattr__(self, "relation", relation)

    @property
    def from_(self) -> Model:
        """The model the path starts at."""
        return self.via.from_

    @property
    def to(self) -> Model:
        """The model at its far end."""
        return self.relation.to

    @property
    def cardinality(self) -> Literal["many"]:
        return "many"

    def find(self, row: Mapping[str, Any]) -> ModelQuery:
        """The rows at the far end related to ``row`` through the junction:
        the far end's query joined to the junction and to the model the path
        starts at, keyed by that model's primary key, as pgorm's
        ``find_related`` writes it; none when one of the row's key fields is
        None."""
        from .queries import ModelQuery, Recipe

        to = self.to
        junction = _free(self.relation.from_.qualifier, {to.qualifier})
        start = _free(self.from_.qualifier, {to.qualifier, junction})
        recipe = Recipe(
            to,
            to._selection(()),
            joins=(
                _back(self.relation, junction, to.qualifier, p.Join.Inner),
                _back(self.via, start, junction, p.Join.Inner),
            ),
        )
        return _keyed(
            ModelQuery(recipe), self.from_, start, _start_key(self.from_, self.via.from_fields), row
        )

    async def load(
        self,
        db: Database,
        rows: Sequence[Mapping[str, Any]],
        query: ModelQuery | None = None,
    ) -> list[list[ModelRecord]]:
        """The far end of each of ``rows`` through the junction, read in one
        query, as pgorm's ``load_many_via`` reads it: ``query`` of the far
        end with the junction and the model the path starts at joined back.
        A list of records per row, in the query's order, an empty one where
        nothing matched or the row's key holds a None."""
        loaded: list[list[ModelRecord]] = await _load(
            db, rows, self.via, (self.relation,), query, True
        )
        return loaded


# [spec:pgorm:req:bindings.linked]
@dataclass(frozen=True, eq=False, slots=True, init=False)
class Linked:
    """A chain of relations from one model to another, each starting where
    the one before it ends, as a Rust ``Linked`` type lists them. A query
    along it joins every hop under an alias of its own, ``r0`` the last
    relation's start, ``r1`` the one before, as pgorm's ``find_linked``
    does, so a chain may pass through one table more than once."""

    relations: tuple[Relation, ...]

    def __init__(self, origin: Model, relations: Sequence[Relation]) -> None:
        from .model import Model

        if not isinstance(origin, Model):
            raise TypeError("a linked chain starts at a model")
        chain = tuple(relations)
        if not chain:
            raise p.ConstructionError("a linked chain has at least one relation")
        if not all(isinstance(relation, Relation) for relation in chain):
            raise TypeError("a linked chain is made of Relations")
        if any(relation.start is not None for relation in chain):
            raise p.ConstructionError("a linked chain's relations start from their own tables")
        if not same_table(chain[0].from_, origin):
            raise p.ConstructionError(
                f"the chain's first relation starts at {chain[0].from_.table.name}, "
                f"not {origin.table.name}"
            )
        for index, (before, after) in enumerate(zip(chain, chain[1:]), start=1):
            if not same_table(after.from_, before.to):
                raise p.ConstructionError(
                    f"relation {index} starts at {after.from_.table.name}, where the one "
                    f"before it reaches {before.to.table.name}"
                )
        object.__setattr__(self, "relations", chain)

    @property
    def from_(self) -> Model:
        return self.relations[0].from_

    @property
    def to(self) -> Model:
        return self.relations[-1].to

    def hop(self, index: int) -> Model:
        """The model a query along the chain reads at hop ``index``, under
        the alias pgorm's ``LinkedAlias::hop`` names it by: ``r0`` the last
        relation's start, counting back towards the chain's own."""
        if type(index) is not int:
            raise TypeError("a hop is its index")
        if not 0 <= index < len(self.relations):
            raise p.ConstructionError(f"the chain has no hop {index}")
        return self.relations[len(self.relations) - 1 - index].from_.as_(f"r{index}")

    def last_hop(self) -> Model:
        """The model the chain starts at, under the alias pgorm's
        ``last_hop_alias`` names: what a condition on the starting row names."""
        return self.hop(len(self.relations) - 1)

    def find_linked(self) -> ModelQuery:
        """Every row at the chain's far end that a chain of rows reaches, read
        along each hop by an INNER JOIN, as pgorm's ``Linked::find_linked``
        writes it."""
        from .queries import ModelQuery, Recipe

        to = self.to
        joins: list[Joined] = []
        right = to.qualifier
        for index, relation in enumerate(reversed(self.relations)):
            left = f"r{index}"
            joins.append(_back(relation, left, right, p.Join.Inner))
            right = left
        return ModelQuery(Recipe(to, to._selection(()), joins=tuple(joins)))

    def find(self, row: Mapping[str, Any]) -> ModelQuery:
        """The rows at the far end the chain reaches from ``row``, a record of
        the model it starts at, keyed by that model's primary key, as pgorm's
        ``ModelTrait::find_linked`` keys it: none when one of the row's key
        fields is None."""
        first = self.relations[0]
        return _keyed(
            self.find_linked(),
            first.from_,
            self.last_hop().qualifier,
            _start_key(first.from_, first.from_fields),
            row,
        )


def answering(model: Model, qualifier: str) -> Model:
    """``model`` answering to ``qualifier``: itself when that is its name,
    otherwise aliased to it."""
    return model if model.qualifier == qualifier else model.as_(qualifier)


def _key_values(fields: tuple[str, ...], row: Mapping[str, Any]) -> tuple[Any, ...] | None:
    if not isinstance(row, Mapping):
        raise TypeError("a relation reads a record of the model it starts at")
    for field in fields:
        if field not in row:
            raise p.ConstructionError(f"the record has no {field}")
    values = tuple(row[field] for field in fields)
    return None if any(value is None for value in values) else values


def hashable(key: tuple[Any, ...]) -> Hashable:
    """A key two equal keys share, whatever containers its values are in."""
    return tuple(
        json.dumps(value, sort_keys=True, default=repr) if isinstance(value, (list, dict)) else value
        for value in key
    )


def same_table(left: Model, right: Model) -> bool:
    """Whether two models read one table, whatever each is aliased as."""
    return (left.table.name, left.table.schema) == (right.table.name, right.table.schema)


# [spec:pgorm:req:bindings.relations+1]
def joined_on(relation: Relation, left: str, right: str) -> p.Condition:
    """The condition joining ``relation`` between the tables answering to
    ``left`` and ``right``, as pgorm's ``join_condition`` writes it: each
    pair of its fields equal, the pairs ANDed, then its ``on_condition``
    given the two ends as they are named, the two composed by its
    ``condition_type``."""
    pairs = zip(relation.from_fields, relation.to_fields)
    parts: list[p.Expr | p.Condition] = [
        p.Condition.all(
            *(
                p.col(relation.from_._sql(origin), table=left)
                == p.col(relation.to._sql(target), table=right)
                for origin, target in pairs
            )
        )
    ]
    if relation.on_condition is not None:
        parts.append(
            condition_of(
                relation.on_condition(
                    answering(relation.from_, left), answering(relation.to, right)
                ),
                "a relation's on_condition",
            )
        )
    compose = p.Condition.any if relation.condition_type == "any" else p.Condition.all
    return compose(*parts)


def condition_of(value: object, what: str) -> p.Expr | p.Condition:
    if not isinstance(value, (p.Expr, p.Condition)):
        raise TypeError(f"{what} gives an expression or a Condition")
    return value


def _free(qualifier: str, taken: set[str]) -> str:
    """``qualifier``, or ``SOURCE`` where a table the read has answers to it."""
    if qualifier not in taken:
        return qualifier
    if SOURCE in taken:
        raise p.ConstructionError(f"the read already has a table called {SOURCE!r}")
    return SOURCE


def _back(relation: Relation, left: str, right: str, kind: p.Join) -> Joined:
    """The relation's starting table joined under ``left`` from the table
    answering to ``right``, its far end: the relation walked backwards, its
    condition written in the roles it was declared with."""
    from .queries import Joined

    model = relation.from_
    table = model.table if left == model.qualifier else model.table.as_(left)
    return Joined(model, table, left, joined_on(relation, left, right), kind)


def _start_key(model: Model, fields: tuple[str, ...]) -> tuple[str, ...]:
    """The fields a find joining its starting model keys the row by: the
    model's primary key, as pgorm keys a model, or where it declares none,
    the fields its path starts from."""
    return model.primary_key or fields


def _keyed(
    query: ModelQuery,
    model: Model,
    qualifier: str,
    fields: tuple[str, ...],
    row: Mapping[str, Any],
) -> ModelQuery:
    values = _key_values(fields, row)
    if values is None:
        return query.where(p.Condition.any())
    return query.where(
        p.Condition.all(
            *(model._column(field, qualifier).eq(value) for field, value in zip(fields, values))
        )
    )


def _among(model: Model, qualifier: str, fields: tuple[str, ...], keys: list[tuple[Any, ...]]) -> p.Expr:
    """``fields`` of ``model`` answering to ``qualifier`` among ``keys``: an
    IN list of one column, or of a row of the key's columns."""
    columns = [model._column(field, qualifier) for field in fields]
    if len(columns) == 1:
        return columns[0].is_in([key[0] for key in keys])
    rows = [
        p.tuple_expr(*(column.declaration._value(value) for column, value in zip(columns, key)))
        for key in keys
    ]
    return p.tuple_expr(*columns).is_in(rows)


def _loader(
    recipe: Recipe, first: Relation, hops: tuple[Relation, ...], keys: list[tuple[Any, ...]]
) -> tuple[p.Select, tuple[Source, Source]]:
    """The one read a load runs, as pgorm's loader builds it: the far end's
    query re-rooted, each hop walked backwards from it, and the model the path
    starts at joined back under ``SOURCE`` as the one required source, whose
    key fields carry each row's key out of the join."""
    from .queries import Joined

    root = recipe.model
    joins: list[Joined] = list(recipe.joins)
    taken = {root.qualifier, *(join.qualifier for join in joins)}
    right = root.qualifier
    for relation in reversed(hops):
        left = relation.from_.qualifier
        if left in taken:
            left = "pgorm_loader_via"
        joins.append(_back(relation, left, right, p.Join.Left))
        taken.add(left)
        right = left
    if SOURCE in taken:
        raise p.ConstructionError(f"a loaded query cannot read a table called {SOURCE!r}")
    joins.append(_back(first, SOURCE, right, p.Join.Inner))
    origin = first.from_
    target_names = tuple(p._model_result_names("s0_", [root._sql(field) for field in recipe.fields]))
    source_names = tuple(
        p._model_result_names("s1_", [origin._sql(field) for field in first.from_fields])
    )
    projection = [
        *(
            root._projected(field, root.col(field), name)
            for field, name in zip(recipe.fields, target_names)
        ),
        *(
            origin._projected(field, origin._column(field, SOURCE), name)
            for field, name in zip(first.from_fields, source_names)
        ),
    ]
    query = p.Select(*projection).from_(root.table)
    for join in joins:
        query = query.join(join.table, join.on, kind=join.kind)
    for where in (*recipe.wheres, _among(origin, SOURCE, first.from_fields, keys)):
        query = query.where(where)
    if recipe.orders:
        query = query.order_by(*recipe.orders)
    query = query.limit(recipe.limit).offset(recipe.offset)
    return query, (
        Source(root, recipe.fields, target_names),
        Source(origin, first.from_fields, source_names),
    )


async def _load(
    db: Database,
    rows: Sequence[Mapping[str, Any]],
    first: Relation,
    hops: tuple[Relation, ...],
    query: ModelQuery | None,
    many: bool,
) -> list[Any]:
    from .queries import ModelQuery

    if not isinstance(rows, (list, tuple)):
        raise TypeError("load takes a list or tuple of records")
    far = (hops[-1] if hops else first).to
    if query is None:
        query = far.find()
    elif not isinstance(query, ModelQuery):
        raise TypeError("load takes a query of the far end's model")
    elif not same_table(query.model, far):
        raise p.ConstructionError(
            f"load reads {far.table.name}, and the query reads {query.model.table.name}"
        )
    keys = [_key_values(first.from_fields, row) for row in rows]
    wanted = {hashable(key): key for key in keys if key is not None}
    buckets: dict[Hashable, list[ModelRecord]] = {key: [] for key in wanted}
    if wanted:
        statement, sources = _loader(query.recipe, first, hops, list(wanted.values()))
        for target, source in await decode_all(db, statement, sources):
            key = hashable(tuple(source[field] for field in first.from_fields))
            bucket = buckets.get(key)
            if bucket is None:
                raise p.DecodeError(
                    f"a loaded row's key {key!r}, read back from {list(first.from_fields)}, "
                    "equals none of the rows' keys"
                )
            bucket.append(target)
    if many:
        return [[] if key is None else list(buckets[hashable(key)]) for key in keys]
    return [None if key is None else _single(buckets[hashable(key)], key, far) for key in keys]


def _single(bucket: list[ModelRecord], key: tuple[Any, ...], far: Model) -> ModelRecord | None:
    """The one row a key of a relation to one row reached: repeats of one row,
    which joining the starting model back makes, collapse, counted by the far
    end's primary key where the query reads it."""
    distinct: dict[Hashable, ModelRecord] = {}
    for record in bucket:
        fields = (
            far.primary_key
            if far.primary_key and all(field in record for field in far.primary_key)
            else tuple(record)
        )
        distinct.setdefault(hashable(tuple(record[field] for field in fields)), record)
    if len(distinct) > 1:
        raise p.DecodeError(
            f"{len(distinct)} {far.table.name} rows have the key {key!r}, where one is"
        )
    return next(iter(distinct.values()), None)
