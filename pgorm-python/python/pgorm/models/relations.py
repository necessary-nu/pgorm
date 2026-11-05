"""Relations between models: fields of one model's table equal to fields of
another's, which a query joins by, a graph decodes along, and ``find`` and
``load`` read the far end of."""

from __future__ import annotations

import json
from collections.abc import Hashable, Mapping, Sequence
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, Literal, cast

from .. import _native as p

if TYPE_CHECKING:
    from .model import Model
    from .queries import ModelQuery
    from .record import Database, ModelRecord

KINDS = ("belongs_to", "has_one", "has_many")

# A foreign key's referential actions, as the schema builders spell them.
ACTIONS = ("no_action", "restrict", "cascade", "set_null", "set_default")

ReferentialAction = Literal["no_action", "restrict", "cascade", "set_null", "set_default"]


def _action(value: object, option: str) -> ReferentialAction | None:
    if value is None:
        return None
    if type(value) is not str or value not in ACTIONS:
        raise TypeError(f"a relation's {option} is one of {', '.join(ACTIONS)}")
    return cast(ReferentialAction, value)


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


# [spec:pgorm:req:bindings.relations+2]
@dataclass(frozen=True, eq=False, slots=True, init=False)
class Relation:
    """A relation from one model to another: the fields of each that are
    equal in a related pair of rows, matched in order. A ``belongs_to`` or
    ``has_one`` relation reads at most one row at its far end, a
    ``has_many`` any number. A ``belongs_to`` relation is the side holding
    the foreign key, whose actions and name it carries for the schema its
    model writes."""

    kind: Literal["belongs_to", "has_one", "has_many"]
    from_: Model
    to: Model
    from_fields: tuple[str, ...]
    to_fields: tuple[str, ...]
    on_delete: ReferentialAction | None
    on_update: ReferentialAction | None
    fk_name: str | None

    def __init__(
        self,
        kind: Literal["belongs_to", "has_one", "has_many"],
        origin: Model,
        target: Model,
        from_: str | Sequence[str],
        to: str | Sequence[str],
        on_delete: ReferentialAction | None = None,
        on_update: ReferentialAction | None = None,
        fk_name: str | None = None,
    ) -> None:
        from .model import Model

        if kind not in KINDS:
            raise TypeError("a relation is belongs_to, has_one or has_many")
        if not isinstance(origin, Model) or not isinstance(target, Model):
            raise TypeError("a relation joins two models")
        from_fields = _fields(origin, from_, "from_")
        to_fields = _fields(target, to, "to")
        if len(from_fields) != len(to_fields):
            raise p.ConstructionError("a relation pairs as many fields at each end")
        keyed = (_action(on_delete, "on_delete"), _action(on_update, "on_update"))
        if fk_name is not None:
            fk_name = p._identifier(fk_name)
        if kind != "belongs_to" and (keyed != (None, None) or fk_name is not None):
            raise TypeError(
                "a foreign key is the belongs_to side's: only it takes on_delete, on_update "
                "and fk_name"
            )
        for name, value in (
            ("kind", kind),
            ("from_", origin),
            ("to", target),
            ("from_fields", from_fields),
            ("to_fields", to_fields),
            ("on_delete", keyed[0]),
            ("on_update", keyed[1]),
            ("fk_name", fk_name),
        ):
            object.__setattr__(self, name, value)

    # [spec:pgorm:req:bindings.schema-from-model]
    def _described(self) -> dict[str, Any]:
        """The relation's foreign key as the native table shape takes it."""
        return {
            "schema": self.to.table.schema,
            "table": self.to.table.name,
            "from": [self.from_._sql(field) for field in self.from_fields],
            "to": [self.to._sql(field) for field in self.to_fields],
            "on_delete": self.on_delete,
            "on_update": self.on_update,
            "fk_name": self.fk_name,
        }

    @property
    def cardinality(self) -> Literal["one", "many"]:
        """Whether a row reads at most one row at the far end, or any number."""
        return "many" if self.kind == "has_many" else "one"

    def find(self, row: Mapping[str, Any]) -> ModelQuery:
        """The rows at the far end related to ``row``, a record of the model
        the relation starts at: none when one of its key fields is None."""
        values = _key_values(self.from_fields, row)
        found = self.to.find()
        if values is None:
            return found.where(p.Condition.any())
        return found.where(
            p.Condition.all(
                *(self.to.col(field).eq(value) for field, value in zip(self.to_fields, values))
            )
        )

    async def load(
        self, db: Database, rows: Sequence[Mapping[str, Any]]
    ) -> list[Any]:
        """The far end of each of ``rows``, read in one query of the distinct
        keys: a list of records per row for ``has_many``; otherwise a record
        per row, or None where the row's key holds a None. A key nothing at
        the far end matches, or that a ``has_one`` relation matches twice, is
        a ``DecodeError``, never a missing row."""
        if not isinstance(rows, (list, tuple)):
            raise TypeError("load takes a list or tuple of records")
        keys = [_key_values(self.from_fields, row) for row in rows]
        wanted = {hashable(key): key for key in keys if key is not None}
        found: dict[Hashable, list[ModelRecord]] = {}
        if wanted:
            query = self.to.find().where(self._among(list(wanted.values())))
            for record in await query.all(db):
                key = hashable(tuple(record[field] for field in self.to_fields))
                found.setdefault(key, []).append(record)
        return [self._matched(key, found) for key in keys]

    def _matched(
        self, key: tuple[Any, ...] | None, found: dict[Hashable, list[ModelRecord]]
    ) -> Any:
        if self.kind == "has_many":
            return [] if key is None else found.get(hashable(key), [])
        if key is None:
            return None
        matched = found.get(hashable(key), [])
        if len(matched) != 1:
            raise p.DecodeError(
                f"{len(matched)} {self.to.table.name} rows have the key {key!r}, where one is"
            )
        return matched[0]

    def _among(self, keys: list[tuple[Any, ...]]) -> p.Expr:
        """The far end's key among ``keys``: an IN list of one column, or of a
        row of the key's columns."""
        columns = [self.to.col(field) for field in self.to_fields]
        if len(columns) == 1:
            return columns[0].is_in([key[0] for key in keys])
        rows = [
            p.tuple_expr(*(column.declaration._value(value) for column, value in zip(columns, key)))
            for key in keys
        ]
        return p.tuple_expr(*columns).is_in(rows)


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


def joined_on(relation: Relation, left: str, right: str) -> p.Condition:
    """The condition joining ``relation`` between the tables answering to
    ``left`` and ``right``: each pair of its fields equal, as pgorm's
    ``join_condition`` writes a relation's columns."""
    pairs = zip(relation.from_fields, relation.to_fields)
    return p.Condition.all(
        p.Condition.all(
            *(
                p.col(relation.from_._sql(origin), table=left)
                == p.col(relation.to._sql(target), table=right)
                for origin, target in pairs
            )
        )
    )
