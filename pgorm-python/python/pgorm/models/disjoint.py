"""Disjoint models: a base model whose every row has a kind, and a model for
each kind that carries data of its own, keyed by the base row's key.

A ``Disjoint`` declares one over models; ``disjoint_entity(name)`` is a
downstream crate's ``#[derive(Disjoint)]`` enum the native module registers.
Either is pgorm's ``DisjointShape``, which builds every statement: the read
and its narrowing by key and kind, its pages and cursor, the writes and the
schema. Python only converts the values written and views each row read: the
base's record, the kind's value, and the kind's record or ``None``.
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any, Generic, Literal, TypeVar, cast, overload

from .. import _native as p
from .disjoint_query import DisjointWrite, _Disjoint
from .model import Model
from .record import ModelRecord

# Whether a disjoint model is written through: a kinded one is, an unkinded
# one, whose base holds no kind, is read-only, and its writes and schema take
# a self of the kinded type alone, so a type checker refuses them.
# [spec:pgorm:req:bindings.disjoint-unkinded]
W = TypeVar("W", Literal["kinded"], Literal["unkinded"], default=Literal["kinded"])

ModelRow = tuple[ModelRecord, str, ModelRecord | None]
EntityRow = tuple[p.EntityModel, str, p.EntityModel | None]

# A registration's row as a generated module types it, a union over its
# kinds, or an `EntityRow` where none does.
# [spec:pgorm:req:bindings.codegen-disjoint]
ER = TypeVar("ER", default=EntityRow)

# The unset `kind` option, told apart from one given as "kind".
_UNSET: Any = object()


def _named(fields: Mapping[str, Any], expected: Sequence[str], what: str) -> None:
    if not isinstance(fields, Mapping):
        raise TypeError(f"{what} is a mapping")
    if set(fields) != set(expected):
        raise p.ConstructionError(
            f"{what} names every primary-key part, {sorted(expected)}, and no other"
        )


def _model(value: object, what: str) -> Model:
    if not isinstance(value, Model):
        raise TypeError(f"{what} is a Model")
    if value.table.alias is not None:
        raise p.ConstructionError(
            f"{what} reads its table by its name: a disjoint read names each table bare"
        )
    return value


# [spec:pgorm:req:bindings.model-unique-keys+2]
def _table(model: Model) -> dict[str, Any]:
    """A model's table as the native shape is described from."""
    return {
        "schema": model.table.schema,
        "table": model.table.name,
        "columns": [
            {
                "name": column.name,
                "kind": column.kind,
                "array": column.array,
                "nullable": column.nullable,
                "default": column.default,
                "generated": column.generated,
                "values": None if column.values is None else list(column.values),
            }
            for column in model.columns.values()
        ],
        "key": [model._sql(field) for field in model.primary_key],
        "unique_keys": [
            (key.name, [model._sql(field) for field in key.fields], key.nulls_not_distinct)
            for key in model._unique_keys
        ],
    }


def _pairs(model: Model, values: Mapping[str, Any] | None) -> list[tuple[str, p.Value]] | None:
    """``values`` as the ``(column, Value)`` pairs a write gives the model's
    table, each converted through its field's declared kind."""
    if values is None:
        return None
    if not isinstance(values, Mapping):
        raise TypeError("a write's values are a mapping of fields")
    for field in values:
        if field not in model.columns:
            raise p.ConstructionError(f"{model.table.name} has no field {field!r}")
    return [
        (model._sql(field), column._value(values[field]))
        for field, column in model.columns.items()
        if field in values
    ]


