"""How a model's column is read, written and compared: through its own
``select_with`` and ``save_with``, the casts pgorm's ``TableColumn`` gives
its type or its ``select_as`` and ``save_as``, and its ``decode``."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

from .. import _native as p

if TYPE_CHECKING:
    from .columns import Column


# [spec:pgorm:req:bindings.model-casts]
def selected(column: Column, expr: p.Expr) -> p.Expr:
    """``expr``, a read of ``column``, as a projection reads it: through
    ``select_with``, cast as ``select_as`` or the column type's own read says
    (pgorm's ``TableColumn::select``), as its text where it is read from its
    text, and as it is otherwise."""
    if column.select_with is not None:
        read = column.select_with(expr)
        if not isinstance(read, p.Expr):
            raise TypeError("a column's select_with returns an expression")
        return read
    if column._casts is not None and (column.select_as, column.column_type) != (None, None):
        cast = column._casts.select(expr)
        return expr if cast is None else cast
    if column._read_as_text:
        return expr.cast(p.TypeName("text"), array=column.array)
    return expr


def projected(column: Column, expr: p.Expr, name: str, own: str) -> p.Expr | p.Aliased:
    """``expr`` read as ``selected`` reads it, under ``name``: aliased unless
    it is the column as it stands under its own name ``own``."""
    read = selected(column, expr)
    return expr if read is expr and name == own else read.as_(name)


def saved(column: Column, value: p.Value) -> p.Value | p.Expr:
    """``value`` as it is written to ``column``: through ``save_with``, cast
    as ``save_as`` or the column type's own write says (pgorm's
    ``TableColumn::save``), and as it is otherwise."""
    if column.save_with is not None:
        written = column.save_with(p.bind(value))
        if not isinstance(written, p.Expr):
            raise TypeError("a column's save_with returns an expression")
        return written
    if column._casts is not None and (column.save_as, column.column_type) != (None, None):
        return column._casts.save(value)
    return value


# [spec:pgorm:req:bindings.model-array-casts]
def compared_array(column: Column, array: p.Value) -> p.Value | p.Expr:
    """``array``, a list's values bound as one array of the column's kind, as
    ``= ANY`` and ``<> ALL`` compare ``column`` with it: through
    ``save_array_with``, cast as the column's type or ``save_as`` says
    (pgorm's ``TableColumn::compared_array``), and as it is otherwise."""
    if column.save_array_with is not None:
        written = column.save_array_with(p.bind(array))
        if not isinstance(written, p.Expr):
            raise TypeError("a column's save_array_with returns an expression")
        return written
    if column._casts is not None and (column.save_as, column.column_type) != (None, None):
        return column._casts.compared_array(array)
    return array


# [spec:pgorm:req:bindings.pipeline-models+1]
def read_as(column: Column) -> tuple[p.TypeName | p._ModelColumnCasts | None, bool]:
    """What a pipeline reads ``column`` as, and whether that is an array of
    it: the casts pgorm's ``TableColumn`` reads its type or ``select_as``
    through, its text where it is read from its text, and nothing — the
    column as it stands — otherwise, ``select_with`` among them, a function
    no pipeline lowers."""
    if column.select_with is not None:
        return None, False
    if column._casts is not None and (column.select_as, column.column_type) != (None, None):
        return column._casts, False
    if column._read_as_text:
        return p.TypeName("text"), column.array
    return None, False


def compared_column(column: Column, expr: p.Expr) -> p.Expr | None:
    """``expr``, a read of ``column``, as a comparison takes it where its type
    says otherwise: cast to the enum a domain over one compares as; ``None``
    where the column is compared as it stands."""
    if column._casts is None or not column._casts.compares_as_enum:
        return None
    return column._casts.compared_column(expr)


def compared(column: Column, value: p.Value) -> p.Value | p.Expr:
    """``value`` as a comparison with ``column`` takes it: cast to the enum a
    domain over one compares as, and otherwise as ``saved`` writes it
    (pgorm's ``TableColumn::compared_value``)."""
    if column.save_with is None and column._casts is not None and (
        column.save_as, column.column_type
    ) != (None, None):
        return column._casts.compared_value(value)
    return saved(column, value)


# [spec:pgorm:req:bindings.model-conversions]
def read(column: Column, value: p.Value) -> Any:
    """The Python value of ``value``, a decoded value of ``column``: through
    ``decode`` where the column has one, a failure of which is a
    ``DecodeError``."""
    if column.decode is None or value.is_null:
        return value.value
    try:
        return column.decode(value.value)
    except p.DecodeError:
        raise
    except Exception as error:
        raise p.DecodeError(f"the column's decode refused {value.value!r}: {error}") from error
