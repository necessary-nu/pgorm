"""Family variations attributed from constructed instructions, never from labels.

matrix.json declares coverage families, each carrying the variations a full
profile must reach. A variation is finer than the instruction it rides on:
`insert.conflict` is one operation, but `crud.conflict-nothing` and
`crud.conflict-update` are two separate obligations, and only one of them is
discharged by any given program.

Attribution keeps the discipline the rest of the coverage module keeps. A cell
is discharged by an instruction the subject actually built through a native
path, or by an effect it actually dispatched. What the cell is read off is the
declared instruction -- its operation, the catalog-typed fields in its `data`,
the shape of its inputs -- or the effect's own observation. Never a label the
generator attached, and never a family the plan merely scheduled.

A variation this module cannot attribute is left dark on purpose: a token that
fires without evidence is worse than an outstanding obligation, because it
retires the cell while leaving it untested.
"""

import math
import struct

from .wire import INTEGER_BITS

# Identifiers PostgreSQL would fold or reject unquoted. A generated name drawn
# from the hostile corpus lands here; a plain lowercase ASCII one does not.
UNQUOTED = frozenset("abcdefghijklmnopqrstuvwxyz0123456789_")

# Chains that keep a write addressable while wrapping it, so a guard applied to
# `update -> update.set -> write.filter` still attributes to the update.
WRITE_CHAIN = ("update.set", "write.filter", "write.returning", "write.all")

TEMPORAL = frozenset({"date", "time", "datetime", "datetime_utc"})

# What the registered Account's native `before_save` appends to the name an
# insert sets (bridge/src/account.rs). Note keeps the default behaviour, so a
# write whose stored name differs from its declared one by exactly this is the
# hook's own work, visible in the effect's observation.
HOOK_SUFFIX = "|hook"


def quoted(name):
    """Whether this identifier can only be spelled with quotes."""
    return not name or any(character not in UNQUOTED for character in name)


class Evidence:
    """Constructed instructions and dispatched effects, indexed for attribution."""

    def __init__(self, program, built, steps):
        self.fixture = program.get("fixture") or {"tables": []}
        self.nodes = {node["id"]: node for node in program["nodes"]}
        self.steps = steps
        self.built = built
        self.by_op = {}
        self.consumers = {}
        for identity in built:
            node = self.nodes.get(identity)
            if node is None:
                continue
            self.by_op.setdefault(node["op"], []).append(node)
            for reference in _references(node):
                self.consumers.setdefault(reference, []).append(node)

    def each(self, op):
        return self.by_op.get(op, ())

    def present(self, *ops):
        return any(self.by_op.get(op) for op in ops)

    def refs(self, node, key):
        value = node["inputs"].get(key)
        if value is None:
            return []
        return list(value) if isinstance(value, list) else [value]

    def input(self, node, key):
        references = self.refs(node, key)
        return self.nodes.get(references[0]) if references else None

    def payload(self, reference):
        """The tagged value a `value` instruction carries, if that is what it is."""
        node = self.nodes.get(reference)
        if node is None or node["op"] != "value":
            return None
        return node["data"]["value"]

    def payloads(self, node, key):
        found = (self.payload(reference) for reference in self.refs(node, key))
        return [value for value in found if value is not None]

    def write_root(self, node):
        """Follow a write's wrapper chain down to the insert/update/delete."""
        seen = set()
        while node is not None and node["op"] in WRITE_CHAIN:
            if node["id"] in seen:
                return None
            seen.add(node["id"])
            node = self.input(node, "query")
        return None if node is None else node["op"]

    def reaches(self, node, ops):
        """Whether any constructed consumer above this node has one of `ops`."""
        seen, stack = {node["id"]}, [node]
        while stack:
            for consumer in self.consumers.get(stack.pop()["id"], ()):
                if consumer["op"] in ops:
                    return True
                if consumer["id"] not in seen:
                    seen.add(consumer["id"])
                    stack.append(consumer)
        return False

    def table_of(self, node):
        """The fixture table declaration a `table` input names, if it is declared."""
        table = self.input(node, "table")
        if table is None or table["op"] != "table":
            return None
        schema, name = table["data"].get("schema"), table["data"].get("name")
        for item in self.fixture.get("tables", ()):
            if item["schema"] == schema and item["name"] == name:
                return item
        return None

    def dispatched(self, op):
        return [step for step in self.steps if step["op"] == op]


