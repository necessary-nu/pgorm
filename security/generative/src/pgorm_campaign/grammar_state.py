"""Typed generation state and bounded deterministic choices."""

from dataclasses import dataclass
from functools import lru_cache
from hashlib import sha256

from . import wire
from .author import Author
from .corpus import encoded
from .corpus_builtin import builtin
from .corpus_random import entropy
from .parameters import input_ids

VERSION = 1
DEFAULT_INPUTS = builtin()


@dataclass(frozen=True)
class Limits:
    depth: int = 2
    stages: int = 4
    nodes: int = 256

    def __post_init__(self):
        if not (
            type(self.depth) is int
            and 1 <= self.depth <= 3
            and type(self.stages) is int
            and 1 <= self.stages <= 5
            and type(self.nodes) is int
            and 128 <= self.nodes <= 256
        ):
            raise ValueError(
                "grammar budgets require depth 1..3, stages 1..5 and nodes 128..256"
            )


@dataclass(frozen=True)
class Expression:
    node: str
    kind: str
    nullable: bool


@dataclass(frozen=True)
class Field:
    name: str
    kind: str
    nullable: bool


@dataclass(frozen=True)
class Source:
    node: str
    alias: str
    fields: tuple[Field, ...]
    kind: str = "table"
    slot: int = 0

    def field(self, name):
        return next(field for field in self.fields if field.name == name)


class Choices:
    def __init__(self, seed, index):
        self.key, self.counter = entropy(seed, index), 0

    def take(self, values):
        if not values:
            raise ValueError("no compatible grammar production")
        raw = sha256(self.key + self.counter.to_bytes(8, "big")).digest()
        self.counter += 1
        return values[int.from_bytes(raw[:8], "big") % len(values)]

    def integer(self, low, high):
        return self.take(range(low, high + 1))


@lru_cache(maxsize=4)
def input_pool(inputs):
    texts, names = [], []
    for item in inputs:
        value = item.value()
        if value["type"]["kind"] != "text" or value["sql_null"]:
            continue
        text = value["data"]
        if "\0" not in text and len(text.encode("utf-8")) <= 4096:
            texts.append(item)
            if 0 < len(text.encode("utf-8")) <= 40:
                names.append(item)
    return tuple(texts), tuple(names)


@lru_cache(maxsize=4)
def name_pools(names):
    """Identifier inputs split on the one character the pipeline refuses.

    `Pipeline::into_sql` refuses an identifier carrying a double quote rather
    than escaping it (`pipeline.errors+5`), so a valid pipeline program can only
    draw from the names without one. The names that do carry one are what the
    rejection family draws to check that refusal. The screen's other shapes, a
    leading `$` and a lone `*`, never arise here: every drawn name is appended
    to a non-empty prefix (`State.name`).
    """
    return {
        False: tuple(item for item in names if '"' not in item.value()["data"]),
        True: tuple(item for item in names if '"' in item.value()["data"]),
        None: names,
    }


