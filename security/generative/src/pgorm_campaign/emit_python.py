"""Emit a standalone runnable Python reproducer for a portable program.

The emitted script imports only ``pgorm`` and the standard library, reaches the
same public calls in the same order as the campaign interpreter, and prints one
executor-shaped report. Identifiers and values travel as Python literals; a
builder operation is never replaced by captured SQL.
"""

import math

from . import baseline, catalog, parameters, wire
from .program import CALLBACK_INPUTS, Program

ARITHMETIC = {
    "add": "+",
    "sub": "-",
    "mul": "*",
    "div": "/",
    "mod": "%",
    "and": "&",
    "or": "|",
}
LIKE = ("like", "not_like", "ilike", "not_ilike")
SCHEMA_TYPES = {
    "i16": "smallint",
    "i32": "integer",
    "i64": "bigint",
    "f32": "real",
    "f64": "double",
    "bool": "boolean",
    "bytes": "bytea",
    "decimal": "numeric",
    "json": "jsonb",
}
MODEL_TEMPORAL = {"timestamp": "datetime", "timestamptz": "datetime_utc"}

HEADER = '''"""Standalone reproducer for generated program {digest}.

The declared baseline is applied on startup, so this expects a database
prepared the way the campaign prepares one, including its ``campaign`` role:

    python replay.py postgresql://campaign:secret@127.0.0.1:5432/subject

The URL may also arrive in PGORM_REPLAY_URL or DATABASE_URL. One JSON report in
the executor shape is printed on stdout.
"""

import asyncio
import json
import os
import struct
import sys
from datetime import date, datetime, time
from decimal import Decimal
from uuid import UUID

import pgorm as p
from pgorm import pipeline as pl
from pgorm import schema as sch
from pgorm.models import Column, Model'''