def _references(node):
    for value in node["inputs"].values():
        if isinstance(value, list):
            yield from value
        else:
            yield value


def _float(value):
    kind, data = value["type"]["kind"], value["data"]
    if kind not in ("f32", "f64") or value["sql_null"]:
        return None
    layout = ">f" if kind == "f32" else ">d"
    return struct.unpack(layout, bytes.fromhex(data))[0]


def _json_null(data):
    if data is None:
        return True
    if isinstance(data, dict):
        return any(_json_null(item) for item in data.values())
    return isinstance(data, list) and any(_json_null(item) for item in data)


def _crud(e):
    tokens = set()
    if e.present("select"):
        tokens.add("select")
    for node in e.each("insert.row"):
        tokens.add("insert-values" if e.refs(node, "values") else "empty-batch")
    for node in e.each("insert"):
        # An insert dispatched without rows or explicit defaults writes nothing.
        if not e.reaches(node, {"insert.row", "insert.defaults"}):
            tokens.add("empty-batch")
        declared = e.table_of(node)
        columns = node["data"].get("columns") or []
        # An omitted write needs a row to omit a column from: an empty batch
        # names columns too, but writes nothing for any of them.
        if (
            declared is not None
            and columns
            and e.reaches(node, {"insert.row"})
            and {item["name"] for item in declared["columns"]} - set(columns)
        ):
            tokens.add("omitted-write")
    if e.present("update.set"):
        tokens.add("update-values")
    for node in e.each("write.filter"):
        root = e.write_root(node)
        if root in ("update", "delete"):
            tokens.add(root + "-guard")
    for node in e.each("insert.conflict"):
        tokens.add("conflict-" + node["data"]["action"])
    if e.present("write.returning"):
        tokens.add("returning")
    if e.present("insert.defaults"):
        tokens.add("default-row")
    for op, key in (("update.set", "value"), ("insert.row", "values")):
        for node in e.each(op):
            for value in e.payloads(node, key):
                tokens.add("null-write" if value["sql_null"] else "set-write")
    if e.present("select.join"):
        tokens.add("join")
    if e.present("select.group") and e.present("select.having"):
        tokens.add("group-having")
    for step in e.dispatched("fetch") + e.dispatched("stream"):
        ordered = step["data"].get("ordered")
        if ordered is True:
            tokens.add("ordered")
        elif ordered is False:
            tokens.add("unordered")
    return tokens


def _conditions(e):
    tokens = set()
    for node in e.each("condition"):
        items = e.refs(node, "items")
        mode = node["data"]["mode"]
        if node["data"].get("negated"):
            tokens.add("not")
        if len(items) > 1:
            tokens.add("predicate-grouping")
        if not items and mode == "any":
            tokens.add("empty-disjunction")
        nested = any(
            (e.nodes.get(item) or {}).get("op") == "condition" for item in items
        )
        if nested:
            tokens.add("nested-and" if mode == "all" else "nested-or")
        if _tenant_guard(e, node):
            tokens.add("tenant-guard")
    for node in e.each("expr.membership"):
        if not e.refs(node, "items"):
            tokens.add("empty-membership")
        if node["data"].get("negated"):
            tokens.add("not")
    for node in e.each("expr.unary"):
        operator = node["data"]["operator"]
        tokens.add("not" if operator == "not" else "null")
    for node in e.each("expr.binary"):
        # A conjunction only evidences nesting when an operand is itself a
        # boolean combinator; two column comparisons joined by AND are not it.
        operator = node["data"]["operator"]
        if operator in ("and", "or") and any(
            _combinator(e.input(node, key)) for key in ("left", "right")
        ):
            tokens.add("nested-and" if operator == "and" else "nested-or")
    # A NULL reaching a predicate is the cell; a NULL written into a row is not.
    for op, key in (("expr.membership", "items"), ("expr.binary", "right")):
        for node in e.each(op):
            if any(value["sql_null"] for value in e.payloads(node, key)):
                tokens.add("null")
    return tokens


