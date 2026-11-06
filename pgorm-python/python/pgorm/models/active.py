"""A declared model's record writes and the hooks around them, the dialect of
Rust's ``ActiveModelTrait`` and ``ActiveModelBehavior``: ``Model.active(values)``
holds the fields a write sets, and its ``insert``, ``update`` and ``delete``
write one record by the model's own statement builders, each between the
model's ``before_*`` and ``after_*`` hooks."""

from __future__ import annotations

import inspect
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from types import MappingProxyType
from typing import TYPE_CHECKING, Any, TypedDict

from .. import _native as p

if TYPE_CHECKING:
    from .model import Model
    from .record import Database, ModelRecord

Values = Mapping[str, Any]


# [spec:pgorm:req:bindings.model-hooks]
class ModelHooks(TypedDict, total=False):
    """The hooks a model runs around its record writes, each optional and each
    a function or a coroutine function, as Rust's ``ActiveModelBehavior``
    methods are: ``before_save(values, insert)`` and ``before_delete(values)``
    give the values to write, or raise to refuse the write; ``after_save(record,
    insert)`` gives the record the write returns; ``after_delete(values)``
    runs once the row is gone."""

    before_save: Callable[[Values, bool], Values | Awaitable[Values]]
    after_save: Callable[[ModelRecord, bool], ModelRecord | Awaitable[ModelRecord]]
    before_delete: Callable[[Values], Values | Awaitable[Values]]
    after_delete: Callable[[Values], object]


HOOKS = ("before_save", "after_save", "before_delete", "after_delete")


def hooks_of(hooks: object) -> Mapping[str, Callable[..., Any]]:
    """``hooks`` as a model holds them, refused unless a mapping of known hooks
    to functions."""
    if hooks is None:
        return MappingProxyType({})
    if not isinstance(hooks, Mapping):
        raise TypeError("a model's hooks are a mapping of hook names to functions")
    for name, hook in hooks.items():
        if name not in HOOKS:
            raise TypeError(f"{name!r} is no model hook: a model takes {', '.join(HOOKS)}")
        if not callable(hook):
            raise TypeError(f"a model's {name} hook is a function")
    return MappingProxyType({name: hooks[name] for name in HOOKS if name in hooks})


async def _run(hook: Callable[..., Any], *arguments: Any) -> Any:
    answer = hook(*arguments)
    return await answer if inspect.isawaitable(answer) else answer


# [spec:pgorm:req:bindings.model-hooks]
@dataclass(frozen=True, eq=False, slots=True, init=False)
class ModelActive:
    """The fields one record write sets, as a Rust ``ActiveModel`` holds them:
    ``insert``, ``update`` and ``delete`` write the record and run the model's
    hooks around the write, as ``ActiveModelTrait`` does. A model's statement
    builders, ``insert``, ``update`` and ``delete`` on the model itself, run
    none, as Rust's ``Insert``, ``Update`` and ``Delete`` run none."""

    model: Model
    values: Values

    def __init__(self, model: Model, values: Values) -> None:
        if not isinstance(values, Mapping):
            raise TypeError("an active model's values are a mapping of fields")
        for field in values:
            model.col(field)
        object.__setattr__(self, "model", model)
        object.__setattr__(self, "values", MappingProxyType(dict(values)))

    async def _before(self, name: str, *arguments: Any) -> dict[str, Any]:
        hook = self.model._hooks.get(name)
        if hook is None:
            return dict(self.values)
        values = await _run(hook, dict(self.values), *arguments)
        if not isinstance(values, Mapping):
            raise TypeError(f"a model's {name} hook gives the values to write")
        return dict(values)

    async def _saved(self, record: ModelRecord, insert: bool) -> ModelRecord:
        from .record import ModelRecord

        hook = self.model._hooks.get("after_save")
        if hook is None:
            return record
        saved = await _run(hook, record, insert)
        if not isinstance(saved, ModelRecord):
            raise TypeError("a model's after_save hook gives the record the write returns")
        return saved

    def _key(self, values: Mapping[str, Any], write: str) -> dict[str, Any]:
        model = self.model
        if not model.primary_key:
            raise p.ConstructionError(f"{model.table.name} declares no primary key to {write} a record by")
        missing = [field for field in model.primary_key if field not in values]
        if missing:
            raise p.ConstructionError(f"a record {write} needs its key: {', '.join(missing)} is not set")
        return {field: values[field] for field in model.primary_key}

    async def insert(self, db: Database) -> ModelRecord:
        """``ActiveModelTrait::insert``: ``before_save(values, True)``, the
        row inserted and read back, then ``after_save(record, True)``."""
        values = await self._before("before_save", True)
        record = await self.model.insert(values).returning().one(db)
        return await self._saved(record, True)

    async def update(self, db: Database) -> ModelRecord:
        """``ActiveModelTrait::update``: ``before_save(values, False)``, the
        row the key names updated with the other fields set and read back,
        then ``after_save(record, False)``. A key no row has is a
        ``DecodeError``, as Rust's ``RecordNotUpdated`` is an error."""
        values = await self._before("before_save", False)
        key = self._key(values, "update")
        changes = {field: value for field, value in values.items() if field not in key}
        rows = await self.model.update(changes).where(self.model.key(key)).returning().all(db)
        if len(rows) != 1:
            raise p.DecodeError(f"no {self.model.table.name} row has the key {key!r} to update")
        return await self._saved(rows[0], False)

    async def delete(self, db: Database) -> int:
        """``ActiveModelTrait::delete``: ``before_delete(values)``, the row
        the key names deleted, then ``after_delete(values)``; the rows
        deleted."""
        values = await self._before("before_delete")
        key = self._key(values, "delete")
        deleted = await self.model.delete().where(self.model.key(key)).execute(db)
        hook = self.model._hooks.get("after_delete")
        if hook is not None:
            await _run(hook, MappingProxyType(values))
        return deleted

    def __bool__(self) -> bool:
        raise p.ConstructionError("active models cannot be tested as Python booleans")