PRELUDE = """def _f32(bits):
    return struct.unpack(">f", bytes.fromhex(bits))[0]


def _f64(bits):
    return struct.unpack(">d", bytes.fromhex(bits))[0]


def _construction_error(message):
    raise p.ConstructionError(message)


def _result_value(rows, row, column, kind):
    value = rows[row].tagged(column)
    if value.snapshot()["type"] != kind:
        raise p.DecodeError("result reference type differs from the declared value tag")
    return value


def _entity_result(rows, row, source=None):
    value = rows[row]
    if source is not None:
        value = value[source]
    if not isinstance(value, p.EntityModel):
        raise p.DecodeError("result reference requires a registered EntityModel")
    return value


def _row(value):
    if value is None:
        return {"kind": "absent"}
    if isinstance(value, tuple):
        return {"kind": "tuple", "items": [_row(item) for item in value]}
    fields = [
        {"name": name, "value": value.tagged(name).snapshot()} for name in value.keys()
    ]
    result = {"kind": "record", "fields": fields}
    if hasattr(value, "entity_name"):
        result["entity"] = value.entity_name
    native = getattr(value, "native", value)
    if hasattr(native, "fields"):
        result["postgres"] = [
            {
                "name": field.name,
                "type": field.type_name.name,
                "schema": field.type_name.schema,
            }
            for field in native.fields
        ]
    return result


def _compiled(value):
    return {
        "kind": "compiled",
        "sql": value.sql,
        "parameters": [item.snapshot() for item in value.params],
    }


def _error(value):
    return {
        "kind": "error",
        "class": type(value).__name__,
        "cause": str(value),
        "sqlstate": getattr(value, "sqlstate", None),
    }


async def _fetch(query, mode, connection):
    for category, prefix in (
        (p.EntityQuery, "pgorm::Select"),
        (p.EntityGraphQuery, "pgorm::SelectGraph"),
        (p.EntityGraphCursor, "pgorm::Cursor"),
        (pl.Pipeline, "pgorm::pipeline::Pipeline"),
        (pl.SelectedSources, "pgorm::pipeline::SelectedSources"),
        (p.ModelQuery, "pgorm::ConnectionTrait::query_raw"),
        (p.ModelRows, "pgorm::ConnectionTrait::query_raw"),
    ):
        if isinstance(query, category):
            if not hasattr(query, mode):
                raise p.UnsupportedCapabilityError(
                    "query has no public " + mode + " terminal"
                )
            value = await getattr(query, mode)(connection)
            rust = "one_opt" if mode == "optional" else mode
            path = (
                prefix
                if category in (p.ModelQuery, p.ModelRows)
                else prefix + "::" + rust
            )
            break
    else:
        value = await getattr(connection, "query" if mode == "all" else mode)(query)
        path = "pgorm::ConnectionTrait::query_raw"
    rows = value if mode == "all" else ([] if value is None else [value])
    return {"kind": "rows", "rows": [_row(item) for item in rows]}, [path], rows


async def _execute(query, connection):
    value = (
        await query.execute(connection)
        if isinstance(query, (p.ModelInsert, p.ModelUpdate, p.ModelDelete))
        else await connection.execute(query)
    )
    paths = ["pgorm::ConnectionTrait::execute_raw"]
    return {"kind": "count", "value": value}, paths, []


async def _active(model, method, connection):
    value = await getattr(model, method)(connection)
    paths = ["pgorm::ActiveModelTrait::" + method]
    if method == "delete":
        return {"kind": "count", "value": value}, paths, []
    return {"kind": "rows", "rows": [_row(value)]}, paths, [value]


def _conflict(conflict, update):
    target = p.Conflict.on(*conflict)
    return target.update(*update) if update else target.do_nothing()


async def _versions(name, inputs, data, connection):
    if name == "entity.change":
        model = inputs["model"]
        change = await p.entity(model.entity_name).update(model).returning_change(connection)
        rows, paths = [(change.old, change.new)], ["pgorm::UpdateOne::exec_returning_change"]
    elif name == "entity.changes":
        update = inputs["entity"].update_many()
        for column, value in zip(data["columns"], inputs["values"], strict=True):
            update = update.set(column, value)
        changes = await update.where(inputs["predicate"]).returning_changes(connection)
        rows = [(change.old, change.new) for change in changes]
        paths = ["pgorm::UpdateMany::exec_returning_changes"]
    else:
        models = inputs["models"]
        entity = p.entity(models[0].entity_name)
        action = _conflict(data["conflict"], data["update"])
        if data["rows"] == "one":
            row = await entity.insert(models[0]).on_conflict(action).returning_upsert(connection)
            written, path = ([] if row is None else [row]), "exec_returning_upsert"
        else:
            insert = entity.insert_many(models).on_conflict(action)
            written, path = await insert.returning_upserts(connection), "exec_returning_upserts"
        rows = [
            (None, row.model)
            if isinstance(row, p.Upserted.Inserted)
            else (row.change.old, row.change.new)
            for row in written
        ]
        paths = ["pgorm::Insert::" + path]
    return {"kind": "rows", "rows": [_row(row) for row in rows]}, paths, rows


def _inspect(query):
    if not hasattr(query, "inspect"):
        raise p.UnsupportedCapabilityError("query has no public inspect terminal")
    prefix = (
        "pgorm::pipeline::Pipeline::into_sql"
        if isinstance(query, (pl.Pipeline, pl.SelectedSources))
        else "pgorm_query::QueryStatementBuilder"
    )
    return _compiled(query.inspect()), [prefix], []


async def _stream(query, take, cancel, connection):
    records = []
    cancelled = False
    complete = False
    async with await connection.stream(query) as stream:
        for _ in range(take):
            try:
                records.append(await anext(stream))
            except StopAsyncIteration:
                complete = True
                break
        if cancel and not complete:
            pending = asyncio.create_task(anext(stream))
            try:
                await asyncio.sleep(0)
                pending.cancel()
                try:
                    await pending
                except asyncio.CancelledError:
                    cancelled = True
                except StopAsyncIteration:
                    complete = True
            finally:
                if not pending.done():
                    pending.cancel()
                    await asyncio.gather(pending, return_exceptions=True)
    state = {"complete": complete, "cancelled": cancelled, "closed": stream.closed}
    observation = {
        "kind": "rows",
        "rows": [_row(item) for item in records],
        "stream": state,
    }
    paths = ["pgorm::ConnectionTrait::query_raw", "tokio_postgres::RowStream"]
    return observation, paths, records


# Raised through a scope's block, so that its transaction rolls back.
class _RolledBack(Exception):
    pass


async def _reacquire(connection, pool, scoped):
    if not connection.closed:
        return connection
    # Stream cancellation can discard the root connection.
    if scoped:
        raise RuntimeError("connection closed with outstanding transactions")
    return await pool.acquire()


def _url():
    if len(sys.argv) > 2:
        raise SystemExit("usage: " + sys.argv[0] + " [DATABASE_URL]")
    if len(sys.argv) == 2:
        return sys.argv[1]
    for name in ("PGORM_REPLAY_URL", "DATABASE_URL"):
        value = os.environ.get(name)
        if value:
            return value
    raise SystemExit("supply the database URL in argv[1] or PGORM_REPLAY_URL")


async def _apply_fixture(connection):
    for statement in FIXTURE_STATEMENTS:
        await connection.execute(p.RawSQL(statement))"""

DESCRIPTOR = """def _column_kind(kind):
    if isinstance(kind, dict):
        schema, name = kind["enum"]
        return p.TypeName(name, schema=schema), False
    array = kind.endswith("[]")
    base = kind[:-2] if array else kind
    temporal = {"timestamp": "datetime", "timestamptz": "datetime_utc"}
    return temporal.get(base, base), array


def _descriptor(table, fields):
    definition = next(
        (
            item
            for item in FIXTURE_TABLES
            if (item["schema"], item["name"]) == (table.schema, table.name)
        ),
        None,
    )
    if definition is None:
        raise p.ConstructionError("runtime model requires declared fixture metadata")
    columns = {column["name"]: column for column in definition["columns"]}
    mapping = fields if fields is not None else {name: name for name in columns}
    declarations = {}
    for field, physical in mapping.items():
        column = columns[physical]
        kind, array = _column_kind(column["kind"])
        declarations[field] = Column(
            kind,
            name=physical,
            array=array,
            nullable=column["nullable"],
            primary_key=column["primary"],
        )
    return Model(table, declarations)"""