def _combinator(node):
    if node is None:
        return False
    if node["op"] == "condition":
        return True
    if node["op"] == "expr.unary":
        return node["data"]["operator"] == "not"
    return node["op"] == "expr.binary" and node["data"]["operator"] in ("and", "or")


def _tenant_guard(e, node):
    """A predicate group that pins the tenant column is the guard shape."""
    for item in e.refs(node, "items"):
        child = e.nodes.get(item)
        if child is None or child["op"] != "expr.binary":
            continue
        if child["data"]["operator"] != "eq":
            continue
        left = e.input(child, "left")
        if (
            left is not None
            and left["op"] == "expr.column"
            and left["data"].get("name") == "tenant"
        ):
            return True
    return False


def _models(e):
    tokens = set()
    for node in e.each("model"):
        tokens.add("python-descriptor")
        mapping = node["data"].get("fields") or {}
        if any(logical != physical for logical, physical in mapping.items()):
            tokens.add("mapped-column")
        declared = e.table_of(node)
        if declared is not None:
            keys = [item for item in declared["columns"] if item.get("primary")]
            if len(keys) > 1:
                tokens.add("composite-key")
    for node in e.each("model.write"):
        tokens.add("crud")
        columns = node["data"].get("columns") or []
        model = e.input(node, "model")
        mapping = (model or {}).get("data", {}).get("fields") or {}
        if mapping and set(mapping) - set(columns):
            tokens.add("omitted-field")
        for value in e.payloads(node, "values"):
            if value["sql_null"]:
                tokens.add("null-field")
    if e.present("model.returning", "model.select"):
        tokens.add("typed-record")
    return tokens


def _entities(e):
    tokens = set()
    for node in e.each("entity"):
        name = node["data"]["name"].rsplit(".", 1)[-1].lower()
        if name in ("account", "note"):
            tokens.add(name)
    for node in e.each("active.set"):
        state = node["data"]["state"]
        tokens.add({"set": "set", "reset": "unchanged", "not_set": "not-set"}[state])
    for step in e.dispatched("active.write"):
        tokens.add("active-" + step["data"]["method"])
        if _hooked(e, step):
            tokens.add("hooks")
        if _written_entity(e, step["inputs"]["model"]) == "campaign.Room":
            tokens.add("temporal-key")
    if _deferred(e):
        tokens.add("deferred-key")
    if e.dispatched("entity.change"):
        tokens.add("update-change")
    if e.dispatched("entity.changes"):
        tokens.add("update-changes")
    for step in e.dispatched("entity.upsert"):
        # Which an upsert did is read off the row as it stood before the
        # write, as the terminal decides it: absent for a row it inserted.
        for row in _rows(step.get("observation")):
            items = row.get("items") or []
            if row.get("kind") == "tuple" and len(items) == 2:
                tokens.add(
                    "upsert-inserted"
                    if items[0].get("kind") == "absent"
                    else "upsert-updated"
                )
    for node in e.each("entity.predicate"):
        value = e.payload(e.refs(node, "value")[0]) if e.refs(node, "value") else None
        if value is not None and value["type"]["kind"] == "enum":
            tokens.add("typed-enum-predicate")
    return tokens


def _written_entity(e, reference):
    """The registration an active model chain was begun from, if one is named.

    A chain begins at `entity.active`, which names its entity, or at
    `entity.into_active` over an earlier step's model: an entity read or
    another active write, followed back the same way.
    """
    seen = set()
    while reference is not None and reference not in seen:
        seen.add(reference)
        node = e.nodes.get(reference)
        if node is None:
            return None
        if node["op"] == "active.set":
            reference = node["inputs"].get("model")
        elif node["op"] in ("entity.active", "entity.find"):
            entity = e.input(node, "entity")
            return None if entity is None else entity["data"].get("name")
        elif node["op"] in ("entity.filter", "entity.order", "entity.page"):
            reference = node["inputs"].get("query")
        elif node["op"] == "entity.into_active":
            result = e.input(node, "model")
            step = next(
                (
                    item
                    for item in e.steps
                    if item["id"] == (result or {}).get("data", {}).get("step")
                ),
                None,
            )
            if step is None:
                return None
            reference = step["inputs"].get(
                "model" if step["op"] == "active.write" else "query"
            )
        else:
            return None
    return None


