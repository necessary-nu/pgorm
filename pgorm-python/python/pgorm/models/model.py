"""Application-defined model metadata, lowered through native statement builders."""

from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, replace
from types import MappingProxyType
from typing import TYPE_CHECKING, Any, Literal, Required, TypeAlias, TypedDict, cast

from .. import _native as p
from . import casts
from .columns import Column, ModelColumn

if TYPE_CHECKING:
    from .active import ModelActive, ModelHooks
    from .graphs import Graph
    from ..raw import SelectorRaw
    from .queries import ModelQuery
    from .record import ModelRecord
    from .relations import ConditionType, Linked, OnCondition, ReferentialAction, Related, Relation
    from .writes import ModelDelete, ModelInsert, ModelInsertMany, ModelUpdate


# The case styles a model's rename_all renames its columns by, as Rust's
# entity derive spells them.
CaseStyle: TypeAlias = Literal[
    "camelCase", "PascalCase", "kebab-case", "snake_case", "SCREAMING_SNAKE_CASE",
    "SCREAMING-KEBAB-CASE", "lowercase", "UPPERCASE", "title_case", "mixed_case",
]


# [spec:pgorm:req:bindings.model-unique-keys+2]
class UniqueKey(TypedDict, total=False):
    """A unique key of a model's table as it is declared: its fields in key
    order, the constraint's name, which PostgreSQL chooses when none is
    given, and whether nulls are equal under it."""

    fields: Required[str | Sequence[str]]
    name: str | None
    nulls_not_distinct: bool


# [spec:pgorm:req:bindings.model-unique-keys+2]
@dataclass(frozen=True, slots=True)
class _DeclaredKey:
    """A unique key a model declares, its fields checked against the
    model's."""

    name: str | None
    fields: tuple[str, ...]
    nulls_not_distinct: bool


_UNIQUE_OPTIONS = frozenset({"fields", "name", "nulls_not_distinct"})


# [spec:pgorm:req:bindings.model-unique-keys+2]
# [spec:pgorm:req:bindings.schema-from-model]
def _check_key(declarations: Mapping[str, Column], key: tuple[str, ...]) -> None:
    """Refuse a key the serial family or a period cannot be part of: a
    serial key is of one column, and a period is the key's last column and
    is preceded by another, as PostgreSQL requires of ``WITHOUT OVERLAPS``."""
    columns = [declarations[field] for field in key]
    if any(column.auto_increment for column in columns) and len(columns) != 1:
        raise p.ConstructionError("auto_increment draws a one-column key from the serial family")
    periods = [index for index, column in enumerate(columns) if column.without_overlaps]
    if periods and (periods != [len(columns) - 1] or len(columns) < 2):
        raise p.ConstructionError(
            "without_overlaps marks the last field of a key of two or more as its period"
        )


def _unique_key(key: object, declarations: Mapping[str, Column]) -> _DeclaredKey:
    """``key`` as the model whose fields are ``declarations`` declares it:
    refused as PostgreSQL or the Rust derive would refuse it, as the model is
    declared."""
    if not isinstance(key, Mapping):
        raise TypeError("a unique key is a mapping of its fields and options")
    unknown = sorted(str(option) for option in key if option not in _UNIQUE_OPTIONS)
    if unknown:
        raise TypeError(
            f"{unknown[0]!r} is no unique key option: a key takes fields, name and "
            "nulls_not_distinct"
        )
    given = key.get("fields", ())
    if isinstance(given, str):
        given = (given,)
    elif not isinstance(given, (list, tuple)):
        raise TypeError("a unique key's fields are a field or a list or tuple of them")
    fields: list[str] = []
    for field in given:
        if type(field) is not str:
            raise TypeError("a unique key's field is named by a string")
        if field not in declarations:
            raise p.ConstructionError(f"a unique key names {field!r}, which is no field of the model")
        if field in fields:
            raise p.ConstructionError(
                f"a unique key names {field!r} twice, which PostgreSQL refuses"
            )
        fields.append(field)
    if not fields:
        raise p.ConstructionError("a unique key names at least one field")
    name = key.get("name")
    if name is not None:
        name = p._identifier(name)
    nulls_not_distinct = key.get("nulls_not_distinct", False)
    if type(nulls_not_distinct) is not bool:
        raise TypeError("a unique key's nulls_not_distinct is a bool")
    return _DeclaredKey(name, tuple(fields), nulls_not_distinct)


