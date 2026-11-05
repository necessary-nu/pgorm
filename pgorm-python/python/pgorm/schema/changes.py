"""A disjoint model's kind changes as data: the steps a change runs, in
order, and the rows the program writes between two of them."""

from dataclasses import dataclass

from .._native import DDL, ConstructionError, Table


class _Part:
    """What a change and its parts share: each refuses truth testing, as the
    statements it holds do, so ``if change:`` fails where it is written."""

    __slots__ = ()

    def __bool__(self) -> bool:
        raise ConstructionError("a disjoint change cannot be tested as a Python boolean")


# [spec:pgorm:req:bindings.disjoint-evolve]
@dataclass(frozen=True, slots=True)
class DisjointGap(_Part):
    """What the program writes between two steps of a ``DisjointChange``: a
    row in ``table`` for every base row of the kind ``kind``, which the next
    step refuses the change without."""

    kind: str
    table: Table


# [spec:pgorm:req:bindings.disjoint-evolve]
@dataclass(frozen=True, slots=True)
class DisjointStep(_Part):
    """One step of a ``DisjointChange``: statements to run in order in one
    transaction, committed before the next step runs, and the rows the
    program writes once it commits, if any."""

    statements: tuple[DDL, ...]
    gap: DisjointGap | None


# [spec:pgorm:req:bindings.disjoint-evolve]
@dataclass(frozen=True, slots=True)
class DisjointChange(_Part):
    """A change to a disjoint model's kinds, as the steps that make it, in
    the order they run. A step that recreates the kind type changes what a
    statement prepared before it returns, so a program running the steps
    itself clears its pool's statement caches after each."""

    steps: tuple[DisjointStep, ...]
