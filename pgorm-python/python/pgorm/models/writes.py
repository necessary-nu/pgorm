"""A model's writes: each value converted through its field's declared kind, a
field left out kept apart from one set to NULL, and the RETURNING list that
reads a written row's two versions as pgorm's ``exec_returning_change(s)``
and ``exec_returning_upsert(s)`` read them.

An insert, an update and a delete are each a class of its own holding only
the methods its statement takes, so a type checker refuses an insert's
``where`` or a delete's ``returning_change`` as it refuses any attribute a
class lacks."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass, replace
from typing import TYPE_CHECKING, Any, ClassVar, Literal, Self

from .. import _native as p
from .._registered.versions import Change, Inserted, Updated
from .clauses import predicate_of
from .queries import ModelRows, written
from .record import Database, ModelRecord, Source, decode_all, decode_single

if TYPE_CHECKING:
    from .columns import Column
    from .model import Model

# The names a RETURNING list reads a written row's versions by: always
# renamed, as pgorm renames them, so a table called old or new cannot take
# the keyword.
OLD, NEW = "pgorm_old", "pgorm_new"


@dataclass(frozen=True, eq=False, slots=True)
class _Write[S: p.Insert | p.Update | p.Delete | None]:
    """What every model write holds: its model and its native statement,
    which ``statement`` exposes, and the terminals every write has. Only an
    insert of no rows has no statement, and it sends nothing."""

    model: Model
    statement: S
    _statements: ClassVar[tuple[type, ...]]
    _made_by: ClassVar[str]

    def __post_init__(self) -> None:
        from .model import Model

        if not isinstance(self.model, Model) or not isinstance(
            self.statement, self._statements
        ):
            raise TypeError(f"a {type(self).__name__} comes from a model's {self._made_by}")

    def returning(self, *fields: str) -> ModelRows:
        """A RETURNING list of ``fields``, every field when it names none,
        whose terminals decode each written row as a record."""
        model = self.model
        selected = model._selection(fields)
        if self.statement is None:
            return ModelRows(model, None, selected)
        columns = (model._projected(name, p.col(model._sql(name))) for name in selected)
        return ModelRows(model, self.statement.returning(*columns), selected)

    async def execute(self, db: Database) -> int:
        """How many rows the write wrote."""
        if self.statement is None:
            return 0
        return await db.execute(self.statement)

    def __bool__(self) -> bool:
        raise p.ConstructionError("model writes cannot be tested as Python booleans")


@dataclass(frozen=True, eq=False, slots=True)
class _Guarded[S: (p.Update, p.Delete)](_Write[S]):
    """A write of the rows a condition picks, which needs ``where`` or
    ``all_rows()`` before it runs, as the statement builders' writes do."""

    def where(self, predicate: p.Expr | p.Condition) -> Self:
        return replace(self, statement=self.statement.where(predicate_of(predicate)))

    def all_rows(self) -> Self:
        return replace(self, statement=self.statement.all_rows())

    def inspect(self) -> p.Compiled:
        """The SQL and values the write runs."""
        return self.statement.inspect()


@dataclass(frozen=True, eq=False, slots=True)
class _Insert(_Write[p.Insert | None]):
    """What a model's INSERT of one row and of a batch share: its rows, the
    fields they set, and its conflict clause. ``fields`` is ``None`` for an
    insert of no rows and empty for a row of defaults."""

    fields: tuple[str, ...] | None
    _statements = (p.Insert, type(None))

    def on_conflict(self, action: p.Conflict | p.ConflictUpdate) -> Self:
        """What a conflict does, the statement builders' ``Conflict`` naming
        SQL columns."""
        if not isinstance(action, (p.Conflict, p.ConflictUpdate)):
            raise TypeError(
                "a conflict's action is Conflict.do_nothing(), or a target's do_nothing() or update(..)"
            )
        return replace(self, statement=written(self.statement).on_conflict(action))

    def add(self, values: Mapping[str, Any]) -> ModelInsertMany:
        """The insert with one more row, which makes it a batch, as Rust's
        ``Insert::add`` does; its conflict clause is kept."""
        return self.add_many([values])

    def add_many(self, rows: Sequence[Mapping[str, Any]]) -> ModelInsertMany:
        """The insert with ``rows`` after its own, a batch, as Rust's
        ``Insert::add_many`` makes one."""
        if not isinstance(rows, (list, tuple)):
            raise TypeError("add_many takes a list or tuple of rows")
        statement, fields = appended(self.model, self.statement, self.fields, rows)
        return ModelInsertMany(self.model, statement, fields)

    def _inspect(self, terminal: str | None, upserts: str) -> p.Compiled:
        if terminal is None:
            return written(self.statement).inspect()
        if terminal != upserts:
            raise TypeError(f'an insert inspects itself, or its "{upserts}"')
        return written(self._upserts()[0]).inspect()

    def _upserts(self) -> tuple[p.Insert | None, tuple[Source, Source]]:
        items, sources = versions(self.model)
        if self.statement is None:
            return None, sources
        return self.statement.returning(*items, old_as=OLD, new_as=NEW), sources


