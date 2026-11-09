"""A pipeline's rows read as declared models' records, as Rust's
``select_sources``, ``into_model`` and ``into_tuple`` read them as entities'
models and a program's types: the staged reads ``Pipeline.select_sources``,
``Pipeline.into_model`` and ``Pipeline.into_tuple`` return, and the model
columns ``col(model, field)`` names."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from typing import TYPE_CHECKING, Any, Generic, Literal, TypeVar

from .. import _native as p
from ..models import Model, ModelRecord, casts
from ..models.record import Source, decoder

if TYPE_CHECKING:
    from ..models.record import Database

T = TypeVar("T")

Terminal = Literal["all", "one", "optional"]
TERMINALS = ("all", "one", "optional")


# [spec:pgorm:req:bindings.pipeline-models+1]
def model_col(model: Model, field: str) -> p.PipelineExpr:
    """``field`` of ``model``, qualified as the model is read, written under
    its column's name, and compared as its declared kind: a value compared
    with it converts through the declaration, as the model column's own
    comparisons convert one, and is cast as pgorm's ``col_typed`` casts a
    value compared with a column of that kind."""
    column = model.col(field)
    declaration = column.declaration

    def convert(value: object) -> p.Value:
        if value is None:
            raise p.ConstructionError(
                f"{column.field_name} compared with None is never true: test it with is_null()"
            )
        return declaration._value(value)

    # A value a save_with function writes is no cast a pipeline can write.
    shaped = declaration._casts if declaration.save_with is None else None
    return p.pipeline_model_col(
        model.qualifier,
        model._sql(column.field_name),
        declaration.kind,
        declaration.array,
        None if declaration.values is None else list(declaration.values),
        convert,
        shaped,
    )


# [spec:pgorm:req:bindings.pipeline-models+1]
class PipelineSelector(Generic[T]):
    """A pipeline's rows staged for one shape of read. ``all``, ``one`` and
    ``optional`` run it as the pipeline's own terminals do, ``one`` and
    ``optional`` taking one row before the read's projection, and
    ``inspect(terminal=)`` shows the SQL and values each sends."""

    __slots__ = ("_pipeline", "_finish", "_read")

    def __init__(
        self,
        pipeline: p.Pipeline,
        finish: Callable[[p.Pipeline], p.Pipeline],
        read: Callable[[Database, p.Pipeline, Terminal], Awaitable[Any]],
    ) -> None:
        self._pipeline = pipeline
        self._finish = finish
        self._read = read

    def _statement(self, terminal: str) -> p.Pipeline:
        if terminal not in TERMINALS:
            raise TypeError('a pipeline read inspects its terminal "all", "one" or "optional"')
        pipeline = self._pipeline if terminal == "all" else self._pipeline.take(1)
        return self._finish(pipeline)

    def inspect(self, *, terminal: Terminal = "all") -> p.Compiled:
        return self._statement(terminal).inspect()

    async def all(self, db: Database) -> list[T]:
        rows: list[T] = await self._read(db, self._statement("all"), "all")
        return rows

    async def one(self, db: Database) -> T:
        row: T = await self._read(db, self._statement("one"), "one")
        return row

    async def optional(self, db: Database) -> T | None:
        row: T | None = await self._read(db, self._statement("optional"), "optional")
        return row

    def __bool__(self) -> bool:
        raise p.ConstructionError("pipeline reads cannot be tested as Python booleans")


def _rows(
    decode: Callable[[p.Record], Any],
) -> Callable[[Database, p.Pipeline, Terminal], Awaitable[Any]]:
    """A read decoding each record ``db`` returns through ``decode``."""

    async def read(db: Database, statement: p.Pipeline, terminal: Terminal) -> Any:
        if terminal == "all":
            return [decode(row) for row in await db.query(statement)]
        if terminal == "one":
            return decode(await db.one(statement))
        row = await db.optional(statement)
        return None if row is None else decode(row)

    return read


def _model_record(model: Model) -> Callable[[p.Record], ModelRecord]:
    """A row read as ``model``'s record, each field from the result column of
    its SQL name, the row's columns exactly the fields'."""
    fields = tuple(model.columns)
    names = tuple(model._sql(field) for field in fields)

    def decode(row: p.Record) -> ModelRecord:
        if len(row) != len(names) or set(row.keys()) != set(names):
            raise p.DecodeError(
                f"the result's columns {list(row.keys())} are not {model.table.name}'s "
                f"{list(names)}"
            )
        return ModelRecord(model, row, fields, names)

    return decode


def _read(model: Model) -> list[tuple[str, p.TypeName | p._ModelColumnCasts | None, bool]]:
    """Each of ``model``'s columns by its SQL name, with what its record reads
    it as."""
    return [(model._sql(field), *casts.read_as(column)) for field, column in model.columns.items()]


def into_model(pipeline: p.Pipeline, target: object) -> PipelineSelector[Any]:
    """Rust's ``into_model::<M>``: each row a declared model's record, the
    pipeline ending in a projection of its columns where one is read through
    a cast, as Rust's projects an entity model's, or a function called with
    the row's columns as keyword arguments."""
    if isinstance(target, Model):
        read = _read(target)
        return PipelineSelector(
            pipeline, lambda staged: staged._read_columns(read), _rows(_model_record(target))
        )
    if not callable(target):
        raise TypeError("into_model takes a Model or a function of a row's columns")
    function = target
    return PipelineSelector(
        pipeline, _same, _rows(lambda row: function(**{name: row[name] for name in row.keys()}))
    )


def into_tuple(pipeline: p.Pipeline) -> PipelineSelector[tuple[Any, ...]]:
    """Rust's ``into_tuple``: each row a tuple of its values in column order,
    read by position."""

    async def read(db: Database, statement: p.Pipeline, terminal: Terminal) -> Any:
        return await p.pipeline_tuples(db, statement, terminal)

    return PipelineSelector(pipeline, _same, read)


def select_models(
    pipeline: p.Pipeline, models: Sequence[object]
) -> PipelineSelector[tuple[ModelRecord | None, ...]]:
    """Rust's ``select_sources`` over declared models: each row a tuple of one
    record per model, None where a join left that model's side absent."""
    if not all(isinstance(model, Model) for model in models):
        raise TypeError(
            "select_sources takes a registered SourceSelection, or one or more Models"
        )
    listed: tuple[Model, ...] = tuple(model for model in models if isinstance(model, Model))
    sources = tuple(
        Source(
            model,
            tuple(model.columns),
            tuple(p._model_result_names(f"s{index}_", [model._sql(field) for field in model.columns])),
            True,
        )
        for index, model in enumerate(listed)
    )
    described = [(model.qualifier, _read(model)) for model in listed]
    decode = decoder(sources)

    def finish(selected: p.Pipeline) -> p.Pipeline:
        return selected._select_described(described)

    def row(record: p.Record) -> tuple[ModelRecord | None, ...]:
        decoded = decode(record)
        return decoded if isinstance(decoded, tuple) else (decoded,)

    return PipelineSelector(pipeline, finish, _rows(row))


def _same(pipeline: p.Pipeline) -> p.Pipeline:
    return pipeline
