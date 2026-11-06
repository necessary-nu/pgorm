"""Relational reference using explicit subqueries and parenthesized set operands."""

from dataclasses import dataclass, field, replace

from . import reference_order as order
from .catalog import OPERATIONS
from .comparison import InvalidOracle
from .reference_models import Model, registered
from .reference_pexpr import Expression, window
from .reference_sql import SQL, Rejection, Table, join
from .reference_values import qualified, quote
from .refusals import UNQUOTABLE, unquotable


@dataclass(frozen=True)
class Relation:
    statement: SQL
    columns: tuple
    shape: dict = field(default_factory=dict)
    grouping: tuple = ()
    ordering: tuple = ()

    def context(self, qualifier="q"):
        context, counts = {}, {}
        for index, (source, name) in enumerate(self.columns):
            value = SQL((qualified("c" + str(index), qualifier),))
            context[source, name] = value
            counts[name] = counts.get(name, 0) + 1
        for index, (_, name) in enumerate(self.columns):
            if counts[name] == 1:
                context[None, name] = SQL((qualified("c" + str(index), qualifier),))
        return context

    def projected(self, qualifier="q"):
        return [
            SQL((qualified("c" + str(index), qualifier),))
            for index in range(len(self.columns))
        ]

    def wrap(
        self,
        projections,
        *,
        columns=None,
        tail=None,
        distinct=False,
        shape=None,
        preserve_order=True,
        page=None,
    ):
        ordering = (
            order.retained(self.ordering, self.projected(), projections, distinct)
            if preserve_order
            else ()
        )
        statement = ("SELECT DISTINCT " if distinct else "SELECT ") + join(
            [
                value + " AS " + quote("c" + str(index))
                for index, value in enumerate(projections)
            ]
            + order.projections(ordering)
        )
        statement += " FROM (" + self.statement + ") AS q"
        if tail is not None:
            statement += tail
        statement += order.clause(ordering)
        if page is not None:
            statement += page
        return Relation(
            statement,
            self.columns if columns is None else tuple(columns),
            self.shape if shape is None else shape,
            ordering=ordering,
        )

    def sorted(self, keys):
        values, ordering = order.keys(keys, self.context(), self.projected())
        statement = (
            "SELECT "
            + join(
                [
                    value + " AS " + quote("c" + str(index))
                    for index, value in enumerate(self.projected())
                ]
                + values
            )
            + " FROM ("
            + self.statement
            + ") AS q"
        )
        return replace(self, statement=statement, ordering=ordering)

    def unordered_operand(self):
        return self.wrap(self.projected(), preserve_order=False).statement

    def terminal(self):
        names = [name for _, name in self.columns]
        if len(names) != len(set(names)):
            raise InvalidOracle("unaliased pipeline output has duplicate field names")
        return (
            "SELECT "
            + join(
                [
                    value + " AS " + quote(name)
                    for value, name in zip(self.projected(), names, strict=True)
                ]
            )
            + " FROM ("
            + self.statement
            + ") AS q"
            + order.clause(self.ordering)
        )


def source(value, alias, fixture):
    if isinstance(value, Relation):
        if alias is None:
            return value
        return replace(value, columns=tuple((alias, name) for _, name in value.columns))
    table = value.table if isinstance(value, Model) else value
    if not isinstance(table, Table):
        raise InvalidOracle("independent pipeline source is not a declared relation")
    table = replace(table, alias=alias or table.alias)
    definition = next(
        (
            item
            for item in fixture["tables"]
            if (item["schema"], item["name"]) == (table.schema, table.name)
        ),
        None,
    )
    if definition is None:
        raise InvalidOracle("pipeline oracle needs declared source columns")
    names = [column["name"] for column in definition["columns"]]
    projections = [
        table.column(name) + " AS " + quote("c" + str(index))
        for index, name in enumerate(names)
    ]
    return Relation(
        "SELECT " + join(projections) + " FROM " + table.source(),
        tuple((table.alias or table.name, name) for name in names),
    )


def combine(left, right, method):
    if len(left.columns) != len(right.columns):
        raise InvalidOracle("set reference operands have different arity")
    operator = {
        "append": "UNION ALL",
        "intersect": "INTERSECT ALL",
        "remove": "EXCEPT ALL",
    }[method]
    statement = (
        "("
        + left.unordered_operand()
        + ") "
        + operator
        + " ("
        + right.unordered_operand()
        + ")"
    )
    columns = (
        left.columns
        if method == "append"
        else tuple((None, name) for _, name in left.columns)
    )
    return Relation(statement, columns)


def joined(left, right, predicate, kind):
    context = left.context("l")
    other = right.context("r")
    for key in context.keys() & other.keys():
        if key[0] is not None:
            raise InvalidOracle("reference join has duplicate source names")
    ambiguous = context.keys() & other.keys()
    context = {
        key: value
        for key, value in {**context, **other}.items()
        if key not in ambiguous
    }
    values = left.projected("l") + right.projected("r")
    statement = "SELECT " + join(
        [value + " AS " + quote("c" + str(index)) for index, value in enumerate(values)]
        + order.projections(left.ordering, "l")
    )
    statement += (
        " FROM ("
        + left.statement
        + ") AS l "
        + kind.upper()
        + " JOIN ("
        + right.statement
        + ") AS r ON "
        + predicate.sql(context)
    )
    return Relation(statement, left.columns + right.columns, ordering=left.ordering)


