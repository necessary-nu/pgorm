"""Runtime column declarations and comparisons over native expressions."""

from __future__ import annotations

from dataclasses import FrozenInstanceError, dataclass, field
from collections.abc import Sequence
from typing import Any, Literal

from .. import _native as p

# These tags are produced by the native Record decoder without reinterpretation.
MODEL_KINDS = frozenset(
    {
        "bool",
        "i8",
        "i16",
        "i32",
        "i64",
        "u32",
        "f32",
        "f64",
        "text",
        "bytes",
        "decimal",
        "uuid",
        "json",
        "date",
        "time",
        "datetime",
        "datetime_utc",
        "ipnetwork",
        "mac_address",
        "vector",
        "interval",
        "int4range",
        "int8range",
        "numrange",
        "daterange",
        "tsrange",
        "tstzrange",
        "int4multirange",
        "int8multirange",
        "nummultirange",
        "datemultirange",
        "tsmultirange",
        "tstzmultirange",
    }
)


GENERATED = ("always", "by_default")


def _flag(value: object, option: str) -> bool:
    if type(value) is not bool:
        raise TypeError(f"a column's {option} is a bool")
    return value


def _labels(values: object) -> tuple[str, ...]:
    if not isinstance(values, (list, tuple)) or not all(
        type(label) is str for label in values
    ):
        raise TypeError("an enum column's values are a list or tuple of strings")
    labels = tuple(values)
    if not labels:
        raise p.ConstructionError("an enum column lists at least one value")
    if len(set(labels)) != len(labels):
        raise p.ConstructionError("an enum column lists each value once")
    return labels


# [spec:pgorm:req:bindings.models+5]
@dataclass(frozen=True, slots=True)
class Column:
    kind: str | p.TypeName | p.CreatedRange | p.CreatedMultirange
    name: str | None = None
    nullable: bool = False
    primary_key: bool = False
    array: bool = False
    default: bool = False
    generated: Literal["always", "by_default"] | None = None
    values: Sequence[str] | None = None
    _null: p.Value = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        for option in ("nullable", "primary_key", "array", "default"):
            _flag(getattr(self, option), option)
        if self.primary_key and self.nullable:
            raise p.ConstructionError("primary-key columns cannot be nullable")
        if self.generated is not None:
            if type(self.generated) is not str:
                raise TypeError('a column is generated "always" or "by_default"')
            if self.generated not in GENERATED:
                raise TypeError('a column is generated "always" or "by_default"')
            if self.default:
                raise p.ConstructionError(
                    "a generated column has no default of its own: declare one or the other"
                )
        if self.name is not None:
            object.__setattr__(self, "name", p.Identifier(self.name).name)
        if isinstance(self.kind, (p.TypeName, p.CreatedRange, p.CreatedMultirange)):
            # A decoded row names an enum or a created range with its schema.
            if self.kind.schema is None:
                raise p.ConstructionError(
                    "an enum or created range column names its type's schema"
                )
        elif type(self.kind) is not str:
            raise TypeError(
                "a column's kind is a kind name, a TypeName, a CreatedRange or a CreatedMultirange"
            )
        elif self.kind not in MODEL_KINDS:
            raise p.UnsupportedCapabilityError(
                "column kind is not supported by the native Record decoder"
            )
        if self.values is not None:
            if not isinstance(self.kind, p.TypeName):
                raise p.ConstructionError("only an enum column lists its values")
            object.__setattr__(self, "values", _labels(self.values))
        kind = self.kind
        if not self.array:
            null = p.Value.null(kind)
        elif isinstance(kind, (p.CreatedRange, p.CreatedMultirange)):
            raise p.ConstructionError(
                "arrays of a created range or multirange type are not supported"
            )
        else:
            null = p.Value.array(kind, None)
        object.__setattr__(self, "_null", null)

    @property
    def omissible(self) -> bool:
        """Whether an insert may leave the column out."""
        return self.nullable or self.default or self.generated is not None

    def _validate(self, value: p.Value, *, decoding: bool = False) -> p.Value:
        """``value``, refused unless it is of the declared kind, null only
        where the column is nullable and an enum's label one it lists."""
        error = p.DecodeError if decoding else p.ConstructionError
        if (value.kind, value.type_name, value.created_type, value.element_type) != (
            self._null.kind,
            self._null.type_name,
            self._null.created_type,
            self._null.element_type,
        ):
            raise error("value type differs from the declared model column")
        if value.is_null and not self.nullable:
            raise error("non-nullable model column received SQL NULL")
        if self.values is not None and not value.is_null:
            labels = value.value if self.array else [value.value]
            for label in labels:
                if label is not None and label not in self.values:
                    raise error(f"{label!r} is not one of the column's values")
        return value

    def _value(self, data: Any) -> p.Value:
        """``data`` converted to the declared kind, and validated."""
        kind = self.kind
        if isinstance(data, p.Value):
            result = data
        elif self.array and not isinstance(kind, (p.CreatedRange, p.CreatedMultirange)):
            result = p.Value.array(kind, data)
        else:
            result = p.Value(data, kind)
        return self._validate(result)

    @property
    def _read_as_text(self) -> bool:
        """Whether the column is projected as its text: a created multirange
        is, since the driver reports one with no subtype to read its binary
        form by, so it is read cast to ``text`` as pgorm's
        ``ColumnTrait::select_as`` reads an entity's."""
        return isinstance(self.kind, p.CreatedMultirange)

    def _decoded(self, value: p.Value) -> p.Value:
        """``value``, the column as its projection read it, as the declared
        kind and validated: a created multirange's text is read as the
        multirange of its declared subtype it spells."""
        if self._read_as_text and value.kind == "text":
            try:
                value = (
                    p.Value.null(self.kind)
                    if value.is_null
                    else p.Value(value.value, self.kind)
                )
            except p.ConstructionError as error:
                raise p.DecodeError(str(error)) from error
        return self._validate(value, decoding=True)


