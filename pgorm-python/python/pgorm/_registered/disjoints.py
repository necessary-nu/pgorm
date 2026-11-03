"""Generated views of a registered disjoint enum: its rows read as the
generated models of its entities, and its writes, for a kinded enum alone,
taking their generated ActiveModels."""

from typing import Any, Callable, Generic, TypeVar

from .. import _native as native
from ..models.disjoint import DisjointEntity
from .entities import ModelView

R = TypeVar("R")


# [spec:pgorm:req:bindings.codegen-disjoint]
def wrap_disjoint_row(
    row: Any, base: type[ModelView[Any]], kinds: dict[str, type[ModelView[Any]] | None]
) -> Any:
    """``(base model, kind, kind model or None)`` of a native row, each model
    the generated view of its entity: a kind with no generated model holds no
    record."""
    record, kind, attached = row
    if kind not in kinds:
        raise native.InternalError(f"native disjoint row of a kind {kind!r} the module does not type")
    model = kinds[kind]
    if (model is None) != (attached is None):
        raise native.InternalError("native disjoint row differs from its kind's generated shape")
    return (base(record), kind, None if model is None else model(attached))


class _Decoded(DisjointEntity[Any, R]):
    """The registration, its rows read through ``decode``."""

    __slots__ = ("_decode",)

    def __init__(self, entity: DisjointEntity[Any, Any], decode: Callable[[Any], R]) -> None:
        super().__init__(entity._native)
        object.__setattr__(self, "_decode", decode)

    def _row(self, item: Any) -> R:
        return self._decode(item)


# [spec:pgorm:req:bindings.codegen-disjoint]
class DisjointView(Generic[R]):
    """A registered disjoint enum's reads, each row as ``decode`` types it:
    an unkinded enum's whole view, which has no write and no schema."""

    __slots__ = ("_entity",)

    def __init__(self, entity: DisjointEntity[Any, Any], decode: Callable[[Any], R]) -> None:
        self._entity = _Decoded(entity, decode)

    @property
    def name(self) -> str:
        return self._entity.name

    def describe(self):
        return self._entity.describe()

    def find(self):
        return self._entity.find()

    def find_by_key(self, key):
        return self._entity.find_by_key(key)


class WritableDisjointView(DisjointView[R]):
    """A kinded enum's view: its reads, and its writes, taking the generated
    ActiveModels of its base and kinds; ``create_disjoint`` takes it for its
    schema."""

    __slots__ = ()

    def insert(self, base, kind, active=None):
        return self._entity.insert(base, kind, active)

    def update(self, base, kind, active=None):
        return self._entity.update(base, kind, active)

    def change_kind(self, key, kind, active=None):
        return self._entity.change_kind(key, kind, active)
