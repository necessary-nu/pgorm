"""One-of references: a model's nullable fields, each holding the key of one
target model, of which a row references exactly one, or at most one where
the reference is optional.

A ``OneOf`` declares one over models; ``one_of_entity(name)`` is a
downstream crate's ``#[derive(OneOf)]`` enum the native module registers.
Either is pgorm's ``OneOfShape``, which decides which target a row
references, which fields a write sets, the condition each target's rows are
loaded by and the error for a key its target does not hold, and writes the
``CHECK``. Python only reads the records' fields and runs the reads.
"""

from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from typing import Any, Generic, TypeVar, cast

from .. import _native as p
from .model import Model
from .record import Database, ModelRecord

# What a reference's records are: a model's, or a registration's entities'.
R = TypeVar("R")


@dataclass(frozen=True, slots=True)
class _Reader(Generic[R]):
    """How a reference reads its records and its targets: a record's key as
    a list of ``Value`` objects, the query a target's rows are read by, and a
    target's key field."""

    row_key: Callable[[R], list[p.Value]]
    query: Callable[[int], Any]
    key: Callable[[int], str]


class _OneOf(Generic[R]):
    """What a declaration and a registration share: the native shape, each
    target's field and value, and the reads, writes and loads over them."""

    __slots__ = ("_native", "_fields", "_values", "_reader")

    _native: p._OneOf
    _fields: tuple[str, ...]
    _values: tuple[str, ...]
    _reader: _Reader[R]

    def _bind(self, native: p._OneOf, reader: _Reader[R]) -> None:
        targets = cast(list[dict[str, Any]], native.describe()["targets"])
        object.__setattr__(self, "_native", native)
        object.__setattr__(self, "_fields", tuple(target["field"] for target in targets))
        object.__setattr__(self, "_values", tuple(target["value"] for target in targets))
        object.__setattr__(self, "_reader", reader)

    def __setattr__(self, name: str, value: object) -> None:
        raise AttributeError("a one-of reference never changes")

    @property
    def name(self) -> str:
        return cast(str, self._native.describe()["name"])

    def describe(self) -> dict[str, Any]:
        """The reference as data: its ``name``, its ``model``, each target's
        ``value``, ``field`` and ``target``, and ``optional``."""
        return self._native.describe()

    def _referenced(self, record: R) -> int | None:
        fields = [cast(Any, record)[field] is not None for field in self._fields]
        return self._native.referenced(self._reader.row_key(record), fields)

    def key(self, record: R) -> tuple[str, Any] | None:
        """The record's reference as ``(value, key)``, its target's value and
        the key its field holds, or ``None`` for a row referencing none where
        the reference is optional; a row against the declaration is a
        ``DecodeError``."""
        target = self._referenced(record)
        if target is None:
            return None
        return self._values[target], cast(Any, record)[self._fields[target]]

    def _assigned(self, value: str | None, key: Any) -> dict[str, Any]:
        if value is not None and type(value) is not str:
            raise TypeError("a one-of reference's target is its value, a str, or None")
        if value is not None and key is None:
            raise p.ConstructionError(
                f"a reference to {value!r} holds its key: pass it, or None for no target"
            )
        target = None if value is None else self._native.target(value)
        assigned = self._native.assigned(target)
        return {field: key if set_ else None for field, set_ in zip(self._fields, assigned)}

    async def load(self, db: Database, records: Sequence[R]) -> list[tuple[str, R] | None]:
        """Each record's target record, as ``(value, record)``, in order, or
        ``None`` for a record referencing none: each target's table read once,
        for the keys the records hold for it. A key its table does not hold
        is a ``DecodeError``."""
        if not isinstance(records, (list, tuple)):
            raise TypeError("load takes a list or tuple of records")
        referenced = [self._referenced(record) for record in records]
        loaded: list[tuple[str, R] | None] = [None] * len(records)
        for target, field in enumerate(self._fields):
            rows = [row for row, reference in enumerate(referenced) if reference == target]
            if not rows:
                continue
            keys = [cast(Any, records[row]).tagged(field) for row in rows]
            condition = self._native.load_condition(target, keys)
            found = await self._reader.query(target).where(condition).all(db)
            key = self._reader.key(target)
            for row, tagged in zip(rows, keys):
                held = cast(Any, records[row])[field]
                match = next((item for item in found if item[key] == held), None)
                if match is None:
                    raise self._native.absent(target, self._reader.row_key(records[row]), tagged)
                loaded[row] = (self._values[target], match)
        return loaded


