"""Runtime column declarations and comparisons over native expressions."""

from __future__ import annotations

from dataclasses import KW_ONLY, FrozenInstanceError, dataclass, field
from collections.abc import Callable, Sequence
from typing import TYPE_CHECKING, Any, Literal, cast

from .. import _native as p
from . import casts

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

# The kinds a serial key is drawn from, as Rust's one-column integer key is.
SERIAL_KINDS = frozenset({"i16", "i32", "i64"})

if TYPE_CHECKING:
    from .._schema import ColumnType as DdlType


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


def _callable(value: object, option: str) -> None:
    if value is not None and not callable(value):
        raise TypeError(f"a column's {option} is a function")


def _expression(value: object, option: str) -> p.Expr | None:
    if value is not None and not isinstance(value, p.Expr):
        raise TypeError(f"a column's {option} is an expression")
    return value


# [spec:pgorm:req:bindings.models+6]
# [spec:pgorm:req:bindings.schema-from-model]
# [spec:pgorm:req:bindings.model-casts]
# [spec:pgorm:req:bindings.model-conversions]
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
    _: KW_ONLY
    column_type: DdlType | None = None
    default_value: Any = None
    default_expr: p.Expr | None = None
    generated_stored: p.Expr | None = None
    generated_virtual: p.Expr | None = None
    auto_increment: bool = False
    indexed: bool = False
    comment: str | None = None
    without_overlaps: bool = False
    select_as: DdlType | None = None
    save_as: DdlType | None = None
    select_with: Callable[[p.Expr], p.Expr] | None = None
    save_with: Callable[[p.Expr], p.Expr] | None = None
    save_array_with: Callable[[p.Expr], p.Expr] | None = None
    decode: Callable[[Any], Any] | None = None
    encode: Callable[[Any], Any] | None = None
    _null: p.Value = field(init=False, repr=False, compare=False)
    _default: p.Value | None = field(init=False, repr=False, compare=False)
    _casts: p._ModelColumnCasts | None = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        for option in (
            "nullable", "primary_key", "array", "default", "auto_increment", "indexed",
            "without_overlaps",
        ):
            _flag(getattr(self, option), option)
        if self.primary_key and self.nullable:
            raise p.ConstructionError("primary-key columns cannot be nullable")
        if self.generated is not None:
            if type(self.generated) is not str:
                raise TypeError('a column is generated "always" or "by_default"')
            if self.generated not in GENERATED:
                raise TypeError('a column is generated "always" or "by_default"')
        if self.comment is not None and type(self.comment) is not str:
            raise TypeError("a column's comment is a string")
        for option in ("select_with", "save_with", "save_array_with", "decode", "encode"):
            _callable(getattr(self, option), option)
        if self.select_as is not None and self.select_with is not None:
            raise p.ConstructionError("a column is read through select_as or select_with, not both")
        if self.save_as is not None and self.save_with is not None:
            raise p.ConstructionError("a column is written through save_as or save_with, not both")
        # [spec:pgorm:req:bindings.model-array-casts]
        if self.save_with is not None and self.save_array_with is None:
            raise p.ConstructionError(
                "a column written through save_with names save_array_with too: the function "
                "writing an array of its values, which = ANY and <> ALL compare it with"
            )
        if self.save_array_with is not None and self.save_with is None:
            raise p.ConstructionError(
                "save_array_with writes an array of the values save_with writes one of: "
                "name save_with beside it"
            )
        default_expr = _expression(self.default_expr, "default_expr")
        expressions = [
            _expression(self.generated_stored, "generated_stored"),
            _expression(self.generated_virtual, "generated_virtual"),
        ]
        computed = any(expression is not None for expression in expressions)
        if computed:
            if self.generated == "by_default":
                raise p.ConstructionError(
                    "a generated column is generated always: no write sets it"
                )
            object.__setattr__(self, "generated", "always")
        # PostgreSQL holds a column to one fill: a default, an identity, a
        # generation expression or the serial family (42601).
        fills = [
            self.default,
            self.default_value is not None,
            default_expr is not None,
            self.generated is not None and not computed,
            *(expression is not None for expression in expressions),
            self.auto_increment,
        ]
        if sum(fills) > 1:
            raise p.ConstructionError(
                "a column is filled one way: a default (default, default_value or "
                "default_expr), an identity (generated), a generation expression "
                "(generated_stored or generated_virtual) or the serial family "
                "(auto_increment)"
            )
        if self.name is not None:
            object.__setattr__(self, "name", p._identifier(self.name))
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
        null = p.Value.array(kind, None) if self.array else p.Value.null(kind)
        object.__setattr__(self, "_null", null)
        self._check_ddl()
        default = None if self.default_value is None else self._value(self.default_value)
        if default is not None and default.is_null:
            raise p.ConstructionError("a column's default_value is a value: NULL is every column's default")
        object.__setattr__(self, "_default", default)
        casted = (self.column_type, self.select_as, self.save_as) != (None, None, None)
        handle = p._model_casts(self._described("column")) if casted else None
        object.__setattr__(self, "_casts", handle)

    def _check_ddl(self) -> None:
        """Refuse what the column's DDL options cannot say of its kind."""
        created = isinstance(self.kind, (p.CreatedRange, p.CreatedMultirange))
        if self.column_type is not None and created:
            raise p.ConstructionError(
                "a created range column's type is its kind's own: it takes no column_type"
            )
        if self.auto_increment and (
            not self.primary_key or self.array or self.kind not in SERIAL_KINDS
        ):
            raise p.ConstructionError(
                "auto_increment draws a key column from the serial family: an i16, i32 or i64 "
                "primary key"
            )
        ranged = isinstance(self.kind, (p.CreatedRange, p.CreatedMultirange)) or (
            isinstance(self.kind, str) and self.kind.endswith("range")
        )
        if self.without_overlaps and (not self.primary_key or self.array or not ranged):
            raise p.ConstructionError(
                "without_overlaps makes a range or multirange key column the key's period"
            )

    @property
    def omissible(self) -> bool:
        """Whether an insert may leave the column out."""
        return (
            self.nullable
            or self.default
            or self.generated is not None
            or self._default is not None
            or self.default_expr is not None
            or self.auto_increment
        )

    def _described(self, name: str) -> dict[str, Any]:
        """The column as the native table shape is described from."""
        return {
            "name": name,
            "kind": self.kind,
            "array": self.array,
            "nullable": self.nullable,
            "default": self.default,
            "generated": self.generated,
            "values": None if self.values is None else list(self.values),
            "column_type": self.column_type,
            "default_value": self._default,
            "default_expr": self.default_expr,
            "generated_stored": self.generated_stored,
            "generated_virtual": self.generated_virtual,
            "indexed": self.indexed,
            "comment": self.comment,
            "select_as": self.select_as,
            "save_as": self.save_as,
        }

    def _accepts(self, value: p.Value) -> bool:
        """Whether a column whose values read as ``value``'s kind is one this
        column decodes: of the declared kind, or the text a column read as its
        text is projected as. The test a row's decode applies, so a statement
        described without running it is held to the same one
        (``bindings.verify``)."""
        if self._read_as_text and (value.kind, value.element_type) in (("text", None), ("array", "text")):
            return True
        return (value.kind, value.type_name, value.created_type, value.element_type) == (
            self._null.kind,
            self._null.type_name,
            self._null.created_type,
            self._null.element_type,
        )

    @property
    def _kind_name(self) -> str:
        """The declared kind as a verification failure names it."""
        kind = self.kind
        name = kind if isinstance(kind, str) else f"{kind.schema}.{kind.name}"
        return f"{name}[]" if self.array else name

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
        """``data`` converted to the declared kind, and validated: through the
        column's ``encode`` first, where it has one and ``data`` is no
        ``Value``."""
        kind = self.kind
        if self.encode is not None and data is not None and not isinstance(data, p.Value):
            data = self.encode(data)
        if isinstance(data, p.Value):
            result = data
        elif self.array:
            result = p.Value.array(kind, data)
        else:
            result = p.Value(data, kind)
        return self._validate(result)

    @property
    def _read_as_text(self) -> bool:
        """Whether the column is projected as its text: a created multirange
        is, and an array of one as ``text[]``, since the driver reports one
        with no subtype to read its binary form by, so it is read cast to
        text as pgorm's ``ColumnTrait::select_as`` reads an entity's; and so
        is an enum column typed as a domain over its enum, which pgorm reads
        as its text, unless the column reads itself another way."""
        if isinstance(self.kind, p.CreatedMultirange):
            return True
        declared = self.column_type is not None and isinstance(self.kind, p.TypeName)
        return declared and self.select_as is None and self.select_with is None

    def _decoded(self, value: p.Value) -> p.Value:
        """``value``, the column as its projection read it, as the declared
        kind and validated: a created multirange's text, or each of an
        array's, is read as the multirange of its declared subtype it spells."""
        if self._read_as_text and (value.kind, value.element_type) in (("text", None), ("array", "text")):
            try:
                if self.array:
                    value = p.Value.array(self.kind, value.value)
                elif value.is_null:
                    value = p.Value.null(self.kind)
                else:
                    value = p.Value(value.value, self.kind)
            except p.ConstructionError as error:
                raise p.DecodeError(str(error)) from error
        return self._validate(value, decoding=True)