# [spec:pgorm:req:bindings.model-columns+1]
# [spec:pgorm:req:bindings.column-operators+1]
class ModelColumn(p._DeclaredColumn):
    """A model field's column, itself the column's expression.

    Every operator that takes a value converts it through the field's
    declared kind; every other ``Expr`` method reads the column as written.
    """

    __slots__ = ("field_name", "declaration", "_expression")
    field_name: str
    declaration: Column
    _expression: p.Expr

    def __new__(
        cls, field_name: str, declaration: Column, expression: p.Expr
    ) -> ModelColumn:
        if not isinstance(declaration, Column):
            raise TypeError("a model column's declaration is a Column")
        # The native base takes its expression in __new__; its stub spells the
        # runtime signature as __init__, which mypy does not read here.
        column = super().__new__(cls, expression)  # type: ignore[call-arg]
        object.__setattr__(column, "field_name", p.Identifier(field_name).name)
        object.__setattr__(column, "declaration", declaration)
        object.__setattr__(column, "_expression", expression)
        return column

    def __setattr__(self, name: str, value: object) -> None:
        raise FrozenInstanceError(f"cannot assign to field {name!r}")

    def __delattr__(self, name: str) -> None:
        raise FrozenInstanceError(f"cannot delete field {name!r}")

    def expr(self) -> p.Expr:
        """The plain expression, whose operators bind as an ``Expr``'s do."""
        return self._expression

    def _operand(self, value: Any) -> p.Expr | p.Value | p.Quantified:
        if isinstance(value, p.Quantified):
            return value
        if isinstance(value, ModelColumn):
            if (
                self.declaration._null.snapshot()["type"]
                != value.declaration._null.snapshot()["type"]
            ):
                raise p.ConstructionError(
                    "compared model columns have different declared types"
                )
            return value
        if isinstance(value, p.Expr):
            return value
        if value is None:
            raise p.ConstructionError(
                f"{self.field_name} compared with None is never true: test it with is_null()"
            )
        return self.declaration._value(value)

    def eq(self, other: Any) -> p.Expr:
        return super().eq(self._operand(other))

    def ne(self, other: Any) -> p.Expr:
        return super().ne(self._operand(other))

    def gt(self, other: Any) -> p.Expr:
        return super().gt(self._operand(other))

    def gte(self, other: Any) -> p.Expr:
        return super().gte(self._operand(other))

    def lt(self, other: Any) -> p.Expr:
        return super().lt(self._operand(other))

    def lte(self, other: Any) -> p.Expr:
        return super().lte(self._operand(other))

    def __eq__(self, other: object) -> p.Expr:  # type: ignore[override]
        return self.eq(other)

    def __ne__(self, other: object) -> p.Expr:  # type: ignore[override]
        return self.ne(other)

    def __gt__(self, other: Any) -> p.Expr:
        return self.gt(other)

    def __ge__(self, other: Any) -> p.Expr:
        return self.gte(other)

    def __lt__(self, other: Any) -> p.Expr:
        return self.lt(other)

    def __le__(self, other: Any) -> p.Expr:
        return self.lte(other)

    def is_distinct_from(self, other: Any) -> p.Expr:
        return super().is_distinct_from(self._operand(other))

    def is_not_distinct_from(self, other: Any) -> p.Expr:
        return super().is_not_distinct_from(self._operand(other))

    def between(self, lower: Any, upper: Any, *, symmetric: bool = False) -> p.Expr:
        return super().between(
            self._operand(lower), self._operand(upper), symmetric=symmetric
        )

    def not_between(
        self, lower: Any, upper: Any, *, symmetric: bool = False
    ) -> p.Expr:
        return super().not_between(
            self._operand(lower), self._operand(upper), symmetric=symmetric
        )

    def contains(self, other: Any) -> p.Expr:
        return super().contains(self._operand(other))

    def contained_by(self, other: Any) -> p.Expr:
        return super().contained_by(self._operand(other))

    def overlaps(self, other: Any) -> p.Expr:
        return super().overlaps(self._operand(other))

    def strictly_left_of(self, other: Any) -> p.Expr:
        return super().strictly_left_of(self._operand(other))

    def strictly_right_of(self, other: Any) -> p.Expr:
        return super().strictly_right_of(self._operand(other))

    def not_extend_right_of(self, other: Any) -> p.Expr:
        return super().not_extend_right_of(self._operand(other))

    def not_extend_left_of(self, other: Any) -> p.Expr:
        return super().not_extend_left_of(self._operand(other))

    def adjacent_to(self, other: Any) -> p.Expr:
        return super().adjacent_to(self._operand(other))

    def is_in(self, values: list[Any] | tuple[Any, ...] | p.Select) -> p.Expr:
        return super().is_in(self._members(values))

    def is_not_in(self, values: list[Any] | tuple[Any, ...] | p.Select) -> p.Expr:
        return super().is_not_in(self._members(values))

    def _members(
        self, values: Any
    ) -> list[p.Expr | p.Value | p.Quantified] | p.Select:
        if isinstance(values, p.Select):
            return values
        if type(values) not in (list, tuple):
            raise TypeError("model IN takes a list or tuple, or a Select")
        return [self._operand(value) for value in values]

    def __repr__(self) -> str:
        return f"ModelColumn({self.field_name!r})"
