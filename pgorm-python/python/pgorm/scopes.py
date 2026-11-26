"""Scopes, the dialect of Rust's ``Scope``, ``Every`` and ``ScopeContext``:
the program's own types a scoped table's statements are held to, and the
contexts a statement is built under.

A scope is a subclass of ``Scope``, ``class CompanyId(Scope[int])``, named by
its class as Rust names one by its type; an instance holds one value of it
and is a context holding itself. ``Every(UserId)`` holds every value of a
scope, lifted on purpose. A subclass of ``ScopeContext``, a dataclass as a
rule, is a context holding each of its fields, each a scope value or an
``Every``. What a context holds a statement to is pgorm-bindings' to decide;
this module reads a context into the items it decides it from."""

from __future__ import annotations

import dataclasses
from typing import Any

from . import _native as p


# [spec:pgorm:req:python.scopes]
class Scope[T]:
    """A scope of the program's own, made by subclassing: an instance holds
    one value, ``CompanyId(7)``, and a statement under it is held to the
    rows of that value. A column holds a scope where it is declared
    ``scope=CompanyId``; its kind converts the value, as it converts a
    value written to it."""

    __slots__ = ("value",)
    value: T

    def __init__(self, value: T) -> None:
        if type(self) is Scope:
            raise TypeError("a scope is the program's own subclass: class CompanyId(Scope[int])")
        object.__setattr__(self, "value", value)

    def __setattr__(self, name: str, value: object) -> None:
        raise AttributeError("a scope value cannot be changed")

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Scope) and type(other) is type(self) and other.value == self.value

    def __hash__(self) -> int:
        return hash((type(self), self.value))

    def __repr__(self) -> str:
        return f"{type(self).__name__}({self.value!r})"


def scope_name(scope: object) -> str:
    """The name of the scope ``scope`` is, refused unless it is one."""
    if not isinstance(scope, type) or not issubclass(scope, Scope) or scope is Scope:
        raise TypeError("a scope is a subclass of Scope: class CompanyId(Scope[int])")
    return scope.__name__


# [spec:pgorm:req:python.scopes]
class Every[S: Scope[Any]]:
    """Every value of the scope ``S``, lifted on purpose: a statement under it
    is held to no value of ``S``, and an insert takes the scope's column
    from its row, which must set it."""

    __slots__ = ("scope",)
    scope: type[S]

    def __init__(self, scope: type[S]) -> None:
        scope_name(scope)
        object.__setattr__(self, "scope", scope)

    def __setattr__(self, name: str, value: object) -> None:
        raise AttributeError("an Every cannot be changed")

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Every) and other.scope is self.scope

    def __hash__(self) -> int:
        return hash((Every, self.scope))

    def __repr__(self) -> str:
        return f"Every({self.scope.__name__})"


# [spec:pgorm:req:python.scopes]
class ScopeContext:
    """The base of a context of several scopes, as Rust's
    ``#[derive(ScopeContext)]`` makes a struct one: each field of a subclass,
    a dataclass as a rule, is a scope value or an ``Every``, and a context
    holds each scope once."""

    __slots__ = ()


type Context = Scope[Any] | Every[Any] | ScopeContext


def _item(value: object, owner: str) -> tuple[str, object, bool]:
    if isinstance(value, Scope):
        return type(value).__name__, value.value, False
    if isinstance(value, Every):
        return value.scope.__name__, None, True
    raise TypeError(f"{owner}'s field is a scope value or an Every, not {type(value).__name__}")


def held(cx: object) -> p._ScopeContext:
    """``cx`` as the native context a statement is built under: what it holds
    of each scope, and its class's name, which a refusal names it by."""
    if isinstance(cx, (Scope, Every)):
        label = type(cx).__name__ if isinstance(cx, Scope) else repr(cx)
        return p._ScopeContext(label, [_item(cx, label)])
    if isinstance(cx, ScopeContext):
        label = type(cx).__name__
        if dataclasses.is_dataclass(cx):
            values = [getattr(cx, field.name) for field in dataclasses.fields(cx)]
        else:
            values = list(vars(cx).values())
        return p._ScopeContext(label, [_item(value, label) for value in values])
    raise TypeError("a context is a scope value, an Every or a ScopeContext")