def _deferred(e):
    """A stay inserted before the room version it references, in one
    transaction that then commits: the temporal key's check waited."""
    pending = {}
    for step in e.steps:
        scope = step.get("scope")
        if step["op"] == "active.write" and step["data"]["method"] == "insert":
            entity = _written_entity(e, step["inputs"]["model"])
            if entity == "campaign.Stay" and scope != "root":
                pending.setdefault(scope, "stay")
            elif entity == "campaign.Room" and pending.get(scope) == "stay":
                pending[scope] = "room"
        elif (
            step["op"] == "commit"
            and pending.get(scope) == "room"
            and (step.get("observation") or {}).get("kind") != "error"
        ):
            return True
    return False


def _declared_name(e, reference):
    """The `name` an active-model chain last set, when it set a text value."""
    node = e.nodes.get(reference)
    while node is not None and node["op"] == "active.set":
        if node["data"]["column"] == "name":
            value = e.payload(node["inputs"].get("value"))
            if node["data"]["state"] != "set" or value is None or value["sql_null"]:
                return None
            return value["data"]
        node = e.input(node, "model")
    return None


def _hooked(e, step):
    """Whether an insert's own observation carries the behaviour hook's rewrite.

    The declared name alone proves nothing and neither does the stored one;
    the stored name being the declared name plus the hook's suffix is what an
    active write could only produce by running `before_save`.
    """
    if step["data"]["method"] != "insert":
        return False
    declared = _declared_name(e, step["inputs"]["model"])
    if declared is None:
        return False
    for row in _rows(step.get("observation")):
        for field in row.get("fields") or ():
            value = field.get("value") or {}
            if (
                field.get("name") == "name"
                and value.get("data") == declared + HOOK_SUFFIX
            ):
                return True
    return False


def _graph(e):
    tokens = set()
    for node in e.each("graph"):
        name = node["data"]["name"].rsplit(".", 1)[-1]
        if name == "RequiredNotes":
            tokens.add("required-join")
        elif name == "OptionalNotes":
            tokens.add("optional-join")
        elif name == "SelfJoin":
            tokens.add("self-join")
        elif name == "StayRooms":
            tokens.add("period-relation")
        elif name == "StayGuests":
            tokens.add("not-enforced-relation")
    for node in e.each("graph.find"):
        aliases = node["data"].get("aliases") or []
        arity = len(aliases) + 1
        if 1 <= arity <= 7:
            tokens.add("arity-" + str(arity))
        if any(quoted(alias) for alias in aliases):
            tokens.add("hostile-alias")
    for node in e.each("graph.cursor"):
        # A non-key sort column is the duplicate-prone one; the cursor has to
        # complete it with the source's own primary key to stay total.
        if node["data"]["column"] != "id":
            tokens.add("duplicate-sort-key")
    for node in e.each("cursor.bound"):
        tokens.add("cursor-" + node["data"]["side"])
    for node in e.each("cursor.page"):
        tokens.add("cursor-" + node["data"]["side"])
    tokens |= _graph_rows(e)
    return tokens


def _graph_rows(e):
    """Decoded graph rows: a record proves decode, an absent slot proves the gap."""
    tokens = set()
    for step in e.steps:
        if step["op"] not in ("fetch", "stream"):
            continue
        query = e.nodes.get(next(iter(e.refs(step, "query")), None))
        if query is None or not query["op"].startswith(("graph.", "cursor.")):
            continue
        for row in _rows(step.get("observation")):
            if row.get("kind") == "tuple":
                items = row.get("items") or []
                if any(item.get("kind") == "record" for item in items):
                    tokens.add("model-decode")
                if any(item.get("kind") == "absent" for item in items):
                    tokens.add("absent-source")
            elif row.get("kind") == "record":
                tokens.add("model-decode")
    return tokens


def _rows(observation):
    if isinstance(observation, list):
        return [item for item in observation if isinstance(item, dict)]
    if isinstance(observation, dict):
        rows = observation.get("rows")
        if isinstance(rows, list):
            return [item for item in rows if isinstance(item, dict)]
        return [observation]
    return []