# [spec:pgorm:req:bindings.model-columns+2]
# [spec:pgorm:req:bindings.column-operators+3]
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
        object.__setattr__(column, "field_name", p._identifier(field_name))
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

    # [spec:pgorm:req:bindings.model-columns+2]
    # [spec:pgorm:req:bindings.model-array-casts]
    def _quantified(self, quantified: p.Quantified) -> p.Quantified:
        """``any_(..)`` or ``all_(..)`` over a list of values, ranging over
        them each converted to the declared kind and bound as one array of it,
        an enum's cast to its array type, then written as the column writes
        the array its comparisons take, as pgorm's ``eq_any`` binds through
        the column's ``save_array_as``. One over a ``Select`` or an
        expression, and one beside an array column, whose elements are
        arrays, is the quantifier given."""
        listed = quantified._listed
        if listed is None or self.declaration.array:
            return quantified
        every, values = listed
        elements = []
        for value in values:
            if value is None:
                raise p.ConstructionError(
                    f"{self.field_name} compared with None is never true: test it with is_null()"
                )
            elements.append(self.declaration._value(value).value)
        array = casts.compared_array(self.declaration, p.Value.array(self.declaration.kind, elements))
        return p.all_(array) if every else p.any_(array)

    def _operand(self, value: Any) -> p.Expr | p.Value | p.Quantified:
        if isinstance(value, p.Quantified):
            return self._quantified(value)
        if isinstance(value, p.Expr):
            return value
        if value is None:
            raise p.ConstructionError(
                f"{self.field_name} compared with None is never true: test it with is_null()"
            )
        return casts.compared(self.declaration, self.declaration._value(value))

    def _apply(self, method: str, *operands: Any, **options: Any) -> p.Expr:
        """The operator ``method`` of the column as a comparison takes it, the
        enum a domain over one compares as, over ``operands``."""
        left = casts.compared_column(self.declaration, self._expression)
        receiver: Any = super() if left is None else left
        return cast(p.Expr, getattr(receiver, method)(*operands, **options))

    def eq(self, other: Any) -> p.Expr:
        return self._apply("eq", self._operand(other))

    def ne(self, other: Any) -> p.Expr:
        return self._apply("ne", self._operand(other))

    def gt(self, other: Any) -> p.Expr:
        return self._apply("gt", self._operand(other))

    def gte(self, other: Any) -> p.Expr:
        return self._apply("gte", self._operand(other))

    def lt(self, other: Any) -> p.Expr:
        return self._apply("lt", self._operand(other))

    def lte(self, other: Any) -> p.Expr:
        return self._apply("lte", self._operand(other))

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
        return self._apply("is_distinct_from", self._operand(other))

    def is_not_distinct_from(self, other: Any) -> p.Expr:
        return self._apply("is_not_distinct_from", self._operand(other))

    def between(self, lower: Any, upper: Any, *, symmetric: bool = False) -> p.Expr:
        return self._apply("between", 
            self._operand(lower), self._operand(upper), symmetric=symmetric
        )

    def not_between(
        self, lower: Any, upper: Any, *, symmetric: bool = False
    ) -> p.Expr:
        return self._apply("not_between", 
            self._operand(lower), self._operand(upper), symmetric=symmetric
        )

    def contains(self, other: Any) -> p.Expr:
        return self._apply("contains", self._operand(other))

    def contained_by(self, other: Any) -> p.Expr:
        return self._apply("contained_by", self._operand(other))

    def overlaps(self, other: Any) -> p.Expr:
        return self._apply("overlaps", self._operand(other))

    def strictly_left_of(self, other: Any) -> p.Expr:
        return self._apply("strictly_left_of", self._operand(other))

    def strictly_right_of(self, other: Any) -> p.Expr:
        return self._apply("strictly_right_of", self._operand(other))

    def not_extend_right_of(self, other: Any) -> p.Expr:
        return self._apply("not_extend_right_of", self._operand(other))

    def not_extend_left_of(self, other: Any) -> p.Expr:
        return self._apply("not_extend_left_of", self._operand(other))

    def adjacent_to(self, other: Any) -> p.Expr:
        return self._apply("adjacent_to", self._operand(other))

    def is_in(self, values: list[Any] | tuple[Any, ...] | p.Select) -> p.Expr:
        return self._apply("is_in", self._members(values))

    def is_not_in(self, values: list[Any] | tuple[Any, ...] | p.Select) -> p.Expr:
        return self._apply("is_not_in", self._members(values))

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
