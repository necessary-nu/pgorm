"""Refusals the binding documents, spelled exactly as it reports them.

An exact-error declaration names one of these, and the independent reference
has to arrive at the same refusal from its own model of the rule, so the text
lives here once rather than in both places. Each entry names the rule that
makes it a refusal rather than a defect.
"""

# `pipeline.errors+4`: an identifier carrying `"` or NUL, beginning with `$`,
# or equal to `*` is refused at `into_sql`, before prqlc sees it, rather than
# escaped.
UNQUOTABLE = (
    "identifier `{}` cannot be written as a pipeline name: it contains a double "
    "quote or NUL byte, begins with `$`, or is `*`; rename it"
)

# pgorm-python's statement builder refuses an insert with no rows, a select
# or `default_values()` (the Python guide's query-builder chapter, under
# INSERT): an empty batch is refused at compilation, before any statement
# reaches the server.
EMPTY_INSERT = "insert requires rows, a select or explicit default_values"

# `bindings.errors+5`: a value tokio-postgres cannot encode while building the
# bind message is a ConstructionError naming the parameter by its one-based
# placeholder, followed by pgorm's refusal of the value.
UNSERIALIZABLE = "parameter ${} cannot be bound: {}"

# pgorm's refusal of a value its parameter's type cannot hold, naming the
# value and the type (`exec.cursor.binding-coerce+2`).
OUT_OF_RANGE = "value `{}` is out of range for Postgres type `{}`"

# The largest u64 an int8 carrier can hold (`exec.cursor.binding-coerce+2`).
CARRIED_UNSIGNED = 2**63 - 1


def unsigned_overflow(value):
    """The first u64 a tagged value is, or holds, that no int8 carrier can
    take, as its digits, or None: an array is encoded element by element, so
    the first such element is the one refused."""
    tag = value["type"]
    if value["sql_null"]:
        return None
    if tag["kind"] == "array":
        return next(
            (digits for item in value["data"] if (digits := unsigned_overflow(item))),
            None,
        )
    if tag["kind"] == "u64" and int(value["data"]) > CARRIED_UNSIGNED:
        return value["data"]
    return None


def unquotable(name):
    """Whether the pipeline refuses this identifier outright."""
    return '"' in name or "\0" in name or name.startswith("$") or name == "*"


__all__ = [
    "CARRIED_UNSIGNED",
    "EMPTY_INSERT",
    "OUT_OF_RANGE",
    "UNQUOTABLE",
    "UNSERIALIZABLE",
    "unquotable",
    "unsigned_overflow",
]