def _pipeline(e):
    tokens = set()
    for op, token in (
        ("pipeline.value", "literal"),
        ("pipeline.bind", "bound"),
        ("pipeline.select", "projection"),
        ("pipeline.derive", "derive"),
        ("pipeline.window", "window"),
        ("pipeline.join", "join"),
        ("pipeline.sources", "select-sources"),
    ):
        if e.present(op):
            tokens.add(token)
    if e.present("pipeline.group") and e.present("pipeline.aggregate"):
        tokens.add("group-having")
    for node in e.each("pipeline.set"):
        tokens.add(node["data"]["method"])
    for node in e.each("pipeline.join"):
        kind = node["data"]["kind"]
        if kind in ("right", "full"):
            tokens.add(kind + "-join")
    for node in e.each("pipeline.source"):
        source = e.input(node, "source")
        if source is not None and source["op"].startswith("pipeline."):
            tokens.add("nested-source")
    for node in e.each("pipeline.sources"):
        arity = len(node["data"].get("qualifiers") or [])
        if 1 <= arity <= 6:
            tokens.add("source-arity-" + str(arity))
    return tokens


def _names(e):
    tokens = set()
    for node in e.each("table"):
        if node["data"].get("schema"):
            tokens.add("schema")
        if node["data"].get("name") or e.refs(node, "name"):
            tokens.add("table")
        if node["data"].get("alias"):
            tokens.add("alias")
    if e.present("expr.column"):
        tokens.add("column")
    if e.present("expr.alias", "pipeline.named", "pipeline.alias"):
        tokens.add("alias")
    if e.present("expr.order", "pipeline.sort"):
        tokens.add("order")
    if e.present("select.group", "pipeline.group"):
        tokens.add("group")
    if e.present("expr.call", "pipeline.function"):
        tokens.add("function")
    for node in e.each("expr.cast"):
        tokens.add("type")
        schema = node["data"].get("schema")
        if schema and schema != "pg_catalog":
            tokens.add("qualified-enum")
    if e.present("pipeline.cast"):
        tokens.add("type")
    for value in _all_payloads(e):
        tag = value["type"]
        tag = tag.get("element", tag) if tag["kind"] == "array" else tag
        if tag["kind"] == "enum" and tag.get("schema"):
            tokens.add("qualified-enum")
    return tokens


def _patterns(e):
    tokens = set()
    for node in e.each("raw.template"):
        tokens.add("raw-template")
        text = node["data"]["text"]
        if "--" in text or "/*" in text:
            tokens.add("template-comment")
        if "'" in text:
            tokens.add("template-quote")
        if "$" in text and _dollar_quoted(text):
            tokens.add("template-dollar-string")
        if _repeated_slot(text):
            tokens.add("repeated-slot")
    for node in e.each("expr.pattern"):
        method = node["data"]["method"]
        if method == "contains_text":
            tokens.add("contains")
        elif method == "starts_with":
            tokens.add("starts-with")
        elif method == "ends_with":
            tokens.add("ends-with")
        else:
            tokens.add("explicit-like")
    return tokens


def _dollar_quoted(text):
    """A `$tag$ ... $tag$` body, as distinct from a bare `$1` placeholder."""
    index = text.find("$")
    while index != -1:
        end = text.find("$", index + 1)
        if end != -1 and text[index + 1 : end].isalpha() is not False:
            tag = text[index : end + 1]
            if (tag[1:-1].isidentifier() or tag == "$$") and text.find(
                tag, end + 1
            ) != -1:
                return True
        index = text.find("$", index + 1)
    return False


def _repeated_slot(text):
    slots = [
        text[index + 1]
        for index, character in enumerate(text[:-1])
        if character == "$" and text[index + 1].isdigit()
    ]
    return len(slots) != len(set(slots))


def _all_payloads(e):
    for node in e.each("value"):
        yield node["data"]["value"]


def _types(e):
    tokens = set()
    for value in _all_payloads(e):
        tokens |= _type_tokens(value)
    return tokens