# [spec:pgorm:req:bindings.models+6]
# [spec:pgorm:req:bindings.model-unique-keys+2]
# [spec:pgorm:req:bindings.schema-from-model]
@dataclass(frozen=True, eq=False, slots=True, init=False)
class Model:
    table: p.Table
    columns: Mapping[str, Column]
    primary_key: tuple[str, ...]
    comment: str | None
    _unique_keys: tuple[_DeclaredKey, ...]
    _hooks: Mapping[str, Callable[..., Any]]

    def __init__(
        self,
        table: str | p.Table,
        columns: Mapping[str, Column],
        *,
        schema: str | None = None,
        unique_keys: Sequence[UniqueKey] = (),
        comment: str | None = None,
        rename_all: CaseStyle | None = None,
        hooks: ModelHooks | None = None,
    ) -> None:
        from .active import hooks_of

        if isinstance(table, p.Table):
            if schema is not None:
                raise p.ConstructionError(
                    "schema is already part of the supplied Table"
                )
            native = table
        else:
            native = p.Table(table, schema=schema)
        if not isinstance(columns, Mapping):
            raise TypeError("a model's columns are a mapping of fields to Columns")
        if not columns:
            raise p.ConstructionError("model columns require a non-empty mapping")
        if comment is not None and type(comment) is not str:
            raise TypeError("a model's comment is a string")
        if rename_all is not None and type(rename_all) is not str:
            raise TypeError("a model's rename_all is a case style's name")
        declarations = {}
        physical = set()
        for identity, column in columns.items():
            identity = p._identifier(identity)
            if identity in declarations:
                raise p.ConstructionError("duplicate model field identity")
            if not isinstance(column, Column):
                raise TypeError("each model field is a Column declaration")
            if column.name is None:
                name = identity if rename_all is None else p._renamed(identity, rename_all)
                column = replace(column, name=name)
            if column.name in physical:
                raise p.ConstructionError(
                    "multiple model fields map to the same SQL column"
                )
            declarations[identity] = column
            physical.add(column.name)
        object.__setattr__(self, "table", native)
        object.__setattr__(self, "columns", MappingProxyType(declarations))
        key = tuple(name for name, column in declarations.items() if column.primary_key)
        _check_key(declarations, key)
        object.__setattr__(self, "primary_key", key)
        object.__setattr__(self, "comment", comment)
        if not isinstance(unique_keys, (list, tuple)):
            raise TypeError("a model's unique_keys are a list or tuple of keys")
        object.__setattr__(
            self,
            "_unique_keys",
            tuple(_unique_key(key, declarations) for key in unique_keys),
        )
        object.__setattr__(self, "_hooks", hooks_of(hooks))
        p._model_table(self._described())

    @property
    def qualifier(self) -> str:
        """What the model's columns are qualified by: its alias, or else its
        table's name, never its schema, as pgorm qualifies an entity's."""
        return self.table.alias or self.table.name

    def as_(self, alias: str) -> Model:
        aliased = Model(self.table.as_(alias), self.columns, comment=self.comment)
        object.__setattr__(aliased, "_unique_keys", self._unique_keys)
        object.__setattr__(aliased, "_hooks", self._hooks)
        return aliased

    def _described(self) -> dict[str, Any]:
        """The model's table as the native table shape is described from:
        its columns, key, unique keys and comment."""
        key = [self.columns[field] for field in self.primary_key]
        return {
            "schema": self.table.schema,
            "table": self.table.name,
            "columns": [column._described(self._sql(name)) for name, column in self.columns.items()],
            "key": [self._sql(field) for field in self.primary_key],
            "auto_increment": any(column.auto_increment for column in key),
            "without_overlaps": any(column.without_overlaps for column in key),
            "unique_keys": [
                (unique.name, [self._sql(field) for field in unique.fields], unique.nulls_not_distinct)
                for unique in self._unique_keys
            ],
            "comment": self.comment,
        }

    def col(self, field: str) -> ModelColumn:
        return self._column(field, self.qualifier)

    def _column(self, field: str, qualifier: str) -> ModelColumn:
        name = p._identifier(field)
        if name not in self.columns:
            raise p.ConstructionError(f"{self.table.name} has no field {name!r}")
        return ModelColumn(name, self.columns[name], p.col(self._sql(name), table=qualifier))

    def _sql(self, field: str) -> str:
        return cast(str, self.columns[field].name)

    def _projected(
        self, field: str, column: p.Expr, name: str | None = None
    ) -> p.Expr | p.Aliased:
        """``column`` projected under ``name``, the field's own by default:
        as it is when that is its column's name, aliased otherwise, and cast
        to ``text``, or ``text[]`` for an array, when the field is read as its
        text."""
        name = field if name is None else name
        return casts.projected(self.columns[field], column, name, self._sql(field))

    def _selection(self, fields: tuple[str, ...]) -> tuple[str, ...]:
        selected = tuple(self.columns) if not fields else fields
        names = tuple(self.col(name).field_name for name in selected)
        if len(set(names)) != len(names):
            raise p.ConstructionError("duplicate model output identity")
        return names

    def select(self, *fields: str) -> ModelQuery:
        from .clauses import Recipe
        from .queries import ModelQuery

        return ModelQuery(Recipe(self, self._selection(fields)))

    def find(self) -> ModelQuery:
        return self.select()

    def find_by_statement(self, statement: p.RawSQL) -> SelectorRaw[ModelRecord]:
        """``statement``'s rows decoded into this model's records, as Rust's
        ``FromQueryResult::find_by_statement`` decodes them: each field read
        from the column of its SQL name and held to its declared kind."""
        from ..raw import SelectorRaw

        return SelectorRaw(statement, self)

    def find_by_key(self, values: Mapping[str, Any]) -> ModelQuery:
        """The row whose primary key ``values`` gives."""
        return self.find().where(self.key(values))

    def key(self, values: Mapping[str, Any]) -> p.Condition:
        if not self.primary_key:
            raise p.ConstructionError("model has no declared primary key")
        if not isinstance(values, Mapping):
            raise TypeError("a key is a mapping of the key's fields")
        if set(values) != set(self.primary_key):
            raise p.ConstructionError(
                "key lookup requires every declared primary-key field and no others"
            )
        return p.Condition.all(
            *(self.col(name).eq(values[name]) for name in self.primary_key)
        )

    def active(self, values: Mapping[str, Any] | None = None) -> ModelActive:
        """The fields one record write sets, whose ``insert``, ``update`` and
        ``delete`` run the model's hooks around the write, as a Rust
        ``ActiveModel``'s do."""
        from .active import ModelActive

        return ModelActive(self, {} if values is None else values)

    def insert(self, values: Mapping[str, Any]) -> ModelInsert:
        """An INSERT of one row, Rust's ``Insert::one``."""
        from .writes import ModelInsert, appended

        statement, fields = appended(self, None, None, [values])
        return ModelInsert(self, statement, fields)

    def insert_many(self, rows: Sequence[Mapping[str, Any]]) -> ModelInsertMany:
        """An INSERT of rows that each set the same fields, Rust's
        ``Insert::many``; no rows writes nothing and sends nothing."""
        from .writes import ModelInsertMany, appended

        if not isinstance(rows, (list, tuple)):
            raise TypeError("insert_many takes a list or tuple of rows")
        statement, fields = appended(self, None, None, rows)
        return ModelInsertMany(self, statement, fields)

    def update(self, values: Mapping[str, Any]) -> ModelUpdate:
        from .writes import updated

        return updated(self, values)

    def delete(self) -> ModelDelete:
        """A DELETE, which needs a ``where`` or ``all_rows()`` before it
        runs."""
        from .writes import ModelDelete

        return ModelDelete(self, p.Delete(self.table))

    def belongs_to(
        self,
        target: Model,
        *,
        from_: str | Sequence[str],
        to: str | Sequence[str],
        on_delete: ReferentialAction | None = None,
        on_update: ReferentialAction | None = None,
        fk_name: str | None = None,
        on_condition: OnCondition | None = None,
        condition_type: ConditionType = "all",
    ) -> Relation:
        """A relation from this model's foreign key to the key of ``target``,
        whose actions and constraint name the table's foreign key takes when
        ``schema.from_model`` writes it. ``on_condition``, given the two ends
        as a join names them, adds a condition of its own, ANDed with the
        fields' equality, or ORed when ``condition_type`` is ``"any"``."""
        from .relations import Relation

        return Relation(
            "belongs_to",
            self,
            target,
            from_,
            to,
            on_delete=on_delete,
            on_update=on_update,
            fk_name=fk_name,
            on_condition=on_condition,
            condition_type=condition_type,
        )

    def has_one(
        self,
        target: Model,
        *,
        from_: str | Sequence[str],
        to: str | Sequence[str],
        on_condition: OnCondition | None = None,
        condition_type: ConditionType = "all",
    ) -> Relation:
        """A relation from this model's key to at most one row of ``target``."""
        from .relations import Relation

        return Relation(
            "has_one", self, target, from_, to, on_condition=on_condition, condition_type=condition_type
        )

    def has_many(
        self,
        target: Model,
        *,
        from_: str | Sequence[str],
        to: str | Sequence[str],
        on_condition: OnCondition | None = None,
        condition_type: ConditionType = "all",
    ) -> Relation:
        """A relation from this model's key to any number of rows of ``target``."""
        from .relations import Relation

        return Relation(
            "has_many", self, target, from_, to, on_condition=on_condition, condition_type=condition_type
        )

    def related(self, relation: Relation, *, via: Relation) -> Related:
        """The path from this model through a junction, as a Rust ``Related``
        implementation with a ``via`` declares it: ``via`` from this model to
        the junction, then ``relation`` from the junction to the far end."""
        from .relations import Related

        return Related(self, relation, via)

    def linked(self, *relations: Relation) -> Linked:
        """A chain of relations from this model, each starting where the one
        before it ends, as a Rust ``Linked`` type lists them."""
        from .relations import Linked

        return Linked(self, relations)

    def graph(self) -> Graph[Literal[False]]:
        """A graph rooted at this model, decoding its records alone until a
        slot is joined."""
        from .graphs import Graph, State, Table

        return Graph(State((Table(self, self.qualifier, "root", False),)))

    def describe(self) -> dict[str, Any]:
        """The declaration as data: the table, its primary key and unique
        keys, and each field's column with every option it was declared
        with, its kind spelled as a decoded column's is compared: ``i64``,
        ``text[]``, ``enum "app"."mood"``."""
        return {
            "schema": self.table.schema,
            "table": self.table.name,
            "alias": self.table.alias,
            "primary_key": list(self.primary_key),
            "unique_keys": [
                {
                    "name": key.name,
                    "fields": list(key.fields),
                    "nulls_not_distinct": key.nulls_not_distinct,
                }
                for key in self._unique_keys
            ],
            "hooks": list(self._hooks),
            "fields": {
                name: {
                    "column": column.name,
                    "kind": column._null._kind_key(),
                    "nullable": column.nullable,
                    "primary_key": column.primary_key,
                    "default": column.default,
                    "generated": column.generated,
                    "values": None if column.values is None else list(column.values),
                }
                for name, column in self.columns.items()
            },
        }
