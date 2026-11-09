"""Raw SQL's reads beyond its terminals: rows decoded into a declared model's
records or read as records, a statement checked against a model before any
row exists, and pages and counts over a ``SELECT`` written by hand."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any, Generic, TypeVar, cast

from . import _native as p
from .models.cursors import Pageable, Paginator
from .models.model import Model
from .models.record import ModelRecord

if TYPE_CHECKING:
    from .models.record import Database
    from .results import Record, RowStream
    from .runtime import Connection, Pool
    from .transactions import Transaction

R = TypeVar("R")


def _statement(statement: object) -> p.RawSQL:
    if not isinstance(statement, p.RawSQL):
        raise TypeError("a raw statement is a RawSQL")
    return statement


def _model(model: object) -> Model:
    if not isinstance(model, Model):
        raise TypeError("a raw statement decodes into a declared Model")
    return model


def _record(model: Model, row: Record) -> ModelRecord:
    """``row`` as ``model``'s record, each field read from the column of its
    SQL name, as Rust's ``FromQueryResult`` reads a row by column name."""
    fields = tuple(model.columns)
    return ModelRecord(model, row, fields, tuple(model._sql(field) for field in fields))


def _pageable(statement: p.RawSQL, decode: Any) -> Pageable:
    wrapped = p._raw_select(statement)
    return Pageable(
        page=lambda size, offset: wrapped.limit(size).offset(offset),
        counted=p._model_count(wrapped),
        sources=() if decode is None else decode,
    )


# [spec:pgorm:req:bindings.raw-models]
class SelectorRaw(Generic[R]):
    """A raw statement whose rows decode into a declared model's records, as
    Rust's ``SelectorRaw`` decodes them, each field read from the column of
    its SQL name and held to the kind it declares, other columns left unread;
    or, made by ``from_statement``, are read as records. pgorm adds nothing to
    the SQL, so ``one`` and ``optional`` add no ``LIMIT``."""

    __slots__ = ("statement", "model")
    statement: p.RawSQL
    model: Model | None

    def __init__(self, statement: p.RawSQL, model: Model | None) -> None:
        object.__setattr__(self, "statement", _statement(statement))
        object.__setattr__(self, "model", None if model is None else _model(model))

    def __setattr__(self, name: str, value: object) -> None:
        raise AttributeError("a SelectorRaw is immutable")

    def inspect(self) -> p.Compiled:
        return self.statement.inspect()

    def _decode(self, row: Record) -> R:
        return cast(R, row if self.model is None else _record(self.model, row))

    async def all(self, db: Database) -> list[R]:
        return [self._decode(row) for row in await db.query(self.statement)]

    async def one(self, db: Database) -> R:
        return self._decode(await db.one(self.statement))

    async def optional(self, db: Database) -> R | None:
        row = await db.optional(self.statement)
        return None if row is None else self._decode(row)

    async def stream(self, db: Database) -> RowStream[R]:
        stream = await db.stream(self.statement)
        return stream._decoding(self._decode)

    # [spec:pgorm:req:bindings.raw-pages]
    def paginate(self, page_size: int) -> Paginator:
        """Pages of the statement's rows, the statement read as a subquery,
        refused unless it is one row-returning ``SELECT``."""
        return Paginator(_pageable(self.statement, self._decode), page_size)

    async def count(self, db: Database) -> int:
        """How many rows the statement returns, counted over it as a subquery."""
        counted = p._model_count(p._raw_select(self.statement))
        return int((await db.one(counted))["num_items"])

    def __bool__(self) -> bool:
        raise p.ConstructionError("raw queries cannot be tested as Python booleans")

    def __repr__(self) -> str:
        target = "records" if self.model is None else self.model.table.name
        return f"SelectorRaw({target!r})"


# [spec:pgorm:req:bindings.raw-models]
def from_statement(statement: p.RawSQL) -> SelectorRaw[Record]:
    """``statement`` as a raw read whose rows are records, which pages and
    counts, as Rust's ``SelectorRaw::from_statement`` holds one."""
    return SelectorRaw(statement, None)


# [spec:pgorm:req:bindings.verify]
async def verify(db: Pool | Connection | Transaction, statement: p.RawSQL, model: Model) -> None:
    """Check ``model`` against the result the server describes for
    ``statement``, without running it: every field's column present, and of
    a type the field's declared kind reads. A mismatch raises ``DecodeError``
    naming the field to change."""
    described = await p._describe(db, _statement(statement))
    declared = _model(model)
    columns: dict[str, tuple[str, p.Value | None]] = {}
    for name, type_name, kind in described:
        columns.setdefault(name, (type_name, kind))
    target = declared.table.name
    for field, column in declared.columns.items():
        name = declared._sql(field)
        if name not in columns:
            returned = ", ".join(name for name, _, _ in described)
            raise p.DecodeError(
                f"`{target}` reads column `{name}`, which the statement does not return; "
                f"it returns: {returned}"
            )
        type_name, kind = columns[name]
        if kind is None or not column._accepts(kind):
            raise p.DecodeError(
                f"`{target}` decodes column `{name}` as `{column._kind_name}`, which cannot "
                f"read PostgreSQL type `{type_name}`"
            )
