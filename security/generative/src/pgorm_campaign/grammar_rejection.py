"""Deliberate rejections, each labelled with the exact cause it must produce.

An invalid program is a claim about where a line is drawn — by PostgreSQL, or
by pgorm and its binding before anything is sent. Every rule declares the
precise error, a SQLSTATE or the binding's documented refusal text, so an
arbitrary failure can never satisfy it, and the independent reference has to
reach the same rejection from its own model of the rule. A valid-mode failure
is never relabelled as one of these.
"""

import math
import struct

from . import baseline, wire
from .grammar_pipeline import Pipeline
from .grammar_sequence import account_row, accounts, tenant_guard
from .grammar_state import DEFAULT_INPUTS
from .refusals import (
    CARRIED_UNSIGNED,
    EMPTY_INSERT,
    OUT_OF_RANGE,
    UNQUOTABLE,
    UNSERIALIZABLE,
    unsigned_overflow,
)


def _database(sqlstate):
    return {"class": "DatabaseError", "cause": "sqlstate:" + sqlstate}


def _division(state):
    left = state.constant("i32", state.choices.integer(1, 100))
    right = state.constant("i32", 0)
    expression = state.binary(left, right, "div")
    query = state.node(
        "select",
        {"columns": [expression.node, state.constant("i64", state.index).node]},
    )
    state.fetch(query, error=_database("22012"))


def _not_null(state):
    table = accounts(state)
    query = state.node("update", {"table": table})
    query = state.node(
        "update.set",
        {"query": query, "value": state.value("text", None, sql_null=True)},
        {"column": "name"},
    )
    query = state.node(
        "write.filter",
        {
            "query": query,
            "predicate": tenant_guard(state, table, state.value("i32", 1)),
        },
    )
    state.author.effect("execute", {"query": query}, error=_database("23502"))


def _duplicate(state):
    table = accounts(state)
    columns, values = account_row(state, 1)
    query = state.node("insert", {"table": table}, {"columns": columns})
    query = state.node("insert.row", {"query": query, "values": values})
    state.author.effect("execute", {"query": query}, error=_database("23505"))


def _unquotable(state):
    """A pipeline alias carrying `"` is refused at `into_sql`, not escaped.

    `pipeline.errors+4` refuses rather than escapes because prqlc does the
    quoting and which prqlc a consumer links decides what the quote becomes.
    Exactly one identifier in the program carries a quote, so the refusal can
    only name that one.
    """
    alias = state.name("origin", quote=True)
    query = Pipeline(state, alias)
    query.project()
    state.fetch(
        query.query,
        error={"class": "ConstructionError", "cause": UNQUOTABLE.format(alias)},
    )


def _empty_batch(state):
    """An insert given columns and no rows writes nothing, by refusing.

    The binding documents the empty batch as an error rather than a zero:
    compilation refuses an insert with no rows and no explicit
    `default_values()`, so no statement reaches the server. The final fixture
    comparison is the evidence that nothing was written, whichever terminal —
    a counted execute or a RETURNING fetch — was asked for.
    """
    table = accounts(state)
    names = [item["name"] for item in baseline.default()["tables"][0]["columns"]]
    columns = [name for name in names if state.choices.take((False, True))]
    query = state.node("insert", {"table": table}, {"columns": columns or names})
    error = {"class": "ConstructionError", "cause": EMPTY_INSERT}
    if state.choices.take((False, True)):
        returning = state.node("expr.column", {"table": table}, {"name": "id"})
        query = state.node("write.returning", {"query": query, "columns": [returning]})
        state.fetch(query, error=error)
    else:
        state.author.effect("execute", {"query": query}, error=error)


def _drawn(state, kind, admits):
    """A corpus value of `kind` the rule admits, recorded as a drawn input."""
    item = state.choices.take(
        tuple(
            item
            for item in DEFAULT_INPUTS
            if item.value()["type"]["kind"] == kind
            and not item.value()["sql_null"]
            and admits(item.value())
        )
    )
    state.used_inputs.add(item.data()["id"])
    return item.value()


