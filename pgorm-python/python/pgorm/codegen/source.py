"""Python source laid out as a person writes it, in black's layout: strings
double-quoted, a bracket's contents on its line where they fit in 88 columns,
on a line of their own where those fit, and otherwise one to a line with a
comma after each, and a conditional expression too long for its line split
before its ``if`` and its ``else``."""

# [spec:pgorm:req:bindings.codegen-layout]

from collections.abc import Iterable, Sequence
from typing import Protocol

WIDTH = 88


def string(value: str) -> str:
    """``value`` as a string literal, double-quoted unless it holds a double
    quote and no single one, as black writes it."""
    quoted = repr(value)
    if quoted.startswith("'") and '"' not in value:
        return '"' + quoted[1:-1] + '"'
    return quoted


class Code(str):
    """Text ``literal`` writes as it is, a name or an expression, where it
    writes any other string as a string literal."""


class Layout(Protocol):
    """An expression whose layout depends on the room it has."""

    def flat(self) -> str: ...

    def render(self, indent: int, before: int, after: int) -> str:
        """The expression starting ``before`` columns into a line indented
        ``indent``, followed by ``after`` columns on its last line; its first
        line carries no indentation of its own."""
        ...


Item = str | Layout


def _flat_item(item: Item) -> str:
    return item if isinstance(item, str) else item.flat()


def _render(item: Item, indent: int, before: int, after: int) -> str:
    if isinstance(item, str):
        return item
    return item.render(indent, before, after)


class Call:
    """``head``, which opens a bracket, its ``items``, then ``close`` and
    ``trailer``: on one line where it fits; else the items on a line of their
    own between the brackets, unless ``explode`` asks for one item a line
    outright, as an import's names take it; else one item to a line, each with
    a comma after it."""

    def __init__(
        self,
        head: str,
        items: Sequence[Item],
        close: str,
        trailer: str = "",
        *,
        explode: bool = False,
    ) -> None:
        self.head, self.items, self.close = head, list(items), close
        self.trailer, self.explode = trailer, explode

    def flat(self) -> str:
        inner = ", ".join(_flat_item(item) for item in self.items)
        return f"{self.head}{inner}{self.close}{self.trailer}"

    def render(self, indent: int, before: int, after: int) -> str:
        line = self.flat()
        if indent + before + len(line) + after <= WIDTH:
            return line
        inner = indent + 4
        pad = " " * inner
        joined = ", ".join(_flat_item(item) for item in self.items)
        end = f"{' ' * indent}{self.close}{self.trailer}"
        if not self.explode and inner + len(joined) <= WIDTH:
            return f"{self.head}\n{pad}{joined}\n{end}"
        body = "\n".join(f"{pad}{_render(item, inner, 0, 1)}," for item in self.items)
        return f"{self.head}\n{body}\n{end}"


class Ternary:
    """``body if test else orelse``, split before its ``if`` and its
    ``else`` where it does not fit."""

    def __init__(self, body: Item, test: str, orelse: Item) -> None:
        self.body, self.test, self.orelse = body, test, orelse

    def flat(self) -> str:
        return f"{_flat_item(self.body)} if {self.test} else {_flat_item(self.orelse)}"

    def render(self, indent: int, before: int, after: int) -> str:
        line = self.flat()
        if indent + before + len(line) + after <= WIDTH:
            return line
        pad = " " * indent
        body = _render(self.body, indent, before, 0)
        orelse = _render(self.orelse, indent, 5, after)
        return f"{body}\n{pad}if {self.test}\n{pad}else {orelse}"


def statement(item: Item, indent: int = 0) -> str:
    """``item`` as a statement at ``indent``."""
    return " " * indent + _render(item, indent, 0, 0)


def bracket(
    head: str,
    items: Sequence[Item],
    close: str,
    indent: int = 0,
    trailer: str = "",
    *,
    explode: bool = False,
) -> str:
    """A ``Call`` as a statement at ``indent``."""
    return statement(Call(head, items, close, trailer, explode=explode), indent)


def flat(value: object) -> str:
    """``literal``'s layout of ``value`` on one line, however long."""
    if isinstance(value, Code):
        return str(value)
    if isinstance(value, str):
        return string(value)
    if isinstance(value, dict):
        return "{" + ", ".join(f"{flat(k)}: {flat(v)}" for k, v in value.items()) + "}"
    if isinstance(value, list):
        return "[" + ", ".join(flat(item) for item in value) + "]"
    if isinstance(value, tuple):
        if len(value) == 1:
            return f"({flat(value[0])},)"
        return "(" + ", ".join(flat(item) for item in value) + ")"
    return repr(value)


def literal(value: object, indent: int = 0, before: int = 0, after: int = 0) -> str:
    """A dict, list, tuple, string, number, boolean or ``None`` as Python
    source, ``before`` and ``after`` columns taken on its first and last line
    by what surrounds it: on one line where that fits, else one item to a
    line, each laid out the same way in turn."""
    line = flat(value)
    if not isinstance(value, (dict, list, tuple)) or not value:
        return line
    if indent + before + len(line) + after <= WIDTH:
        return line
    inner = indent + 4
    lines: list[str] = []
    if isinstance(value, dict):
        opening, closing = "{", "}"
        for key, item in value.items():
            key_text = f"{flat(key)}: "
            lines.append(key_text + literal(item, inner, len(key_text), 1))
    else:
        opening, closing = ("[", "]") if isinstance(value, list) else ("(", ")")
        lines.extend(literal(item, inner, 0, 1) for item in value)
    body = "\n".join(" " * inner + line + "," for line in lines)
    return f"{opening}\n{body}\n{' ' * indent}{closing}"


def union(head: str, members: Sequence[str], indent: int = 0) -> str:
    """``head`` followed by the union of ``members``: on one line where it
    fits, else parenthesized, a member to a line, each after the first led by
    its ``|``."""
    pad = " " * indent
    line = f"{pad}{head}{' | '.join(members)}"
    if len(line) <= WIDTH or len(members) < 2:
        return line
    inner = " " * (indent + 4)
    body = "\n".join(
        f"{inner}{'| ' if position else ''}{member}"
        for position, member in enumerate(members)
    )
    return f"{pad}{head}(\n{body}\n{pad})"


def from_import(module: str, names: Iterable[str]) -> str:
    """``from module import names``, the names in isort's order, on one line
    where it fits and otherwise a name to a line in parentheses."""
    ordered = sorted(set(names), key=_isort_key)
    line = f"from {module} import {', '.join(ordered)}"
    if len(line) <= WIDTH:
        return line
    return bracket(f"from {module} import (", ordered, ")", explode=True)


def _isort_key(name: str) -> tuple[int, str]:
    """isort's order of the names one import gives, ``order_by_type``:
    constants, then classes, then functions and the rest, each by name."""
    bare = name.split(" as ")[0]
    if bare.isupper() and len(bare) > 1:
        return (0, bare)
    if bare[:1].isupper():
        return (1, bare)
    return (2, bare)