# [spec:pgorm:req:bindings.disjoint+1]
# [spec:pgorm:req:bindings.disjoint-unkinded]
class Disjoint(_Disjoint[ModelRow], Generic[W]):
    """A disjoint model over ``base``: each row of its table has a kind,
    held in its ``kind`` field, and ``kinds`` maps each kind's value, in
    order, to the model of its table or to ``None`` for a kind with none.

    The kind type is ``enum_name``, ``{name}_kind`` by default, in the schema
    ``schema_name``, none by default; a model's enum column names its schema,
    so the base's kind field is of the kind type only when ``schema_name``
    names it. Declaring one sends nothing; what pgorm's shape refuses — no
    kinds, one model for two kinds, a kind model keyed by other types than
    the base, a kind field missing, nullable, generated or of another type, a
    kind model whose key the database fills — is a ``ConstructionError``
    raised here.

    ``unkinded=True`` declares a base with no kind field, a row's kind being
    the kind whose table has a row for it, ``None`` standing for at most one
    kind of a row with none: a ``Disjoint[Literal["unkinded"]]``, which is
    read-only, and beside which ``kind``, ``enum_name`` and ``schema_name``
    are refused."""

    __slots__ = ("base", "kinds", "kind_field")

    base: Model
    kinds: Mapping[str, Model | None]
    kind_field: str | None

    @overload
    def __init__(
        self: Disjoint[Literal["kinded"]],
        name: str,
        base: Model,
        kinds: Mapping[str, Model | None],
        *,
        kind: str = ...,
        enum_name: str | None = ...,
        schema_name: str | None = ...,
        unkinded: Literal[False] = ...,
    ) -> None: ...

    @overload
    def __init__(
        self: Disjoint[Literal["unkinded"]],
        name: str,
        base: Model,
        kinds: Mapping[str, Model | None],
        *,
        unkinded: Literal[True],
    ) -> None: ...

    def __init__(
        self,
        name: str,
        base: Model,
        kinds: Mapping[str, Model | None],
        *,
        kind: str = _UNSET,
        enum_name: str | None = None,
        schema_name: str | None = None,
        unkinded: bool = False,
    ) -> None:
        if type(name) is not str:
            raise TypeError("a disjoint model's name is a str")
        if type(unkinded) is not bool:
            raise TypeError("a disjoint model's unkinded is a bool")
        if unkinded and (kind is not _UNSET or enum_name is not None or schema_name is not None):
            raise p.ConstructionError(
                "an unkinded disjoint model's base holds no kind: it takes no kind, enum_name "
                "or schema_name"
            )
        kind = "kind" if kind is _UNSET else kind
        if type(kind) is not str:
            raise TypeError("a disjoint model's kind field is a str")
        base = _model(base, "a disjoint model's base")
        if not isinstance(kinds, Mapping):
            raise TypeError("a disjoint model's kinds map each kind's value to a Model or None")
        described: list[tuple[str, tuple[str, dict[str, Any]] | None]] = []
        held: dict[str, Model | None] = {}
        for value, model in kinds.items():
            if type(value) is not str:
                raise TypeError("a kind's value is a str")
            if model is not None:
                model = _model(model, f"the kind {value!r}'s model")
            held[value] = model
            described.append((value, None if model is None else (model.table.name, _table(model))))
        kinded: tuple[str, str, p.TypeName] | None = None
        if not unkinded:
            if kind not in base.columns:
                raise p.ConstructionError(
                    f"{base.table.name} has no field {kind!r}, the kind field a disjoint model's "
                    "base holds each row's kind in"
                )
            kinded = (
                base._sql(kind),
                kind,
                p.TypeName(f"{name}_kind" if enum_name is None else enum_name, schema=schema_name),
            )
        object.__setattr__(self, "_native", p._disjoint_model(
            name, (base.table.name, _table(base)), kinded, described,
        ))
        object.__setattr__(self, "base", base)
        object.__setattr__(self, "kinds", held)
        object.__setattr__(self, "kind_field", None if unkinded else kind)

    def __setattr__(self, name: str, value: object) -> None:
        raise AttributeError("a disjoint model never changes")

    def _row(self, item: Any) -> ModelRow:
        native, index, source = item
        value = self._native.kinds()[index]
        base = ModelRecord(
            self.base, native, tuple(self.base.columns), self._native.result_names(0)
        )
        model = self.kinds[value]
        if source is None or model is None:
            return base, value, None
        return base, value, ModelRecord(
            model, native, tuple(model.columns), self._native.result_names(source)
        )

    def _key(self, key: Mapping[str, Any]) -> list[Any]:
        _named(key, self.base.primary_key, "a key")
        return [self.base.columns[field]._value(key[field]) for field in self.base.primary_key]

    def _columns(self, fields: tuple[str, ...]) -> list[str]:
        for field in fields:
            self.base.col(field)
        return [self.base._sql(field) for field in fields]

    def _boundary(self, fields: tuple[str, ...], values: tuple[Any, ...]) -> list[Any]:
        return [self.base.columns[field]._value(value) for field, value in zip(fields, values)]

    def _row_values(self, kind: str, values: Mapping[str, Any] | None) -> list[tuple[str, p.Value]] | None:
        if type(kind) is not str:
            raise TypeError("a kind is its value, a str")
        model = self.kinds.get(kind)
        if model is None:
            if values is not None and not isinstance(values, Mapping):
                raise TypeError("a write's values are a mapping of fields")
            return None if values is None else []
        return _pairs(model, values)

    def insert(
        self: Disjoint[Literal["kinded"]],
        base: Mapping[str, Any],
        kind: str,
        values: Mapping[str, Any] | None = None,
    ) -> DisjointWrite[ModelRow]:
        """Insert a value of the kind ``kind``: the base row from ``base``,
        its kind field holding ``kind``, and for a kind with a table its row
        from ``values``, its key read from the inserted base row. The base's
        kind field and the kind row's key fields come from ``kind`` and from
        the base row, so a value given for either is refused."""
        return DisjointWrite(
            self, self._native.insert(_pairs(self.base, base), kind, self._row_values(kind, values))
        )

    def update(
        self: Disjoint[Literal["kinded"]],
        base: Mapping[str, Any],
        kind: str,
        values: Mapping[str, Any] | None = None,
    ) -> DisjointWrite[ModelRow]:
        """Update a value of the kind ``kind``: the fields ``base`` sets of the
        base row whose key it holds, and those ``values`` sets of its kind's
        row. A row of another kind, and a key no row has, read back nothing."""
        return DisjointWrite(
            self, self._native.update(_pairs(self.base, base), kind, self._row_values(kind, values))
        )

    def change_kind(
        self: Disjoint[Literal["kinded"]],
        key: Mapping[str, Any],
        kind: str,
        values: Mapping[str, Any] | None = None,
    ) -> DisjointWrite[ModelRow]:
        """Change the kind of the row of ``key`` to ``kind``, its row in the
        new kind's table, where it has one, from ``values``."""
        return DisjointWrite(
            self, self._native.change_kind(self._key(key), kind, self._row_values(kind, values))
        )

    def __repr__(self) -> str:
        return f"Disjoint({self.name!r})"


