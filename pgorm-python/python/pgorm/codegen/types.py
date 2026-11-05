"""Map reflected standard Rust field spellings to their native Python values."""

from dataclasses import dataclass
from collections.abc import Iterable
import re
from typing import Any

from .config import CodegenError
from .source import Item, Ternary, string


@dataclass(frozen=True)
class FieldType:
    annotation: str
    kind: str | None = None
    array: bool = False
    nullable: bool = False
    json: bool = False

    def binding(self, variable: str = "value") -> Item:
        """The expression binding ``variable`` under its field's tag, an
        explicit ``Value`` passed through as it is."""
        if self.kind is None:
            return variable
        if self.array:
            expression = f"p.Value.array({self.kind}, {variable})"
        elif self.json:
            expression = f"p.Value.json({variable})"
            if self.nullable:
                expression = (
                    f'(p.Value.null("json") if {variable} is None else {expression})'
                )
        else:
            expression = f"p.Value({variable}, {self.kind})"
        return Ternary(variable, f"isinstance({variable}, p.Value)", expression)


SCALARS = {
    "bool": ("bool", "bool"),
    "i8": ("int", "i8"),
    "i16": ("int", "i16"),
    "i32": ("int", "i32"),
    "i64": ("int", "i64"),
    "u32": ("int", "u32"),
    "u64": ("int", "u64"),
    "f32": ("float", "f32"),
    "f64": ("float", "f64"),
    "String": ("str", "text"),
    "std::string::String": ("str", "text"),
    "alloc::string::String": ("str", "text"),
    "char": ("str", "char"),
    "Decimal": ("Decimal", "decimal"),
    "rust_decimal::Decimal": ("Decimal", "decimal"),
    "Uuid": ("UUID", "uuid"),
    "uuid::Uuid": ("UUID", "uuid"),
    # Generated entities spell these through the pgorm prelude, so the bare
    # aliases and the jiff paths behind them both have to resolve.
    "Date": ("date", "date"),
    "civil::Date": ("date", "date"),
    "jiff::civil::Date": ("date", "date"),
    "Time": ("time", "time"),
    "civil::Time": ("time", "time"),
    "jiff::civil::Time": ("time", "time"),
    "DateTime": ("datetime", "datetime"),
    "civil::DateTime": ("datetime", "datetime"),
    "jiff::civil::DateTime": ("datetime", "datetime"),
    "DateTimeWithTimeZone": ("datetime", "datetime_utc"),
    "Timestamp": ("datetime", "datetime_utc"),
    "jiff::Timestamp": ("datetime", "datetime_utc"),
    "IpNetwork": ("str", "ipnetwork"),
    "ipnetwork::IpNetwork": ("str", "ipnetwork"),
    "MacAddress": ("bytes", "mac_address"),
    "mac_address::MacAddress": ("bytes", "mac_address"),
    "Vector": ("list[float]", "vector"),
    "pgvector::Vector": ("list[float]", "vector"),
    # The generated module imports pgorm as `p`, so the class is named through it.
    "Interval": ("p.Interval", "interval"),
    "pgorm_query::Interval": ("p.Interval", "interval"),
    "pgorm::pgorm_query::Interval": ("p.Interval", "interval"),
}


def wrapped(spelling: str, names: Iterable[str]) -> str | None:
    for name in names:
        prefix = name + "<"
        if spelling.startswith(prefix) and spelling.endswith(">"):
            return spelling[len(prefix) : -1]
    return None


# One double-quoted part of a type's name, its own quotes doubled.
QUOTED = re.compile(r'"((?:[^"]|"")*)"')


def type_name(spelling: str) -> tuple[str, str | None]:
    """A described kind's type, ``"schema"."name"`` or ``"name"``, as its
    name and schema."""
    parts: list[str] = []
    position = 0
    while True:
        match = QUOTED.match(spelling, position)
        if match is None:
            raise CodegenError(f"a described kind names its type in quotes: {spelling!r}")
        parts.append(match[1].replace('""', '"'))
        position = match.end()
        if position == len(spelling):
            break
        if spelling[position] != "." or len(parts) == 2:
            raise CodegenError(f"a described kind names its type in quotes: {spelling!r}")
        position += 1
    return parts[-1], (parts[0] if len(parts) == 2 else None)


def field_type(column: dict[str, Any]) -> FieldType:
    spelling = column.get("rust_type")
    if not isinstance(spelling, str):
        return FieldType("Any")
    return resolve(spelling.replace(" ", ""), column.get("kind"))


def resolve(spelling: str, kind: str | None) -> FieldType:
    inner = wrapped(spelling, ("Option", "std::option::Option", "core::option::Option"))
    if inner is not None:
        resolved = resolve(inner, kind)
        return FieldType(
            resolved.annotation + " | None",
            resolved.kind,
            resolved.array,
            True,
            resolved.json,
        )
    if spelling in ("Vec<u8>", "std::vec::Vec<u8>", "alloc::vec::Vec<u8>"):
        return FieldType("bytes", string("bytes"))
    inner = wrapped(spelling, ("Vec", "std::vec::Vec", "alloc::vec::Vec"))
    if inner is not None:
        if kind is None or not kind.endswith("[]"):
            return FieldType("Any")
        element = resolve(inner, kind[:-2])
        if element.kind is None or element.array:
            return FieldType("Any")
        return FieldType(f"list[{element.annotation}]", element.kind, array=True)
    if kind is not None and kind.startswith("enum "):
        name, schema = type_name(kind[len("enum ") :])
        schema_text = "None" if schema is None else string(schema)
        return FieldType("str", f"p.TypeName({string(name)}, schema={schema_text})")
    if spelling in ("Json", "JsonValue", "serde_json::Value"):
        return FieldType("Any", string("json"), json=True)
    if spelling in SCALARS:
        annotation, kind = SCALARS[spelling]
        return FieldType(annotation, string(kind))
    return FieldType("Any")