MAIN = """async def _main():
    report = {
        "program_sha256": PROGRAM_SHA256,
        "status": "error",
        "steps": [],
        "cleanup_errors": [],
        "builds": 0,
    }
    pool = p.Pool(
        _url(),
        max_size=2,
        connect_timeout=5,
        acquire_timeout=5,
        statement_cache_size=64,
    )
    try:
        await _run(pool, report)
    except Exception as error:
        report["error"] = _error(error)
    finally:
        try:
            await pool.close()
        except Exception as error:
            report["cleanup_errors"].append(_error(error))
    report["status"] = (
        "executed"
        if "error" not in report
        and not report["cleanup_errors"]
        and len(report["steps"]) == STEP_COUNT
        and all(step["status"] == "observed" for step in report["steps"])
        else "error"
    )
    print(json.dumps(report, ensure_ascii=True, sort_keys=True))


if __name__ == "__main__":
    asyncio.run(_main())"""

OBSERVED = (
    '_step.update(status="observed", native_paths=_paths, observation=_observation)'
)

CLEANUP = """    finally:
        try:
            await connection.close()
        except Exception as error:
            report["cleanup_errors"].append(_error(error))"""


class UnsupportedInstruction(Exception):
    """A catalog instruction that the public Python API cannot express."""


def literal(value):
    """Encode data as a Python literal so hostile text cannot leave its quotes."""
    if value is None or isinstance(value, (str, bool, int)):
        return repr(value)
    if isinstance(value, float):
        if not math.isfinite(value):
            raise UnsupportedInstruction("a nonfinite float has no Python literal")
        return repr(value)
    if isinstance(value, (list, tuple)):
        return "[" + ", ".join(literal(item) for item in value) + "]"
    if isinstance(value, dict):
        items = (f"{literal(key)}: {literal(item)}" for key, item in value.items())
        return "{" + ", ".join(items) + "}"
    raise UnsupportedInstruction("value has no Python literal: " + type(value).__name__)


def call(target, method, arguments):
    return f"{target}.{method}(" + ", ".join(arguments) + ")"


def statements(sql):
    """Split rendered fixture SQL at the newlines its renderer joined it with."""
    parts, current, single, double = [], [], False, False
    for char in sql:
        if char == "'" and not double:
            single = not single
        elif char == '"' and not single:
            double = not double
        if char == "\n" and not single and not double:
            parts.append("".join(current))
            current = []
        else:
            current.append(char)
    parts.append("".join(current))
    rendered = [part for part in parts if part]
    if any(not part.endswith(";") for part in rendered):
        raise ValueError("rendered fixture SQL is not one statement per line")
    return rendered


def type_name(name, schema):
    return f"p.TypeName({literal(name)}, schema={literal(schema)})"


def kind_source(tag):
    if tag["kind"] == "enum":
        return type_name(tag["name"], tag["schema"])
    return literal(tag["kind"])


def scalar_source(tag, data):
    name = tag["kind"]
    if name in wire.INTEGER_BITS:
        return literal(int(data))
    if name in ("f32", "f64"):
        return f"_{name}({literal(data)})"
    if name in ("bytes", "mac_address"):
        return f"bytes({literal(list(data))})"
    if name == "decimal":
        return f"Decimal({literal(data)})"
    if name == "uuid":
        return f"UUID({literal(data)})"
    if name in ("date", "time"):
        return f"{name}.fromisoformat({literal(data)})"
    if name.startswith("datetime"):
        return f"datetime.fromisoformat({literal(wire.temporal_text(data))})"
    if name == "vector":
        items = ", ".join(f"_f32({literal(item)})" for item in data)
        return "[" + items + "]"
    if name == "interval":
        months, days, microseconds = wire.interval_fields(data)
        return f"p.Interval(months={months}, days={days}, microseconds={microseconds})"
    return literal(data)


def value_source(snapshot):
    """Rebuild a tagged value through the public Value constructors."""
    wire.validate(snapshot)
    tag = snapshot["type"]
    if tag["kind"] == "array":
        items = (
            "None"
            if snapshot["sql_null"]
            else "[" + ", ".join(value_source(item) for item in snapshot["data"]) + "]"
        )
        return f"p.Value.array({kind_source(tag['element'])}, {items})"
    if snapshot["sql_null"]:
        return f"p.Value.null({kind_source(tag)})"
    if tag["kind"] == "json":
        return f"p.Value.json({literal(snapshot['data'])})"
    if tag["kind"] in wire.RANGES:
        return f"p.Value({range_source(tag, snapshot['data'])}, {kind_source(tag)})"
    return f"p.Value({scalar_source(tag, snapshot['data'])}, {kind_source(tag)})"


def range_source(tag, data):
    """A public ``pgorm.Range`` of the payload's bounds."""
    if data == {"empty": True}:
        return "p.Range.empty()"
    bound = {"kind": wire.RANGES[tag["kind"]]}
    lower, upper = (
        "None" if data[side] is None else scalar_source(bound, data[side])
        for side in ("lower", "upper")
    )
    return f"p.Range({lower}, {upper}, {literal(data['bounds'])})"