# [spec:pgorm:req:bindings.disjoint+1]
# [spec:pgorm:req:bindings.disjoint-unkinded]
class DisjointEntity(_Disjoint[ER], Generic[W, ER]):
    """A downstream crate's ``#[derive(Disjoint)]`` enum, which the native
    module registers with its base and kind entities: its records are the
    entities' own models, and a write takes their ``ActiveModel``s. An enum
    declared ``unkinded`` is a ``DisjointEntity[Literal["unkinded"]]``, which
    is read-only."""

    __slots__ = ()

    def __init__(self, native: p._Disjoint) -> None:
        if not isinstance(native, p._Disjoint):
            raise TypeError("a disjoint entity is one the native module registers")
        object.__setattr__(self, "_native", native)

    def __setattr__(self, name: str, value: object) -> None:
        raise AttributeError("a disjoint entity never changes")

    def _row(self, item: Any) -> ER:
        return cast(ER, tuple(item))

    def _key(self, key: Mapping[str, Any]) -> list[Any]:
        columns = self._native.key_columns()
        _named(key, columns, "a key")
        return [key[column] for column in columns]

    def _columns(self, fields: tuple[str, ...]) -> list[str]:
        """A registration's fields are its base's columns."""
        return list(fields)

    def _boundary(self, fields: tuple[str, ...], values: tuple[Any, ...]) -> list[Any]:
        """A registration's boundary values are converted natively, as its
        columns' kinds declare."""
        return list(values)

    def insert(
        self: DisjointEntity[Literal["kinded"], ER],
        base: p.ActiveModel,
        kind: str,
        active: p.ActiveModel | None = None,
    ) -> DisjointWrite[ER]:
        """Insert a value of the kind ``kind`` from the base entity's
        ``ActiveModel`` and, for a kind with a table, its entity's."""
        return DisjointWrite(self, self._native.insert(_active(base), kind, _active(active)))

    def update(
        self: DisjointEntity[Literal["kinded"], ER],
        base: p.ActiveModel,
        kind: str,
        active: p.ActiveModel | None = None,
    ) -> DisjointWrite[ER]:
        """Update a value of the kind ``kind``, as ``Disjoint.update`` does."""
        return DisjointWrite(self, self._native.update(_active(base), kind, _active(active)))

    def change_kind(
        self: DisjointEntity[Literal["kinded"], ER],
        key: Mapping[str, Any],
        kind: str,
        active: p.ActiveModel | None = None,
    ) -> DisjointWrite[ER]:
        """Change the kind of the row of ``key``, as ``Disjoint.change_kind``
        does."""
        return DisjointWrite(self, self._native.change_kind(self._key(key), kind, _active(active)))

    def __repr__(self) -> str:
        return f"DisjointEntity({self.name!r})"


def _active(active: Any) -> Any:
    """The native ActiveModel a write takes: a generated module's view of one
    holds it."""
    from .._registered.entities import ActiveView

    return active.native if isinstance(active, ActiveView) else active


def disjoint_entity(name: str) -> DisjointEntity[Any, Any]:
    """The disjoint enum the native module registers as ``name``: kinded or
    unkinded as the enum is declared, which a generated module types."""
    return DisjointEntity(p._disjoint_entity(name))


def disjoint_entities() -> list[str]:
    """The names of the disjoint enums the native module registers."""
    return p._disjoint_entities()