def selected_sources(query, data):
    count = len(data["qualifiers"])
    if data["name"] != "campaign.Sources" + str(count) or not 1 <= count <= 6:
        raise InvalidOracle("independent source tuple registration unavailable")
    models = [
        registered("campaign.Account" if index == 0 else "campaign.Note")
        for index in range(count)
    ]
    context, projections, columns = query.context(), [], []
    for slot, (model, qualifier) in enumerate(
        zip(models, data["qualifiers"], strict=True)
    ):
        for index, (_, physical) in enumerate(model.fields):
            key = (qualifier, physical)
            if key not in context:
                raise InvalidOracle("selected source columns are no longer available")
            projections.append(context[key])
            columns.append((None, f"slot_{slot}_{index}"))
    return query.wrap(
        projections,
        columns=columns,
        shape={"slots": models, "optional_root": True, "tuple": True},
    )


def identifiers(name, i, d):
    """Every identifier one pipeline instruction hands the compiler.

    The declared fields the catalog types as identifiers — aliases, source
    qualifiers, column segments — plus the schema and name of a table or entity
    it reads from, which is the set pgorm's screen walks.
    """
    names = []
    for key, declared in OPERATIONS[name].data.items():
        value = d.get(key)
        if isinstance(declared, str) and declared.startswith("identifier"):
            names.extend(value if isinstance(value, list) else [value])
    source = i.get("source")
    table = source.table if isinstance(source, Model) else source
    if isinstance(table, Table):
        names.extend((table.schema, table.name))
    return [item for item in names if item is not None]


def screen(name, i, d):
    """Refuse an identifier no quoted PostgreSQL identifier can carry.

    `pipeline.errors+5` makes this a refusal rather than an escape: prqlc does
    the quoting, and which prqlc a consumer links decides what an embedded `"`
    becomes, so pgorm refuses the name before compiling. The reference applies
    the same closed rule — a double quote or NUL anywhere, a leading `$`, or a
    lone `*` — to what it resolves.
    """
    for identifier in identifiers(name, i, d):
        if unquotable(identifier):
            raise Rejection("ConstructionError", UNQUOTABLE.format(identifier))


def pipeline_node(name, i, d, fixture):
    screen(name, i, d)
    operation = name.removeprefix("pipeline.")
    if operation in (
        "column",
        "alias",
        "value",
        "bind",
        "binary",
        "unary",
        "membership",
        "function",
        "cast",
        "named",
    ):
        return Expression(operation, i, d)
    if operation in ("source", "from"):
        return source(i["source"], d.get("alias"), fixture)
    query = i["query"]
    if operation == "join":
        return joined(query, source(i["source"], None, fixture), i["on"], d["kind"])
    if operation == "set":
        return combine(query, i["source"], d["method"])
    if operation == "sources":
        return selected_sources(query, d)
    context = query.context()
    if operation == "filter":
        return query.wrap(
            query.projected(), tail=" WHERE " + i["predicate"].sql(context)
        )
    if operation == "sort":
        return query.sorted(i["keys"])
    if operation == "take":
        if d["start"] < 1 or d["end"] < d["start"]:
            raise InvalidOracle("reference take range is inclusive and starts at one")
        return query.wrap(
            query.projected(),
            page=SQL(
                (
                    " OFFSET "
                    + str(d["start"] - 1)
                    + " LIMIT "
                    + str(d["end"] - d["start"] + 1),
                )
            ),
        )
    if operation == "distinct":
        # The public distinct contract is group this (take 1); grouping resets
        # the output order. A later sort is required for ordered observations.
        return query.wrap(query.projected(), distinct=True, preserve_order=False)
    if operation == "group":
        return replace(query, grouping=tuple(i["keys"]))
    if operation in ("select", "derive", "window", "aggregate"):
        if operation == "window" and not i["partition"] and i["order"]:
            # An unpartitioned window's authored sort also orders its relation.
            query = query.sorted(i["order"])
        over = window(i, d, context) if operation == "window" else None
        added = i["columns"]
        expressions = [item.sql(context, over) for item in added]
        columns = [(None, item.name()) for item in added]
        tail = None
        if operation in ("derive", "window"):
            retained = [
                index
                for index, (_, name) in enumerate(query.columns)
                if name not in {name for _, name in columns}
            ]
            expressions = [query.projected()[index] for index in retained] + expressions
            columns = [query.columns[index] for index in retained] + columns
        elif operation == "aggregate":
            expressions = [item.sql(context) for item in query.grouping] + expressions
            columns = [(None, item.name()) for item in query.grouping] + columns
            if query.grouping:
                tail = " GROUP BY " + join(
                    [item.sql(context) for item in query.grouping]
                )
        return query.wrap(
            expressions,
            columns=columns,
            tail=tail,
            shape={},
            preserve_order=operation != "aggregate"
            and not (operation == "window" and i["partition"]),
        )
    raise InvalidOracle("independent pipeline stage semantics uncovered: " + operation)