def _type_tokens(value):
    tokens = set()
    tag = value["type"]
    kind = tag["kind"]
    if kind == "array":
        element = tag["element"]["kind"]
        if value["sql_null"]:
            tokens.add("array-null")
        elif not value["data"]:
            tokens.add("array-empty")
        elif any(item["sql_null"] for item in value["data"]):
            tokens.add("array-element-null")
        if element == "enum":
            tokens.add("enum-array")
        for item in value["data"] or ():
            tokens |= _type_tokens(item)
        return tokens
    if value["sql_null"]:
        tokens.add("sql-null")
        return tokens
    if kind in INTEGER_BITS:
        bits, signed = INTEGER_BITS[kind]
        low = -(2 ** (bits - 1)) if signed else 0
        high = 2 ** (bits - int(signed)) - 1
        if int(value["data"]) in (low, high):
            tokens.add("integer-boundary")
    elif kind in ("f32", "f64"):
        number = _float(value)
        if math.isnan(number) or math.isinf(number):
            tokens.add("float-special")
        elif number == 0.0 and math.copysign(1.0, number) < 0:
            tokens.add("signed-zero")
    elif kind == "json":
        tokens.add("json")
        if _json_null(value["data"]):
            tokens.add("json-null")
    elif kind in ("decimal", "uuid", "enum", "interval"):
        tokens.add(kind)
    elif kind in TEMPORAL:
        tokens.add("temporal")
    return tokens


def _schema(e):
    tokens = set()
    names = []
    for node in e.each("schema.create"):
        tokens.add("create")
        names.extend(item["name"] for item in node["data"].get("columns") or [])
    if e.present("schema.drop"):
        tokens.add("drop")
    for node in e.each("schema.rename"):
        tokens.add("rename-column" if node["data"].get("column") else "rename-table")
        names.append(node["data"]["name"])
    for node in e.each("schema.index"):
        tokens.add("index")
        names.append(node["data"]["name"])
    for node in e.each("schema.enum"):
        tokens.add("enum-create")
        names.append(node["data"]["name"])
    for node in e.each("schema.enum_change"):
        tokens.add("enum-" + node["data"]["method"])
        names.append(node["data"]["name"])
    if tokens:
        for node in e.each("table"):
            names.extend(
                value
                for key in ("name", "schema", "alias")
                if (value := node["data"].get(key)) is not None
            )
        if any(quoted(name) for name in names):
            tokens.add("quoted-names")
    return tokens


def _sequences(e):
    tokens = set()
    for node in e.each("result.value"):
        tokens.add("store-read-value")
    for node in e.each("name"):
        source = e.input(node, "value")
        if source is not None and source["op"] == "result.value":
            tokens.add("store-read-identifier")
    for step in e.steps:
        if step["op"] in ("commit", "rollback"):
            tokens.add(step["op"])
        elif step["op"] == "begin":
            if step["scope"] != "root":
                tokens.add("savepoint")
            if step["data"].get("mode") == "read_only":
                tokens.add("read-only")
    reused = {}
    for step in e.steps:
        for reference in e.refs(step, "query"):
            reused[reference] = reused.get(reference, 0) + 1
    if any(count > 1 for count in reused.values()):
        tokens.add("repeated-execution")
    for step in e.dispatched("stream"):
        # The terminal reports how the stream actually ended; the declared
        # `take`/`cancel` are only the request.
        observation = step.get("observation")
        state = (
            (observation or {}).get("stream") if isinstance(observation, dict) else None
        )
        if not isinstance(state, dict):
            continue
        if state.get("cancelled"):
            tokens.add("stream-cancel")
        elif state.get("complete"):
            tokens.add("stream-complete")
        elif state.get("closed"):
            tokens.add("stream-early-close")
    return tokens


FAMILIES = {
    "crud": _crud,
    "conditions": _conditions,
    "models": _models,
    "entities": _entities,
    "graph": _graph,
    "pipeline": _pipeline,
    "names": _names,
    "patterns": _patterns,
    "types": _types,
    "schema": _schema,
    "sequences": _sequences,
}


# [spec:pgorm:req:generative.matrix]
# [spec:pgorm:req:generative.verdict]
def observed(program, built, steps):
    """Family/variation tokens evidenced by one program's construction and run."""
    evidence = Evidence(program, built, steps)
    tokens = set()
    for family, rule in FAMILIES.items():
        tokens.update(family + "." + name for name in rule(evidence))
    return tokens


__all__ = ["FAMILIES", "Evidence", "observed", "quoted"]