# [spec:pgorm:req:generative.grammar]
class State:
    def __init__(self, seed, index, *, limits=None, corpus=()):
        if type(index) is not int or not 0 <= index <= 2**31 - 1024:
            raise ValueError(
                "program index must leave room for int4 fixture identities"
            )
        self.seed, self.index = seed, index
        self.limits, self.choices = limits or Limits(), Choices(seed, index)
        self.author = Author(seed=seed)
        self.inputs = tuple(corpus) or DEFAULT_INPUTS
        self.texts, self.names = input_pool(self.inputs)
        if not self.texts or not self.names:
            raise ValueError(
                "grammar corpus needs compatible text and identifier inputs"
            )
        self.used_inputs = set()

    def node(self, operation, inputs=None, data=None, *, scope="root"):
        if len(self.author.nodes) >= self.limits.nodes:
            raise ValueError("generated program exceeds its declared node budget")
        return self.author.node(operation, inputs, data, scope=scope)

    def value(self, kind, data, *, sql_null=False):
        if isinstance(kind, str) and kind in wire.INTEGER_BITS and not sql_null:
            data = str(data)
        return self.node(
            "value", data={"value": wire.scalar(kind, data, sql_null=sql_null)}
        )

    def text(self):
        item = self.choices.take(self.texts)
        self.used_inputs.add(item.data()["id"])
        return item.value()["data"]

    def name(self, prefix, *, quote=None):
        """A prefixed identifier input; `quote` selects names with or without `"`."""
        item = self.choices.take(name_pools(self.names)[quote])
        self.used_inputs.add(item.data()["id"])
        return prefix + item.value()["data"]

    def source(self, name="accounts", *, alias=None, schema="fixture", optional=False):
        alias = alias or self.name("s" + str(len(self.author.nodes)))
        definition = next(
            table
            for table in self.author.fixture["tables"]
            if table["schema"] == schema and table["name"] == name
        )
        fields = tuple(
            Field(
                column["name"],
                column["kind"],
                optional or column.get("nullable", False),
            )
            for column in definition["columns"]
            if isinstance(column["kind"], str)
        )
        node = self.node("table", data={"schema": schema, "name": name, "alias": alias})
        return Source(node, alias, fields)

    def column(self, source, name):
        field = source.field(name)
        if source.kind == "graph":
            node = self.node(
                "graph.column",
                {"query": source.node},
                {"source": source.slot, "column": name},
            )
        elif source.kind in ("entity", "model"):
            node = self.node(
                source.kind + ".column", {source.kind: source.node}, {"name": name}
            )
        else:
            node = self.node("expr.column", {"table": source.node}, {"name": name})
        return Expression(
            node,
            field.kind,
            field.nullable,
        )

    def constant(self, kind, data, *, mode=None, nullable=False):
        value = self.value(kind, data, sql_null=nullable)
        expression = self.node(
            "expr.value",
            {"value": value},
            {"mode": mode or self.choices.take(("bound", "literal"))},
        )
        postgres = {"i32": "int4", "i64": "int8", "text": "text", "bool": "bool"}[kind]
        node = self.node(
            "expr.cast",
            {"value": expression},
            {"schema": "pg_catalog", "name": postgres},
        )
        return Expression(node, kind, nullable)

    def binary(self, left, right, operator):
        if left.kind != right.kind:
            raise ValueError("binary expression mixes incompatible schema types")
        comparison = operator in ("eq", "ne", "lt", "lte", "gt", "gte", "and", "or")
        node = self.node(
            "expr.binary",
            {"left": left.node, "right": right.node},
            {"operator": operator},
        )
        return Expression(
            node, "bool" if comparison else left.kind, left.nullable or right.nullable
        )

    def fetch(self, query, *, scope="root", ordered=False, error=None):
        return self.author.effect(
            "fetch",
            {"query": query},
            {"mode": "all", "ordered": ordered},
            scope=scope,
            error=error,
        )

    def finish(self):
        nodes = {node["id"]: node for node in self.author.nodes}
        pending = [
            ref for step in self.author.steps for ref in input_ids(step["inputs"])
        ]
        seen = set()
        while pending:
            node = pending.pop()
            if node not in seen:
                seen.add(node)
                pending.extend(input_ids(nodes[node]["inputs"]))
        self.author.nodes = [node for node in self.author.nodes if node["id"] in seen]
        self.author.binders = [
            binder for binder in self.author.binders if binder["owner"] in seen
        ]
        return self.author.finish()


def structure(program):
    """Fingerprint topology and API choices without fixture values or seed text."""
    data = program.data()
    nodes = []
    mapping = {node["id"]: index for index, node in enumerate(data["nodes"])}
    for node in data["nodes"]:
        options = {
            key: value
            for key, value in node["data"].items()
            if key
            in ("operator", "method", "mode", "kind", "state", "direction", "negated")
        }
        nodes.append(
            {
                "op": node["op"],
                "inputs": {
                    key: [mapping[v] for v in value]
                    if isinstance(value, list)
                    else mapping[value]
                    for key, value in node["inputs"].items()
                },
                "options": options,
                "bound": node["scope"] != "root",
            }
        )
    return sha256(
        encoded({"nodes": nodes, "steps": [step["op"] for step in data["steps"]]})
    ).hexdigest()