# [spec:pgorm:req:bindings.one-of+1]
class OneOf(_OneOf[ModelRecord]):
    """A reference from ``model``'s rows to exactly one of ``targets``, or to
    at most one where ``optional`` says so: ``targets`` maps each target's
    value, in order, to the field of ``model`` holding its key and the
    target's model, keyed by one field.

    Declaring one sends nothing; what pgorm's shape refuses — a field the
    model lacks, one that is not nullable, one field for two targets, one of
    another kind than its target's key, a target keyed by more than one
    field, one target model for two values, no target — is a
    ``ConstructionError`` raised here."""

    __slots__ = ("model", "targets", "optional")

    model: Model
    targets: Mapping[str, tuple[str, Model]]
    optional: bool

    def __init__(
        self,
        name: str,
        model: Model,
        targets: Mapping[str, tuple[str, Model]],
        *,
        optional: bool = False,
    ) -> None:
        from .disjoint import _model, _table

        if type(name) is not str:
            raise TypeError("a one-of reference's name is a str")
        if type(optional) is not bool:
            raise TypeError("a one-of reference's optional is a bool")
        model = _model(model, "a one-of reference's model")
        if not isinstance(targets, Mapping):
            raise TypeError("a one-of reference's targets map each value to (field, Model)")
        described: list[tuple[str, str, str, tuple[str, dict[str, Any]]]] = []
        held: dict[str, tuple[str, Model]] = {}
        for value, target in targets.items():
            if type(value) is not str:
                raise TypeError("a target's value is a str")
            if not isinstance(target, tuple) or len(target) != 2 or type(target[0]) is not str:
                raise TypeError("a target is (field, Model)")
            field, target_model = target[0], _model(target[1], f"the target {value!r}'s model")
            if field not in model.columns:
                raise p.ConstructionError(f"{model.table.name} has no field {field!r}")
            held[value] = (field, target_model)
            described.append(
                (value, field, model._sql(field), (target_model.table.name, _table(target_model)))
            )
        native = p._one_of_model(name, (model.table.name, _table(model)), described, optional)
        models = [target_model for _, target_model in held.values()]
        self._bind(native, _Reader(
            row_key=lambda record: [record.tagged(field) for field in model.primary_key],
            query=lambda target: models[target].find(),
            key=lambda target: models[target].primary_key[0],
        ))
        object.__setattr__(self, "model", model)
        object.__setattr__(self, "targets", held)
        object.__setattr__(self, "optional", optional)

    def set(self, value: str | None, key: Any = None) -> dict[str, Any]:
        """The fields a write sets for a reference to ``value``'s target of
        key ``key``: its field set to the key and every other to ``None``,
        each ``None`` for ``set(None)`` where the reference is optional,
        which a model's ``insert`` and ``update`` take beside their other
        values."""
        return self._assigned(value, key)

    def __repr__(self) -> str:
        return f"OneOf({self.name!r}, {self.model.table.name!r})"


# [spec:pgorm:req:bindings.one-of+1]
class OneOfEntity(_OneOf[p.EntityModel]):
    """A downstream crate's registered ``#[derive(OneOf)]`` enum: its
    targets named as its variants are, in snake_case, and its records the
    entities' own."""

    __slots__ = ()

    def __init__(self, native: p._OneOf) -> None:
        if not isinstance(native, p._OneOf):
            raise TypeError("a OneOfEntity is made by one_of_entity(name)")
        description = native.describe()
        entity = p.entity(description["model"])
        targets = [p.entity(target["target"]) for target in description["targets"]]
        self._bind(native, _Reader(
            row_key=lambda record: [
                record.tagged(column) for column in entity.describe()["primary_key"]
            ],
            query=lambda target: targets[target].find(),
            key=lambda target: cast(str, targets[target].describe()["primary_key"][0]),
        ))

    def set(self, active: p.ActiveModel, value: str | None, key: Any = None) -> p.ActiveModel:
        """``active`` with its reference set to ``value``'s target of key
        ``key``: that target's column set to the key and every other to
        null, each null for ``set(active, None)`` where the reference is
        optional."""
        if not isinstance(active, p.ActiveModel):
            raise TypeError("set takes the entity's ActiveModel")
        for column, written in self._assigned(value, key).items():
            active = active.set(column, written)
        return active

    def __repr__(self) -> str:
        return f"OneOfEntity({self.name!r})"


def one_of_entity(name: str) -> OneOfEntity:
    """The ``#[derive(OneOf)]`` enum the native module registers as ``name``."""
    return OneOfEntity(p._one_of_entity(name))


def one_of_entities() -> list[str]:
    """The names of the one-of enums the native module registers, in order."""
    return p._one_of_entities()