# [spec:pgorm:req:bindings.model-writes+2]
@dataclass(frozen=True, eq=False, slots=True)
class ModelInsert(_Insert):
    """A model's INSERT of one row, made by its ``insert()``: Rust's
    ``Insert<A, OneRow>``, whose upsert terminal answers for that row."""

    _made_by = "insert()"

    def inspect(self, *, terminal: Literal["returning_upsert"] | None = None) -> p.Compiled:
        """The SQL and values the insert runs, or the ones its upsert
        terminal runs."""
        return self._inspect(terminal, "returning_upsert")

    async def returning_upsert(
        self, db: Database
    ) -> Inserted[ModelRecord] | Updated[ModelRecord] | None:
        """What the insert did with its row, as ``exec_returning_upsert``
        says, or None where its conflict clause wrote nothing."""
        statement, sources = self._upserts()
        pair = await decode_single(db, written(statement), sources, optional=True)
        return None if pair is None else upserted(pair)


# [spec:pgorm:req:bindings.model-writes+2]
@dataclass(frozen=True, eq=False, slots=True)
class ModelInsertMany(_Insert):
    """A model's INSERT of a batch, made by its ``insert_many()`` or an
    insert's ``add``: Rust's ``Insert<A, ManyRows>``, whose upsert terminal
    answers for every row it writes."""

    _made_by = "insert_many(), or an insert's add() or add_many()"

    def inspect(self, *, terminal: Literal["returning_upserts"] | None = None) -> p.Compiled:
        """The SQL and values the insert runs, or the ones its upsert
        terminal runs."""
        return self._inspect(terminal, "returning_upserts")

    async def returning_upserts(
        self, db: Database
    ) -> list[Inserted[ModelRecord] | Updated[ModelRecord]]:
        """What the insert did with each row it wrote, as
        ``exec_returning_upserts`` says; a row its conflict clause held back
        is left out, and a batch of none sends nothing."""
        statement, sources = self._upserts()
        if statement is None:
            return []
        return [upserted(pair) for pair in await decode_all(db, statement, sources)]


# [spec:pgorm:req:bindings.model-writes+2]
@dataclass(frozen=True, eq=False, slots=True)
class ModelUpdate(_Guarded[p.Update]):
    """A model's UPDATE, made by its ``update()``."""

    _statements = (p.Update,)
    _made_by = "update()"

    def inspect(
        self, *, terminal: Literal["returning_change", "returning_changes"] | None = None
    ) -> p.Compiled:
        """The SQL and values the update runs, or the ones its change
        terminals run."""
        if terminal is None:
            return self.statement.inspect()
        if terminal not in ("returning_change", "returning_changes"):
            raise TypeError(
                'an update inspects itself, or its "returning_change" or "returning_changes"'
            )
        return self._changes()[0].inspect()

    async def returning_change(self, db: Database) -> Change[ModelRecord]:
        """The one row the update wrote, before and after; any other count
        is a ``DecodeError``."""
        statement, sources = self._changes()
        return change(await decode_single(db, statement, sources, optional=False))

    async def returning_changes(self, db: Database) -> list[Change[ModelRecord]]:
        """Every row the update wrote, before and after."""
        statement, sources = self._changes()
        return [change(pair) for pair in await decode_all(db, statement, sources)]

    def _changes(self) -> tuple[p.Update, tuple[Source, Source]]:
        items, sources = versions(self.model)
        return self.statement.returning(*items, old_as=OLD, new_as=NEW), sources