def datatype_source(kind):
    if isinstance(kind, dict):
        schema, name = kind["enum"]
        return f"sch.DataType({type_name(name, schema)})"
    array = kind.endswith("[]")
    base = kind[:-2] if array else kind
    result = f"sch.DataType({literal(SCHEMA_TYPES.get(base, base))})"
    return result + ".array()" if array else result


def model_kind_source(kind):
    if isinstance(kind, dict):
        schema, name = kind["enum"]
        return type_name(name, schema), False
    array = kind.endswith("[]")
    base = kind[:-2] if array else kind
    return literal(MODEL_TEMPORAL.get(base, base)), array


# [spec:pgorm:req:generative.replay]
class Emitter:
    """Render one validated program as reproducer source, node by node."""

    def __init__(self, data, digest):
        self.data = data
        self.digest = digest
        self.nodes = {node["id"]: node for node in data["nodes"]}
        self.order = [node["id"] for node in data["nodes"]]
        self.owned = {item["id"]: [] for item in data["binders"]}
        for identity in self.order:
            scope = self.nodes[identity]["scope"]
            if scope != "root":
                self.owned[scope].append(identity)
        self.types = {}
        for node in data["nodes"]:
            operation = catalog.OPERATIONS[node["op"]]
            self.types[node["id"]] = (
                self.types[node["inputs"]["query"]]
                if operation.output == "same"
                else operation.output
            )
        self.results = {
            node["data"]["step"]
            for node in data["nodes"]
            if node["op"] in ("result.value", "entity.result")
        }
        self.emitted = set()
        self.descriptors = False

    def var(self, reference):
        return "n_" + reference

    def variables(self, references):
        return [self.var(reference) for reference in references]

    def connection(self, scope):
        return "connection" if scope == "root" else "tx_" + scope

    def required(self, references):
        """Root-scope nodes this effect newly needs, in program order."""
        pending, seen = list(references), set()
        while pending:
            reference = pending.pop()
            if reference in seen:
                continue
            seen.add(reference)
            pending.extend(parameters.input_ids(self.nodes[reference]["inputs"]))
        return [
            identity
            for identity in self.order
            if identity in seen
            and identity not in self.emitted
            and self.nodes[identity]["scope"] == "root"
        ]

    def static_table(self, node):
        """A table's identity, when it does not depend on a fetched row value."""
        schema = node["data"].get("schema")
        if "name" in node["data"]:
            return schema, node["data"]["name"]
        name = self.nodes[node["inputs"]["name"]]
        value = self.nodes[name["inputs"]["value"]]
        if value["op"] != "value":
            return None
        snapshot = value["data"]["value"]
        if snapshot["sql_null"] or snapshot["type"]["kind"] != "text":
            return None
        return schema, snapshot["data"]

    def model_source(self, node):
        table = self.var(node["inputs"]["table"])
        fields = node["data"].get("fields")
        identity = self.static_table(self.nodes[node["inputs"]["table"]])
        definition = next(
            (
                item
                for item in self.data["fixture"]["tables"]
                if identity is not None and (item["schema"], item["name"]) == identity
            ),
            None,
        )
        columns = (
            {column["name"]: column for column in definition["columns"]}
            if definition is not None
            else {}
        )
        mapping = fields if fields is not None else {name: name for name in columns}
        if definition is None or not set(mapping.values()) <= set(columns):
            # The table identity only exists at runtime; declare it there too.
            self.descriptors = True
            return f"_descriptor({table}, {literal(fields)})"
        declarations = []
        for field, physical in mapping.items():
            column = columns[physical]
            kind, array = model_kind_source(column["kind"])
            declarations.append(
                f"{literal(field)}: Column({kind}, name={literal(physical)}, "
                f"array={literal(array)}, nullable={literal(column['nullable'])}, "
                f"primary_key={literal(column['primary'])})"
            )
        return f"Model({table}, " + "{" + ", ".join(declarations) + "})"

    def expression(self, node, binder):
        name, d, i = node["op"], node["data"], node["inputs"]
        var = self.var
        match name:
            case "value":
                return value_source(d["value"])
            case "name":
                return f"p.Identifier({var(i['value'])}.value)"
            case "result.value":
                return (
                    f"_result_value(r_{d['step']}, {literal(d['row'])}, "
                    f"{literal(d['column'])}, {literal(d['type'])})"
                )
            case "entity.result":
                arguments = ["r_" + d["step"], literal(d["row"])]
                if "source" in d:
                    arguments.append(literal(d["source"]))
                return "_entity_result(" + ", ".join(arguments) + ")"
            case "table":
                target = var(i["name"]) if "name" in i else literal(d.get("name"))
                options = [
                    f"{key}={literal(item)}" for key, item in d.items() if key != "name"
                ]
                return "p.Table(" + ", ".join([target] + options) + ")"
            case "expr.column":
                column = var(i["name"]) if "name" in i else literal(d.get("name"))
                if "table" in i:
                    return call(var(i["table"]), "col", [column])
                return f"p.col({column})"
            case "expr.value":
                function = "p.literal" if d["mode"] == "literal" else "p.bind"
                return f"{function}({var(i['value'])})"
            case "expr.binary" | "pipeline.binary":
                left, right = var(i["left"]), var(i["right"])
                if name == "expr.binary" and self.types[i["left"]] == "value":
                    left = f"p.bind({left})"
                if d["operator"] in ARITHMETIC:
                    return f"({left} {ARITHMETIC[d['operator']]} {right})"
                return call(left, d["operator"], [right])
            case "expr.unary" | "pipeline.unary":
                value = var(i["value"])
                if d["operator"] == "not":
                    return f"(~{value})"
                if d["operator"] == "neg":
                    return f"(-{value})"
                return call(value, d["operator"], [])
            case "expr.membership":
                method = "is_not_in" if d["negated"] else "is_in"
                items = ", ".join(self.variables(i["items"]))
                return call(var(i["value"]), method, ["[" + items + "]"])
            case "expr.pattern":
                pattern = var(i["pattern"]) + ".value"
                if d["method"] in LIKE:
                    pattern = f"p.LikeExpr({pattern})"
                    if d.get("escape") is not None:
                        pattern += f".escape({literal(d['escape'])})"
                return call(var(i["value"]), d["method"], [pattern])
            case "expr.call":
                arguments = [literal(d["name"])] + self.variables(i["arguments"])
                return "p.call(" + ", ".join(arguments) + ")"
            case "expr.cast":
                kind = type_name(d["name"], d.get("schema"))
                array = literal(d.get("array", False))
                return call(var(i["value"]), "cast", [kind, f"array={array}"])
            case "expr.alias":
                target = var(i["name"]) if "name" in i else literal(d.get("name"))
                return call(var(i["value"]), "as_", [target])
            case "expr.order":
                nulls = (
                    "None"
                    if d["nulls"] == "default"
                    else "p.Nulls." + d["nulls"].title()
                )
                return call(var(i["value"]), d["direction"], [f"nulls={nulls}"])
            case "condition":
                result = call("p.Condition", d["mode"], self.variables(i["items"]))
                return f"(~{result})" if d.get("negated") else result
            case "select":
                return "p.Select(" + ", ".join(self.variables(i["columns"])) + ")"
            case "select.from":
                return call(var(i["query"]), "from_", [var(i["table"])])
            case "select.filter" | "write.filter":
                return call(var(i["query"]), "where", [var(i["predicate"])])
            case "select.join":
                if d["kind"] == "cross":
                    return call(var(i["query"]), "cross_join", [var(i["table"])])
                if "on" not in i:
                    raise UnsupportedInstruction(
                        "select.join needs an on predicate for a " + d["kind"] + " join"
                    )
                arguments = [
                    var(i["table"]),
                    var(i["on"]),
                    "kind=p.Join." + d["kind"].title(),
                ]
                return call(var(i["query"]), "join", arguments)
            case "select.group":
                return call(var(i["query"]), "group_by", self.variables(i["keys"]))
            case "select.having":
                return call(var(i["query"]), "having", [var(i["predicate"])])
            case "select.order" | "entity.order" | "graph.order":
                return call(var(i["query"]), "order_by", self.variables(i["keys"]))
            case "select.page" | "entity.page":
                query = var(i["query"])
                for method in ("limit", "offset"):
                    if method in d:
                        query = call(query, method, [literal(d[method])])
                if query == var(i["query"]):
                    message = literal("pagination requires a limit or offset")
                    return f"_construction_error({message})"
                return query
            case "select.distinct" | "pipeline.distinct":
                return call(var(i["query"]), "distinct", [])
            case "insert":
                query = f"p.Insert({var(i['table'])})"
                if d["columns"]:
                    query = call(query, "columns", [literal(c) for c in d["columns"]])
                return query
            case "insert.row":
                return call(var(i["query"]), "values", self.variables(i["values"]))
            case "insert.defaults":
                return call(var(i["query"]), "default_values", [])
            case "insert.conflict":
                keys = ", ".join(literal(key) for key in d["keys"])
                target = "p.Conflict.on(" + keys + ")"
                if d["action"] == "nothing":
                    action = call(target, "do_nothing", [])
                elif "columns" in d:
                    action = call(target, "update", [literal(c) for c in d["columns"]])
                else:
                    raise UnsupportedInstruction(
                        "insert.conflict update needs its assigned columns"
                    )
                return call(var(i["query"]), "on_conflict", [action])
            case "update":
                return f"p.Update({var(i['table'])})"
            case "update.set":
                arguments = [literal(d["column"]), var(i["value"])]
                return call(var(i["query"]), "set", arguments)
            case "delete":
                return f"p.Delete({var(i['table'])})"
            case "write.all":
                return call(var(i["query"]), "all_rows", [])
            case "write.returning":
                return call(var(i["query"]), "returning", self.variables(i["columns"]))
            case "raw.template":
                items = ", ".join(self.variables(i["parameters"]))
                return f"p.RawSQL({literal(d['text'])}, [{items}])"
            case "model":
                return self.model_source(node)
            case "model.column":
                column = call(var(i["model"]), "col", [literal(d["name"])])
                return call(column, "expr", [])
            case "model.select":
                query = call(var(i["model"]), "select", [])
                if "predicate" in i:
                    query = call(query, "where", [var(i["predicate"])])
                if i["order"]:
                    query = call(query, "order_by", self.variables(i["order"]))
                return query
            case "model.write":
                if d["method"] == "insert" and "predicate" in i:
                    message = literal("insert cannot carry a predicate")
                    return f"_construction_error({message})"
                if d["method"] == "delete":
                    if d["columns"]:
                        message = literal("delete cannot carry assignments")
                        return f"_construction_error({message})"
                    query = call(var(i["model"]), "delete", [])
                else:
                    items = ", ".join(self.variables(i["values"]))
                    values = (
                        f"dict(zip({literal(d['columns'])}, [{items}], strict=True))"
                    )
                    query = call(var(i["model"]), d["method"], [values])
                if "predicate" in i:
                    query = call(query, "where", [var(i["predicate"])])
                return query
            case "model.returning":
                columns = [literal(column) for column in d["columns"]]
                return call(var(i["query"]), "returning", columns)
            case "entity":
                return f"p.entity({literal(d['name'])})"
            case "entity.column":
                column = call(var(i["entity"]), "col", [literal(d["name"])])
                return call(column, "expr", [])
            case "entity.predicate":
                column = call(var(i["entity"]), "col", [literal(d["column"])])
                return call(column, d["operator"], [var(i["value"])])
            case "entity.find":
                return call(var(i["entity"]), "find", [])
            case "entity.filter" | "graph.filter":
                return call(var(i["query"]), "where", [var(i["predicate"])])
            case "entity.active":
                return call(var(i["entity"]), "active", [])
            case "entity.into_active":
                return call(var(i["model"]), "into_active", [])
            case "active.set":
                if d["state"] == "set":
                    arguments = [literal(d["column"]), var(i["value"])]
                    return call(var(i["model"]), "set", arguments)
                if "value" in i:
                    message = literal("reset and not_set cannot receive a value")
                    return f"_construction_error({message})"
                return call(var(i["model"]), d["state"], [literal(d["column"])])
            case "graph":
                return f"p.graph({literal(d['name'])})"
            case "graph.find":
                return call(
                    var(i["graph"]), "find", [f"aliases={literal(d['aliases'])}"]
                )
            case "graph.column":
                arguments = [literal(d["source"]), literal(d["column"])]
                return call(var(i["query"]), "col", arguments)
            case "graph.cursor":
                return call(var(i["query"]), "cursor", [literal(d["column"])])
            case "cursor.bound":
                method = d["side"] + "_with"
                return call(var(i["cursor"]), method, self.variables(i["values"]))
            case "cursor.page":
                cursor = call(var(i["cursor"]), d["side"], [literal(d["count"])])
                return call(cursor, d["direction"], [])
            case "pipeline.source":
                source = f"pl.source({var(i['source'])})"
                if "alias" in d:
                    source = call(source, "named", [literal(d["alias"])])
                return source
            case "pipeline.from":
                return f"pl.Pipeline({var(i['source'])})"
            case "pipeline.column":
                return f"pl.col({literal(d['source'])}, {literal(d['column'])})"
            case "pipeline.alias":
                return f"pl.alias({literal(d['name'])})"
            case "pipeline.value":
                return f"pl.literal({var(i['value'])})"
            case "pipeline.bind":
                if binder is None:
                    raise UnsupportedInstruction(
                        "pipeline.bind has no owning public callback"
                    )
                return call(binder, "bind", [var(i["value"])])
            case "pipeline.membership":
                items = ", ".join(self.variables(i["items"]))
                return call(var(i["value"]), "in_array", ["[" + items + "]"])
            case "pipeline.function":
                arguments = ", ".join(self.variables(i["arguments"]))
                return f"pl.{d['name']}({arguments})"
            case "pipeline.cast":
                return call(var(i["value"]), "cast", [literal(d["name"])])
            case "pipeline.named":
                return call(var(i["value"]), "as_", [literal(d["name"])])
            case "pipeline.take":
                arguments = [literal(d["start"]), literal(d["end"])]
                return call(var(i["query"]), "take_range", arguments)
            case "pipeline.set":
                return call(var(i["query"]), d["method"], [var(i["source"])])
            case "pipeline.sources":
                selection = f"pl.sources({literal(d['name'])})"
                qualifiers = f"qualifiers={literal(d['qualifiers'])}"
                return call(var(i["query"]), "select_sources", [selection, qualifiers])
            case "schema.create":
                query = f"sch.create_table({var(i['table'])})"
                for column in d["columns"]:
                    kind = datatype_source(column["kind"])
                    definition = f"sch.ColumnDef({literal(column['name'])}, {kind})"
                    nullable = "null" if column["nullable"] else "not_null"
                    definition = call(definition, nullable, [])
                    query = call(query, "column", [definition])
                key = [column["name"] for column in d["columns"] if column["primary"]]
                if key:
                    query = call(query, "primary_key", [literal(key)])
                return query
            case "schema.drop":
                return f"sch.drop_table({var(i['table'])})"
            case "schema.rename":
                if "column" in d:
                    arguments = [literal(d["column"]), literal(d["name"])]
                    return (
                        "sch.rename_column("
                        + ", ".join([var(i["table"])] + arguments)
                        + ")"
                    )
                return f"sch.rename_table({var(i['table'])}, {literal(d['name'])})"
            case "schema.index":
                if not d["columns"]:
                    raise UnsupportedInstruction("schema.index needs a first column")
                first, *rest = d["columns"]
                query = (
                    f"sch.create_index({var(i['table'])}, {literal(first)}, "
                    f"name={literal(d['name'])})"
                )
                for column in rest:
                    query = call(query, "column", [literal(column)])
                return call(query, "unique", []) if d["unique"] else query
            case "schema.enum":
                kind = type_name(d["name"], d["schema"])
                return f"sch.create_type({kind}).values({literal(d['labels'])})"
            case "schema.enum_change":
                kind = type_name(d["name"], d["schema"])
                if d["method"] == "drop":
                    return f"sch.drop_type({kind})"
                if "value" not in d:
                    raise UnsupportedInstruction(
                        "schema.enum_change " + d["method"] + " needs a value"
                    )
                if d["method"] == "add":
                    return f"sch.alter_type({kind}).add_value({literal(d['value'])})"
                if "new_value" not in d:
                    raise UnsupportedInstruction(
                        "schema.enum_change rename needs its replacement value"
                    )
                arguments = [literal(d["value"]), literal(d["new_value"])]
                return (
                    f"sch.alter_type({kind}).rename_value(" + ", ".join(arguments) + ")"
                )
        raise UnsupportedInstruction("no public Python source for " + name)

    def stage(self, node, indent, lines):
        """Emit a pipeline stage with the callback owning its binder scope."""
        name, d, i = node["op"], node["data"], node["inputs"]
        method = name.removeprefix("pipeline.")
        references = i[next(iter(CALLBACK_INPUTS[name]))]
        query = self.var(i["query"])
        bound = "binder" in d
        if bound:
            callback = "_cb_" + node["id"]
            lines.append(f"{indent}def {callback}(binder):")
            for identity in self.owned[d["binder"]]:
                self.node(self.nodes[identity], indent + "    ", lines, "binder")
            returned = (
                "[" + ", ".join(self.variables(references)) + "]"
                if isinstance(references, list)
                else self.var(references)
            )
            lines.append(f"{indent}    return {returned}")
            lines.append("")
            arguments = [callback]
        else:
            arguments = self.variables(
                references if isinstance(references, list) else [references]
            )
        suffix = "_with" if bound else ""
        if method == "window":
            over = "w_" + node["id"]
            builder = call("pl.Over()", "by", self.variables(i["partition"]))
            builder = call(builder, "sort_by", self.variables(i["order"]))
            if "start" in d or "end" in d:
                rows = [literal(d.get("start")), literal(d.get("end"))]
                builder = call(builder, "rows", rows)
            lines.append(f"{indent}{over} = {builder}")
            result = (
                call(query, "window_with", [over] + arguments)
                if bound
                else call(query, "window", arguments + ["over=" + over])
            )
        elif method == "join":
            source = [self.var(i["source"])]
            kind = ["kind=p.Join." + d["kind"].title()]
            result = call(query, "join" + suffix, source + arguments + kind)
        else:
            result = call(query, method + suffix, arguments)
        lines.append(f"{indent}{self.var(node['id'])} = {result}")

    def node(self, node, indent, lines, binder=None):
        self.emitted.add(node["id"])
        if node["op"] in CALLBACK_INPUTS:
            self.stage(node, indent, lines)
            return
        variable = self.var(node["id"])
        lines.append(f"{indent}{variable} = {self.expression(node, binder)}")
        if node["op"] == "raw.template":
            lines.append(f"{indent}{variable}.inline_sql()")

    def effect(self, step, indent, lines):
        name, d, i = step["op"], step["data"], step["inputs"]
        connection = self.connection(step["scope"])
        query = self.var(i["query"]) if "query" in i else None
        match name:
            case "fetch":
                result = f"await _fetch({query}, {literal(d['mode'])}, {connection})"
            case "execute":
                result = f"await _execute({query}, {connection})"
            case "active.write":
                model = self.var(i["model"])
                result = f"await _active({model}, {literal(d['method'])}, {connection})"
            case "entity.change" | "entity.changes" | "entity.upsert":
                inputs = ", ".join(
                    f"{literal(key)}: "
                    + (
                        "[" + ", ".join(self.var(item) for item in value) + "]"
                        if isinstance(value, list)
                        else self.var(value)
                    )
                    for key, value in i.items()
                )
                result = (
                    f"await _versions({literal(name)}, {{{inputs}}}, "
                    f"{literal(d)}, {connection})"
                )
            case "stream":
                result = (
                    f"await _stream({query}, {literal(d['take'])}, "
                    f"{literal(d['cancel'])}, {connection})"
                )
            case "inspect":
                result = f"_inspect({query})"
            case _:
                raise UnsupportedInstruction("no public Python source for " + name)
        lines.append(f"{indent}_observation, _paths, _rows = {result}")
        if step["id"] in self.results:
            lines.append(f"{indent}r_{step['id']} = _rows")
        lines.append(indent + OBSERVED)

    def observe(self, paths, observation, indent, lines):
        lines.append(f"{indent}_paths = {literal(paths)}")
        lines.append(f"{indent}_observation = {literal(observation)}")
        lines.append(indent + OBSERVED)

    def scope(self, step, remaining, indent, lines):
        """A transaction scope: its steps run inside the ``async with`` block
        that opens it, and the step that closes it is how the block exits,
        normally to commit and by raising to roll back. How many of
        ``remaining`` the scope consumed."""
        body, inner = indent + "    ", indent + "        "
        d = step["data"]
        connection = self.connection(step["scope"])
        if step["scope"] == "root":
            isolation = (
                "None" if d["isolation"] == "default" else literal(d["isolation"])
            )
            mode = f"mode={literal(d['mode'])}"
            opened = call(connection, "transaction", [mode, f"isolation={isolation}"])
            path = "pgorm::DatabaseConnection::transaction_with"
        else:
            opened = call(connection, "transaction", [])
            path = "pgorm::TransactionTrait::transaction"
        lines.append(indent + "try:")
        for identity in self.required(parameters.input_ids(step["inputs"])):
            self.node(self.nodes[identity], body, lines)
        lines.append(f"{body}async with {opened} as {self.connection(d['child'])}:")
        self.observe([path], {"kind": "transaction", "scope": d["child"]}, inner, lines)
        consumed, closing = self.steps(remaining, inner, lines, True)
        closed = (["pgorm::TransactionTrait::transaction"], {"kind": "unit"})
        if closing is not None and closing["op"] == "commit":
            self.observe(*closed, body, lines)
        else:
            lines.append(inner + "raise _RolledBack")
            lines.append(indent + "except _RolledBack:")
            if closing is None:
                lines.append(body + "pass")
            else:
                self.observe(*closed, body, lines)
        lines.append(indent + "except p.PgormError as error:")
        lines.append(body + '_step.update(status="error", observation=_error(error))')
        return consumed

    def steps(self, remaining, indent, lines, scoped):
        """Emit steps until the scope the caller opened closes: how many it
        consumed, and the commit or rollback that closed it."""
        body = indent + "    "
        consumed = 0
        while consumed < len(remaining):
            step = remaining[consumed]
            consumed += 1
            record = {
                "id": step["id"],
                "operation": step["op"],
                "status": "attempted",
                "native_paths": [],
            }
            lines.append("")
            lines.append(f"{indent}_step = {literal(record)}")
            lines.append(indent + 'report["steps"].append(_step)')
            if step["op"] in ("commit", "rollback"):
                return consumed, step
            if step["op"] == "begin":
                consumed += self.scope(step, remaining[consumed:], indent, lines)
            else:
                lines.append(indent + "try:")
                for identity in self.required(parameters.input_ids(step["inputs"])):
                    self.node(self.nodes[identity], body, lines)
                self.effect(step, body, lines)
                lines.append(indent + "except p.PgormError as error:")
                lines.append(
                    body + '_step.update(status="error", observation=_error(error))'
                )
            lines.append(
                f"{indent}connection = await _reacquire(connection, pool, {scoped})"
            )
        return consumed, None

    def program(self):
        lines = [
            "async def _run(pool, report):",
            "    connection = await pool.acquire()",
            "    try:",
            "        await _apply_fixture(connection)",
        ]
        self.steps(self.data["steps"], "        ", lines, False)
        lines.append(CLEANUP)
        return "\n".join(lines)

    def constants(self):
        lines = [
            f"PROGRAM_SHA256 = {literal(self.digest)}",
            f"STEP_COUNT = {literal(len(self.data['steps']))}",
            "FIXTURE_STATEMENTS = [",
        ]
        for statement in statements(baseline.render(self.data["fixture"])):
            lines.append(f"    {literal(statement)},")
        lines.append("]")
        return "\n".join(lines)

    def tables(self):
        lines = ["FIXTURE_TABLES = ["]
        for table in self.data["fixture"]["tables"]:
            declaration = {
                "schema": table["schema"],
                "name": table["name"],
                "columns": table["columns"],
            }
            lines.append(f"    {literal(declaration)},")
        lines.append("]")
        return "\n".join(lines)

    def render(self):
        # The program is rendered first: it decides whether runtime model
        # metadata has to travel with the script.
        program = self.program()
        parts = [HEADER.format(digest=self.digest), PRELUDE, self.constants()]
        if self.descriptors:
            parts.extend([self.tables(), DESCRIPTOR])
        parts.extend([program, MAIN])
        return "\n\n\n".join(parts) + "\n"


# [spec:pgorm:req:generative.replay]
def render(program):
    """Emit a runnable Python reproducer for a validated Program."""
    if not isinstance(program, Program):
        program = (
            Program(program)
            if isinstance(program, (str, bytes))
            else Program.from_dict(program)
        )
    return Emitter(program.data(), program.digest).render()