def _select(state, value, mode, cast, *, error=None):
    """Fetch one cast value beside a literal nonce, declaring any rejection.

    The nonce is inlined, so the value is the statement's only bound
    parameter and a refusal naming a parameter position can only name it.
    """
    expression = state.node(
        "expr.value",
        {"value": state.node("value", data={"value": value})},
        {"mode": mode},
    )
    array = value["type"]["kind"] == "array"
    expression = state.node(
        "expr.cast", {"value": expression}, {**cast, "array": array}
    )
    columns = [
        state.node("expr.alias", {"value": expression}, {"name": "value"}),
        state.constant("i64", state.index, mode="literal").node,
    ]
    state.fetch(state.node("select", {"columns": columns}), error=error)


def _unsigned(state):
    """A u64 past i64::MAX has no int8 to travel as, and is refused.

    PostgreSQL has no unsigned 64-bit type. pgorm writes a u64 as int8 when it
    fits and refuses one past i64::MAX while encoding the bind message
    (`exec.cursor.binding-coerce+2`); inlined as a literal, the same digits
    reach the server as numeric and the int8 cast refuses them (22003). What
    does fit — a NULL, an empty or NULL array, small values beside a NULL
    element — is carried, and is checked against the reference first, so the
    program shows both halves of `construction-or-rejection`.
    """
    mode = state.choices.take(("bound", "literal"))
    fits = _drawn(state, "u64", lambda value: int(value["data"]) <= CARRIED_UNSIGNED)
    past = _drawn(state, "u64", lambda value: int(value["data"]) > CARRIED_UNSIGNED)
    null = wire.scalar("u64", None, sql_null=True)
    array = {"kind": "array", "element": {"kind": "u64"}}
    carried, refused = state.choices.take(
        (
            (null, past),
            (fits, past),
            (wire.scalar(array, []), wire.scalar(array, [past])),
            (wire.scalar(array, None, sql_null=True), wire.scalar(array, [null, past])),
            (wire.scalar(array, [fits, null]), wire.scalar(array, [past, null])),
        )
    )
    int8 = {"name": "int8", "schema": "pg_catalog"}
    _select(state, carried, mode, int8)
    refusal = OUT_OF_RANGE.format(unsigned_overflow(refused), "int8")
    error = (
        {"class": "ConstructionError", "cause": UNSERIALIZABLE.format(1, refusal)}
        if mode == "bound"
        else _database("22003")
    )
    _select(state, refused, mode, int8, error=error)


def _finite(value):
    return all(
        math.isfinite(struct.unpack(">f", bytes.fromhex(item))[0])
        for item in value["data"]
    )


def _vector(state):
    """The pinned image installs no pgvector, so the vector type is missing.

    pgorm casts a vector to the type named `vector`, unqualified, and the
    server refuses the statement at the type lookup (42704) before any value
    is examined — bound or literal, scalar or array, NULL or not. Elements are
    finite so the reference can still spell the value pgvector would read.
    """
    mode = state.choices.take(("bound", "literal"))
    present = _drawn(state, "vector", _finite)
    null = wire.scalar("vector", None, sql_null=True)
    array = {"kind": "array", "element": {"kind": "vector"}}
    value = state.choices.take(
        (
            present,
            null,
            wire.scalar(array, []),
            wire.scalar(array, None, sql_null=True),
            wire.scalar(array, [present, null]),
        )
    )
    _select(state, value, mode, {"name": "vector"}, error=_database("42704"))


RULES = {
    "division": _division,
    "not-null": _not_null,
    "duplicate": _duplicate,
    "unquotable-identifier": _unquotable,
    "empty-batch": _empty_batch,
    "unsigned-overflow": _unsigned,
    "missing-vector-type": _vector,
}


# [spec:pgorm:req:generative.grammar]
def rejection(state):
    """An intentional rejection; never relabel a valid-mode failure."""
    rule = state.choices.take(tuple(RULES))
    RULES[rule](state)
    return rule


__all__ = ["RULES", "rejection"]