# [spec:pgorm:req:bindings.model-writes+2]
@dataclass(frozen=True, eq=False, slots=True)
class ModelDelete(_Guarded[p.Delete]):
    """A model's DELETE, made by its ``delete()``."""

    _statements = (p.Delete,)
    _made_by = "delete()"


def assignments(
    model: Model, values: Mapping[str, Any], *, insert: bool
) -> tuple[tuple[str, p.Value | p.Expr], ...]:
    """The SQL column and value of each field ``values`` sets, in declaration
    order whatever order it names them in."""
    if not isinstance(values, Mapping):
        raise TypeError("a model write takes a mapping of the fields it sets")
    for name in values:
        field = model.col(name).field_name
        if model.columns[field].generated == "always":
            raise p.ConstructionError(f"{field} is generated always: the server writes it")
    if insert:
        for field, column in model.columns.items():
            if not column.omissible and field not in values:
                raise p.ConstructionError(
                    f"{field} is required: it is neither nullable, defaulted nor generated"
                )
    return tuple(
        (model._sql(field), _value(column, values[field]))
        for field, column in model.columns.items()
        if field in values
    )


def _value(column: Column, value: Any) -> p.Value | p.Expr:
    return value if isinstance(value, p.Expr) else column._value(value)


def appended(
    model: Model,
    statement: p.Insert | None,
    fields: tuple[str, ...] | None,
    rows: Sequence[Mapping[str, Any]],
) -> tuple[p.Insert | None, tuple[str, ...] | None]:
    """``statement``, the INSERT of rows setting ``fields``, with ``rows``
    after them, each setting the same fields; the first row of an insert of
    none names them."""
    if model.table.alias is not None:
        raise p.ConstructionError("model inserts require an unaliased table")
    for row in rows:
        set_ = assignments(model, row, insert=True)
        names = tuple(column for column, _ in set_)
        if statement is None or fields is None:
            statement = p.Insert(model.table)
            if not names:
                statement, fields = statement.default_values(), ()
                continue
            statement, fields = statement.columns(*names), names
        elif not fields:
            raise p.ConstructionError("rows that set no field are inserted one at a time")
        elif names != fields:
            raise p.ConstructionError("every row of an insert sets the same fields")
        statement = statement.values(*(value for _, value in set_))
    return statement, fields


def updated(model: Model, values: Mapping[str, Any]) -> ModelUpdate:
    """An UPDATE of the fields ``values`` sets, which needs a ``where`` or
    ``all_rows()`` before it runs."""
    set_ = assignments(model, values, insert=False)
    if not set_:
        raise p.ConstructionError("model update requires at least one assigned field")
    statement = p.Update(model.table)
    for column, value in set_:
        statement = statement.set(column, value)
    return ModelUpdate(model, statement)


# [spec:pgorm:req:bindings.model-writes+2]
def versions(model: Model) -> tuple[list[p.Expr | p.Aliased], tuple[Source, Source]]:
    """Every field of both versions for a ``RETURNING WITH (OLD AS
    pgorm_old, NEW AS pgorm_new)`` list, the old under ``o_`` and the new
    under ``n_``, and the sources each decodes as."""
    fields = tuple(model.columns)
    columns = [model._sql(name) for name in fields]
    sources: list[Source] = []
    items = []
    for row, prefix in ((OLD, "o_"), (NEW, "n_")):
        names = tuple(p._model_result_names(prefix, columns))
        sources.append(Source(model, fields, names, optional=row == OLD))
        items += [
            model._projected(field, p.col(column, table=row), name)
            for field, column, name in zip(fields, columns, names)
        ]
    return items, (sources[0], sources[1])


def change(pair: tuple[ModelRecord | None, ModelRecord]) -> Change[ModelRecord]:
    old, new = pair
    if old is None:
        raise p.DecodeError("an updated row returned no old version")
    return Change(old, new)


def upserted(
    pair: tuple[ModelRecord | None, ModelRecord],
) -> Inserted[ModelRecord] | Updated[ModelRecord]:
    old, new = pair
    if old is None:
        return Inserted(new)
    return Updated(Change(old, new))
