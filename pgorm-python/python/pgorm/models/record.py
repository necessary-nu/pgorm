"""Declared-field views over native PostgreSQL records, and the decode of a
statement's rows into them."""

from __future__ import annotations

from collections.abc import Iterator, Mapping, Sequence
from dataclasses import dataclass
from types import MappingProxyType
from typing import TYPE_CHECKING, Any, Callable, TypeAlias

from .. import _native as p
from . import casts

if TYPE_CHECKING:
    from ..results import RowStream
    from ..runtime import Connection, Pool
    from ..transactions import Transaction
    from .model import Model

Database: TypeAlias = "Connection | Pool | Transaction"
Row: TypeAlias = "ModelRecord | tuple[ModelRecord | None, ...] | p.Record"
# A row's decode: its sources' records, or what a projection or a raw
# statement reads it as.
Decode: TypeAlias = "Sequence[Source] | Callable[[p.Record], Any]"


# [spec:pgorm:req:bindings.model-records+1]
# [spec:pgorm:req:python.records+1]
@dataclass(frozen=True, eq=False, slots=True, init=False)
class ModelRecord(Mapping[str, Any]):
    """A row's fields of one model, keyed by field in declaration order.

    Each field is read from the result column ``columns`` names for it, the
    field's own name by default; the native row may carry other sources'
    columns beside them, as a graph's row does. A field the model projects
    as its text, a created multirange's, holds the value that text spells.
    """

    model: Model
    native: p.Record
    selected: tuple[str, ...]
    columns: tuple[str, ...]
    read: Mapping[str, p.Value]
    _values: Mapping[str, Any]

    def __init__(
        self,
        model: Model,
        native: p.Record,
        selected: Sequence[str],
        columns: Sequence[str] | None = None,
    ) -> None:
        from .model import Model

        if not isinstance(model, Model) or not isinstance(native, p.Record):
            raise TypeError("a model record is a Model's view of a native Record")
        fields = model._selection(tuple(selected))
        if columns is None:
            names = fields
            if native.keys() != names:
                raise p.DecodeError(
                    "record columns differ from the declared model selection"
                )
        else:
            names = tuple(columns)
            if len(names) != len(fields):
                raise p.ConstructionError("a record reads one column per field")
            for name in names:
                if name not in native:
                    raise p.DecodeError(f"the result has no column {name!r}")
        read = {}
        values = {}
        for name, column in zip(fields, names):
            declared = model.columns[name]
            value = declared._decoded(native.tagged(column))
            if declared._read_as_text:
                read[name] = value
            if declared._read_as_text or declared.decode is not None:
                values[name] = casts.read(declared, value)
        object.__setattr__(self, "model", model)
        object.__setattr__(self, "native", native)
        object.__setattr__(self, "selected", fields)
        object.__setattr__(self, "columns", names)
        object.__setattr__(self, "read", MappingProxyType(read))
        object.__setattr__(self, "_values", MappingProxyType(values))

    def __getitem__(self, name: str) -> Any:
        if name in self._values:
            return self._values[name]
        return self.native[self._column(name)]

    def __iter__(self) -> Iterator[str]:
        return iter(self.selected)

    def __len__(self) -> int:
        return len(self.selected)

    def tagged(self, name: str) -> p.Value:
        if name in self.read:
            return self.read[name]
        return self.native.tagged(self._column(name))

    def _column(self, name: str) -> str:
        if name not in self.selected:
            raise KeyError(name)
        return self.columns[self.selected.index(name)]

    def __repr__(self) -> str:
        return f"ModelRecord({dict(self)!r})"


@dataclass(frozen=True, slots=True)
class Source:
    """One model's fields in a result: the column each is read from, and
    whether the source may be absent, as a LEFT JOIN's is."""

    model: Model
    fields: tuple[str, ...]
    names: tuple[str, ...]
    optional: bool = False


def own_source(model: Model, fields: tuple[str, ...]) -> Source:
    """The source of a model's own projection, each field under its name."""
    return Source(model, fields, fields)


def decoder(sources: Decode) -> Callable[[p.Record], Any]:
    """The decode of a native row into its one source's record, or a tuple of
    each source's, the row's columns held to exactly the sources' names; a
    projection's decode as it is."""
    if callable(sources):
        return sources
    expected = tuple(name for source in sources for name in source.names)

    def decode(row: p.Record) -> Any:
        if row.keys() != expected:
            raise p.DecodeError(
                f"the result's columns {list(row.keys())} are not the declared "
                f"fields' {list(expected)}"
            )
        records = tuple(_record(row, source) for source in sources)
        return records[0] if len(records) == 1 else records

    return decode


def _record(row: p.Record, source: Source) -> ModelRecord | None:
    # An optional source whose every column is NULL is the row of a LEFT JOIN
    # that matched nothing, as pgorm's absence witness reads it.
    if source.optional and all(row.tagged(name).is_null for name in source.names):
        return None
    return ModelRecord(source.model, row, source.fields, source.names)


async def decode_all(db: Database, statement: Any, sources: Decode) -> list[Any]:
    decode = decoder(sources)
    return [decode(row) for row in await db.query(statement)]


async def decode_single(
    db: Database, statement: Any, sources: Decode, *, optional: bool
) -> Any:
    """The one row ``statement`` reads, decoded: exactly one, or with
    ``optional`` at most one, None when it reads none."""
    if not optional:
        return decoder(sources)(await db.one(statement))
    row = await db.optional(statement)
    return None if row is None else decoder(sources)(row)


async def decode_stream(
    db: Database, statement: Any, sources: Decode
) -> RowStream[Any]:
    from ..runtime import Connection, Pool
    from ..transactions import Transaction

    if not isinstance(db, (Connection, Pool, Transaction)):
        raise TypeError("a model's rows stream from a Pool, a Connection or a Transaction")
    stream = await db.stream(statement)
    return stream._decoding(decoder(sources))


async def decode_count(db: Database, statement: p.Select) -> int:
    """The rows ``statement`` reads, counted as pgorm's paginator counts."""
    return int((await db.one(p._model_count(statement)))["num_items"])
