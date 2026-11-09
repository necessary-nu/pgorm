"""Declared models generated from DDL: pgorm-codegen's Python writer, which
reads a schema as it generates Rust entities and writes the module of
``Model`` declarations that describe it."""

from collections.abc import Sequence
from typing import NotRequired, TypedDict

from .. import _native as p

# A table, by its name or by its schema and name.
Table = str | tuple[str, str]


class UnkindedDisjoint(TypedDict):
    """A base table and its kinds' tables, read as an unkinded disjoint
    model, as pgorm-codegen's ``UnkindedDisjoint`` names one: the base, each
    kind's table in the order its kinds take, and the value of the kind a
    base row with a row in no kind's table is, if such a row is one."""

    base: Table
    kinds: Sequence[Table]
    unit: NotRequired[str | None]


def _table(table: object, what: str) -> tuple[str | None, str]:
    if type(table) is str:
        return (None, table)
    if (
        type(table) is tuple
        and len(table) == 2
        and all(type(part) is str for part in table)
    ):
        return (table[0], table[1])
    raise TypeError(f"{what} is a table's name, or a (schema, name) pair")


# [spec:pgorm:req:bindings.models-from-sql]
def models_from_sql(
    ddl: str,
    *,
    schema_name: str | None = None,
    unkinded_disjoint: Sequence[UnkindedDisjoint] = (),
) -> str:
    """The Python module declaring the models of the tables ``ddl`` creates,
    as pgorm-codegen reads them for Rust entities: each table a ``Model``,
    each foreign key a ``belongs_to`` relation and the other side's
    ``has_one`` or ``has_many``, each junction a ``related`` path, and the
    tables of a disjoint enum and a one-of reference's ``CHECK`` their
    ``Disjoint`` and ``OneOf``. ``schema_name`` is the schema an unqualified
    table and type are in, ``public`` for a type by default.

    DDL pgorm-codegen refuses, and anything a declared model cannot say, is a
    ``ConstructionError`` naming it."""
    if type(ddl) is not str:
        raise TypeError("models_from_sql reads DDL text, a str")
    if schema_name is not None and type(schema_name) is not str:
        raise TypeError("a schema_name is a str")
    sets = []
    for entry in unkinded_disjoint:
        if not isinstance(entry, dict) or set(entry) - {"base", "kinds", "unit"}:
            raise TypeError("an unkinded disjoint set is a dict of base, kinds and unit")
        kinds = entry.get("kinds", ())
        if isinstance(kinds, str) or not isinstance(kinds, Sequence):
            raise TypeError("an unkinded disjoint set's kinds are a sequence of tables")
        unit = entry.get("unit")
        if unit is not None and type(unit) is not str:
            raise TypeError("an unkinded disjoint set's unit is a str")
        sets.append((
            _table(entry.get("base"), "an unkinded disjoint set's base"),
            [_table(kind, "an unkinded disjoint set's kind") for kind in kinds],
            unit,
        ))
    return p._models_from_sql(ddl, schema_name, sets)
