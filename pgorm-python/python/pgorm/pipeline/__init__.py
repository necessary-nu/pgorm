"""Compose pgorm's native PRQL-shaped pipeline stages without HTTP.

A plain operand is bound as a parameter by the stage that takes it; only
``literal(value)`` writes a value into the SQL. A ``*_with`` callback's Binder
mints placeholders to reuse within its stage, which cannot escape it.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, overload

from .. import _native
from .._native import (
    Pipeline as Pipeline,
    PipelineExpr as PipelineExpr,
    PipelineBinder as Binder,
    PipelineGrouped as Grouped,
    PipelineSource as Source,
    PipelineOver as Over,
    SourceSelection as SourceSelection,
    SelectedSources as SelectedSources,
    Join as Join,
    pipeline_literal as literal,
    pipeline_alias as alias,
    pipeline_case as case,
    pipeline_sources as sources,
    pipeline_source_tuples as source_tuples,
)
from ..models import Model
from ._typed import PipelineSelector as PipelineSelector, model_col

if TYPE_CHECKING:
    from .._entities import EntityColumn
    from .._pipeline_expr import Operand, Name


source = _native.pipeline_source
from_ = Pipeline


@overload
def col(table: Model, column: str) -> PipelineExpr: ...
@overload
def col(table: Name, column: Name | EntityColumn) -> PipelineExpr: ...


# [spec:pgorm:req:bindings.pipeline-models+1]
def col(table: Name | Model, column: Name | EntityColumn) -> PipelineExpr:
    """A column of the relation ``table`` names; of a model, its field
    ``column``, typed by the field's declaration."""
    if isinstance(table, Model):
        if not isinstance(column, str):
            raise TypeError("a model's column is named by its field")
        return model_col(table, column)
    return _native.pipeline_col(table, column)


# [spec:pgorm:req:python.pipeline+1]
def null() -> PipelineExpr:
    return literal(None)


def this(name: Name) -> PipelineExpr:
    return _native.pipeline_role("this", name)


def that(name: Name) -> PipelineExpr:
    return _native.pipeline_role("that", name)


over = Over


def sum(value: Operand) -> PipelineExpr:
    return _native.pipeline_function("sum", value)


def min(value: Operand) -> PipelineExpr:
    return _native.pipeline_function("min", value)


def max(value: Operand) -> PipelineExpr:
    return _native.pipeline_function("max", value)


def average(value: Operand) -> PipelineExpr:
    return _native.pipeline_function("average", value)


def stddev(value: Operand) -> PipelineExpr:
    return _native.pipeline_function("stddev", value)


def count(value: Operand) -> PipelineExpr:
    return _native.pipeline_function("count", value)


def count_distinct(value: Operand) -> PipelineExpr:
    return _native.pipeline_function("count_distinct", value)


def count_rows() -> PipelineExpr:
    return _native.pipeline_function("count_rows")


def row_number() -> PipelineExpr:
    return _native.pipeline_function("row_number")


def rank(value: Operand) -> PipelineExpr:
    return _native.pipeline_function("rank", value)


def rank_dense(value: Operand) -> PipelineExpr:
    return _native.pipeline_function("rank_dense", value)


def first(value: Operand) -> PipelineExpr:
    return _native.pipeline_function("first", value)


def last(value: Operand) -> PipelineExpr:
    return _native.pipeline_function("last", value)


def lag(offset: int, value: Operand) -> PipelineExpr:
    return _native.pipeline_function("lag", offset, value)


def lead(offset: int, value: Operand) -> PipelineExpr:
    return _native.pipeline_function("lead", offset, value)


__all__ = [
    "Pipeline",
    "PipelineExpr",
    "Binder",
    "Grouped",
    "Source",
    "Over",
    "Join",
    "SourceSelection",
    "SelectedSources",
    "PipelineSelector",
    "sources",
    "source_tuples",
    "source",
    "from_",
    "literal",
    "alias",
    "col",
    "case",
    "null",
    "this",
    "that",
    "over",
    "sum",
    "min",
    "max",
    "average",
    "stddev",
    "count",
    "count_distinct",
    "count_rows",
    "row_number",
    "rank",
    "rank_dense",
    "first",
    "last",
    "lag",
    "lead",
]
