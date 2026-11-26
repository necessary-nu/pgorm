"""Registered entities, active models, graphs, cursors and runtime models."""

from .emit_rust_values import (
    PRELUDE,
    REPLAY,
    VARIANTS,
    Q,
    UnsupportedInstruction,
    literal,
)

ENTITIES = {
    "campaign.Account": "account",
    "campaign.Note": "note",
    "campaign.Room": "room",
    "campaign.Stay": "stay",
}
# The physical table each registered entity is declared against, which is the
# qualifier a graph gives its root source.
ENTITY_TABLES = {
    "campaign.Account": "accounts",
    "campaign.Note": "notes",
    "campaign.Room": "rooms",
    "campaign.Stay": "stays",
}
ACCOUNT, NOTE, ROOM = "campaign.Account", "campaign.Note", "campaign.Room"
# Each registered graph: the harness factory that builds it, and one entry per
# joined slot — the entity it decodes into, and whether the slot is required.
# `security/generative/replay/src/entities.rs` holds the compiled shapes and
# `bridge/src/registration.rs` registers the same ones under the same names.
GRAPH_SHAPES = {
    "campaign.AccountOnly": ("account_only", ()),
    "campaign.OptionalNotes": ("optional", ((NOTE, False),)),
    "campaign.RequiredNotes": ("required", ((NOTE, True),)),
    "campaign.SelfJoin": ("self_join", ((ACCOUNT, False),)),
    "campaign.StayRooms": ("stay_rooms", ((ROOM, False),)),
    "campaign.StayGuests": ("stay_guests", ((ACCOUNT, False),)),
    **{
        "campaign.Arity" + str(arity): (
            "arity" + str(arity),
            tuple((NOTE, False) for _ in range(arity - 1)),
        )
        for arity in range(3, 8)
    },
}
# The entity a graph is rooted at, where it is not an account.
GRAPH_ROOTS = {
    "campaign.StayRooms": "campaign.Stay",
    "campaign.StayGuests": "campaign.Stay",
}


def graph_root(name):
    """The registration a graph's root source decodes into."""
    return GRAPH_ROOTS.get(name, ACCOUNT)


# Each registered source tuple, as `registration.rs` declares it: one entity per
# listed position, every position decoding into `Option<Model>`.
SOURCE_SHAPES = {
    "campaign.Sources" + str(arity): (ACCOUNT,) + (NOTE,) * (arity - 1)
    for arity in range(1, 7)
}
# Query shapes whose rows decode into compiled models rather than result rows.
TYPED_ROWS = ("entity_query", "graph_query", "cursor", "sources")
# The Rust type each result column is read as, for kinds with one lossless
# `FromSql` counterpart reachable from the entity prelude.
RESULT_READS = {
    "bool": "bool",
    "i8": "i8",
    "i16": "i16",
    "i32": "i32",
    "i64": "i64",
    "u32": "u32",
    "f32": "f32",
    "f64": "f64",
    "text": "String",
    "bytes": "Vec<u8>",
    "uuid": f"{PRELUDE}::Uuid",
    "date": f"{PRELUDE}::Date",
    "time": f"{PRELUDE}::Time",
    "datetime": f"{PRELUDE}::DateTime",
    "datetime_utc": f"{PRELUDE}::DateTimeWithTimeZone",
    "interval": f"{PRELUDE}::Interval",
}
# Kinds whose driver decoder is only nearly lossless: `numeric` can round,
# JSON numbers can change spelling, and inet/macaddr have no `FromSql` at all.
# Reading them as their bare Rust type would hand the next statement a value
# the Python run never bound, so they go through the replay crate's checked
# codecs — the same ones its observation decoder uses — and all of them box.
CHECKED_READS = {
    "decimal": f"{REPLAY}::codecs::ExactDecimal",
    "json": f"{REPLAY}::codecs::ExactJson",
    "ipnetwork": f"{REPLAY}::codecs::Inet",
    "mac_address": f"{REPLAY}::codecs::Mac",
}
# Variants whose payload is boxed in `pgorm_query::Value`.
BOXED_READS = frozenset(
    {"text", "bytes", "uuid", "date", "time", "datetime", "datetime_utc", "interval"}
    | CHECKED_READS.keys()
)


def readable(tag):
    """Whether a decoded tag has a lossless standalone-Rust result read.

    An enum is read as a label only after its column's type identity matches
    the declared name and schema, which the replay crate's `declared_enum`
    checks; every other kind needs a bare or checked `FromSql` read.
    """
    return tag["kind"] == "enum" or tag["kind"] in RESULT_READS.keys() | CHECKED_READS


# [spec:pgorm:req:generative.replay]
class ModelEmitter:
    """Emit the compiled entity, graph, cursor and runtime-model shapes."""

    def entity_type(self, name):
        """The compiled Rust entity a registration name denotes."""
        if name not in ENTITIES:
            raise UnsupportedInstruction("no compiled entity for " + name)
        return f"{REPLAY}::entities::{ENTITIES[name]}::Entity"

    def entity_path(self, reference):
        return self.entity_type(self.nodes[reference]["data"]["name"])

    # -- compiled entity, graph and source shapes ---------------------------

    def registration(self, reference):
        """The registration whose models a typed node carries, by construction.

        Every one of these operations threads a single compiled entity through
        without ever changing it, so the identity is read off the declaration
        rather than recovered from the value.
        """
        node = self.nodes[reference]
        name, d, i = node["op"], node["data"], node["inputs"]
        if name == "entity":
            return d["name"]
        if name in ("entity.find", "entity.active"):
            return self.registration(i["entity"])
        if name in ("entity.filter", "entity.order", "entity.page"):
            return self.registration(i["query"])
        if name in ("entity.into_active", "active.set"):
            return self.registration(i["model"])
        if name == "entity.result":
            return self.result_shape(node)[0]
        raise UnsupportedInstruction("no compiled entity behind " + name)

    def graph_find(self, reference):
        """The `graph.find` a graph query or cursor was built from."""
        node = self.nodes[reference]
        if node["op"] == "graph.find":
            return node
        for key in ("query", "cursor"):
            if key in node["inputs"]:
                return self.graph_find(node["inputs"][key])
        raise UnsupportedInstruction("no graph shape behind " + node["op"])

    def graph_shape(self, reference):
        """A graph query's registration, aliases and joined slot declarations."""
        node = self.graph_find(reference)
        name = self.nodes[node["inputs"]["graph"]]["data"]["name"]
        if name not in GRAPH_SHAPES:
            raise UnsupportedInstruction("no compiled graph for " + name)
        _, slots = GRAPH_SHAPES[name]
        aliases = list(node["data"]["aliases"])
        if len(aliases) != len(slots):
            raise UnsupportedInstruction(
                "graph alias count does not match the compiled shape"
            )
        return name, aliases, slots

    def source_shape(self, reference):
        """A `pipeline.sources` node's registration and its listed entities."""
        data = self.nodes[reference]["data"]
        name = data["name"]
        if name not in SOURCE_SHAPES:
            raise UnsupportedInstruction("no compiled source tuple for " + name)
        entities = SOURCE_SHAPES[name]
        if len(data["qualifiers"]) != len(entities):
            raise UnsupportedInstruction(
                "source qualifier count does not match the compiled tuple"
            )
        return name, list(data["qualifiers"]), entities

    def result_shape(self, node):
        """What `entity.result` reads: a registration and the Rust access path.

        The step's retained rows are already typed — models for an entity read
        or an active write, graph items for a graph read — so the slot is
        addressed positionally rather than by decoding anything again.
        """
        self.helpers.add("decoded")
        d = node["data"]
        rows = "r_" + d["step"]
        step = self.scheduled.get(d["step"])
        if step is None:
            raise UnsupportedInstruction("entity.result names no scheduled step")
        access = f"decoded(&{rows}, {int(d['row'])}usize)?"
        if step["op"] == "active.write":
            if step["data"]["method"] == "delete":
                raise UnsupportedInstruction("a delete retains no model to read")
            return self.registration(step["inputs"]["model"]), access
        if step["op"] != "fetch":
            raise UnsupportedInstruction(
                "entity.result requires a model-producing step, not " + step["op"]
            )
        query = step["inputs"]["query"]
        kind = self.types[query]
        index = d.get("source")
        if kind == "entity_query":
            if index:
                raise UnsupportedInstruction("an entity read has one source slot")
            return self.registration(query), access
        if kind not in ("graph_query", "cursor"):
            raise UnsupportedInstruction(
                "entity.result requires a compiled model read, not a " + kind
            )
        name, _, slots = self.graph_shape(query)
        if not slots:
            if index:
                raise UnsupportedInstruction("this graph has one source slot")
            return graph_root(name), access
        if index is None:
            raise UnsupportedInstruction(
                "a joined graph row needs the source slot to read"
            )
        if index == 0:
            return graph_root(name), access + ".0"
        if index > len(slots):
            raise UnsupportedInstruction("source slot is outside the graph shape")
        entity, required = slots[index - 1]
        if required:
            return entity, f"{access}.{index}"
        self.helpers.add("present")
        return entity, f"present({access}.{index})?"

    # -- compiled entities --------------------------------------------------

    def entity_column(self, registration, name):
        """The compiled column a portable name denotes, resolved at runtime."""
        entity = self.entity_type(registration)
        return f"{REPLAY}::entities::column::<{entity}>({literal(name)})?"

    def compiled_entity(self, node):
        name, d, i = node["op"], node["data"], node["inputs"]
        match name:
            case "entity.column":
                column = self.entity_column(self.registration(i["entity"]), d["name"])
                return (
                    f"{Q}::SimpleExpr::from({Q}::Expr::col("
                    f"pgorm::Column::as_column_ref(&{column})))"
                )
            case "entity.predicate":
                column = self.entity_column(self.registration(i["entity"]), d["column"])
                return (
                    f"pgorm::Column::{d['operator']}(&{column}, {self.use(i['value'])})"
                )
            case "entity.find":
                entity = self.entity_type(self.registration(i["entity"]))
                return f"<{entity} as pgorm::Entity>::find()"
            case "entity.filter":
                query = self.use(i["query"])
                return f"pgorm::QueryFilter::filter({query}, {self.predicate(i['predicate'])})"
            case "entity.order":
                return self.typed_order(node)
            case "entity.page":
                return self.typed_page(node)
            case "entity.active":
                entity = self.entity_type(self.registration(i["entity"]))
                return (
                    f"<<{entity} as pgorm::Entity>::ActiveModel"
                    " as pgorm::ActiveModelBehavior>::new()"
                )
            case "entity.result":
                return self.result_shape(node)[1]
            case "entity.into_active":
                return (
                    f"pgorm::IntoActiveModel::into_active_model({self.use(i['model'])})"
                )
            case "active.set":
                return self.active_set(node)
        raise UnsupportedInstruction("no public Rust source for " + name)

    def typed_order(self, node):
        """`QueryOrder` over a typed query, one declared key at a time."""
        i = node["inputs"]
        body = [f"    let mut query = {self.use(i['query'])};"]
        for key in i["keys"]:
            body.append(f"    let (expression, order, nulls) = {self.use(key)};")
            body.append("    query = match nulls {")
            body.append(
                "        Some(nulls) => "
                "pgorm::QueryOrder::order_by_with_nulls(query, expression, order, nulls),"
            )
            body.append(
                "        None => pgorm::QueryOrder::order_by(query, expression, order),"
            )
            body.append("    };")
        return "{\n" + "\n".join(body) + "\n    query\n    }"

    def typed_page(self, node):
        d, i = node["data"], node["inputs"]
        methods = [method for method in ("limit", "offset") if method in d]
        if not methods:
            raise UnsupportedInstruction("entity.page requires a limit or an offset")
        result = self.use(i["query"])
        for method in methods:
            result = f"pgorm::QuerySelect::{method}({result}, {int(d[method])}u64)"
        return result

    def active_set(self, node):
        d, i = node["data"], node["inputs"]
        column = self.entity_column(self.registration(i["model"]), d["column"])
        body = [f"    let mut active = {self.use(i['model'])};"]
        if d["state"] == "set":
            if "value" not in i:
                raise UnsupportedInstruction("active.set set needs a value")
            body.append(
                "    pgorm::ActiveModel::set(&mut active, "
                f"{column}, {self.use(i['value'])})?;"
            )
        else:
            body.append(f"    pgorm::ActiveModel::{d['state']}(&mut active, {column});")
        return "{\n" + "\n".join(body) + "\n    active\n    }"

    # -- compiled graphs and cursors ----------------------------------------

    def compiled_graph(self, node):
        name, d, i = node["op"], node["data"], node["inputs"]
        match name:
            case "graph.find":
                aliases = ", ".join(
                    f"{literal(alias)}.to_owned()" for alias in d["aliases"]
                )
                return f"{self.use(i['graph'])}(&[{aliases}])"
            case "graph.column":
                return self.graph_column(node)
            case "graph.filter":
                query = self.use(i["query"])
                return f"pgorm::QueryFilter::filter({query}, {self.predicate(i['predicate'])})"
            case "graph.order":
                return self.typed_order(node)
            case "graph.cursor":
                column = self.entity_column(ACCOUNT, d["column"])
                return f"{self.use(i['query'])}.cursor_by({column})"
            case "cursor.bound":
                return self.cursor_bound(node)
            case "cursor.page":
                body = [
                    f"    let mut cursor = {self.use(i['cursor'])};",
                    f"    cursor.{d['side']}({int(d['count'])}u64);",
                    f"    cursor.{d['direction']}();",
                ]
                return "{\n" + "\n".join(body) + "\n    cursor\n    }"
        raise UnsupportedInstruction("no public Rust source for " + name)

    def graph_column(self, node):
        """A graph's column, qualified the way the shape qualifies its sources.

        The root keeps its entity's table name — `Entity` fixes it, and no
        alias is offered for it — while each joined slot answers to the alias
        the find supplied.
        """
        d, i = node["data"], node["inputs"]
        name, aliases, slots = self.graph_shape(i["query"])
        index = d["source"]
        if index > len(slots):
            raise UnsupportedInstruction("source is outside the registered graph shape")
        root = ENTITY_TABLES[graph_root(name)]
        qualifier = root if index == 0 else aliases[index - 1]
        reference = (
            f"({Q}::Name::runtime({literal(qualifier)}), "
            f"{Q}::Name::runtime({literal(d['column'])}))"
        )
        return f"{Q}::SimpleExpr::from({Q}::Expr::col({reference}))"

    def cursor_bound(self, node):
        d, i = node["data"], node["inputs"]
        values = ", ".join(self.uses(i["values"]))
        body = [
            f"    let mut cursor = {self.use(i['cursor'])};",
            f"    let key = vec![{values}].into_iter().collect::<{Q}::ValueTuple>();",
            f"    cursor.{d['side']}_with(key);",
        ]
        return "{\n" + "\n".join(body) + "\n    cursor\n    }"

    # -- runtime models -----------------------------------------------------

    def model_fields(self, reference):
        """A runtime model's declared (logical, physical) fields, in order."""
        node = self.nodes[reference]
        declared = node["data"].get("fields")
        if declared is not None:
            return tuple(declared.items())
        schema, name, _ = self.table_identity(node["inputs"]["table"])
        definition = next(
            (
                item
                for item in self.data["fixture"]["tables"]
                if (item["schema"], item["name"]) == (schema, name)
            ),
            None,
        )
        if definition is None:
            raise UnsupportedInstruction(
                "a runtime model over an undeclared table has no known columns"
            )
        return tuple((item["name"], item["name"]) for item in definition["columns"])

    def model_projection(self, reference, names=None):
        """`Model._projected`: each declared field's column, aliased to its
        logical name where the two differ."""
        fields = dict(self.model_fields(reference))
        selected = list(fields) if not names else list(names)
        items = []
        for logical in selected:
            if logical not in fields:
                raise UnsupportedInstruction("unknown runtime model field")
            items.append(
                (
                    self.model_column(reference, fields[logical]),
                    logical,
                    fields[logical],
                )
            )
        return items

    def model_column(self, reference, physical):
        """A runtime model's column, qualified as `Model.col` qualifies it: by
        the table's alias or else its name, never its schema."""
        table = self.nodes[self.nodes[reference]["inputs"]["table"]]
        alias = table["data"].get("alias")
        qualifier = (
            f"{Q}::Name::runtime({literal(alias)})"
            if alias is not None
            else self.alias_of(table, "name")
        )
        column = f"{Q}::IntoName::into_name({Q}::Name::runtime({literal(physical)}))"
        reference = f"{Q}::ColumnRef::TableColumn({Q}::IntoName::into_name({qualifier}), {column})"
        return f"{Q}::SimpleExpr::from({Q}::Expr::col({reference}))"

    def runtime_model(self, node):
        name, d, i = node["op"], node["data"], node["inputs"]
        match name:
            case "model.column":
                fields = dict(self.model_fields(i["model"]))
                if d["name"] not in fields:
                    raise UnsupportedInstruction("unknown runtime model field")
                return self.model_column(i["model"], fields[d["name"]])
            case "model.select":
                return self.model_select(node)
            case "model.write":
                return self.model_write(node)
            case "model.returning":
                return self.model_returning(node)
        raise UnsupportedInstruction("no public Rust source for " + name)

    def model_select(self, node):
        i = node["inputs"]
        body = [f"    let mut query = {Q}::Query::select();"]
        for expression, logical, physical in self.model_projection(i["model"]):
            body.append(
                f"    query.expr({expression});"
                if logical == physical
                else f"    query.expr_as({expression}, {Q}::Name::runtime({literal(logical)}));"
            )
        body.append(f"    query.from({self.use(i['model'])});")
        if "predicate" in i:
            body.append(f"    query.cond_where({self.predicate(i['predicate'])});")
        body.extend(self.order_lines(i.get("order", ())))
        return "{\n" + "\n".join(body) + "\n    query\n    }"

    def model_write(self, node):
        d, i = node["data"], node["inputs"]
        fields = dict(self.model_fields(i["model"]))
        method, table = d["method"], self.use(i["model"])
        columns = []
        for logical in d["columns"]:
            if logical not in fields:
                raise UnsupportedInstruction("unknown runtime model field")
            columns.append(fields[logical])
        values = [self.coerce(value) for value in i["values"]]
        if method == "insert":
            body = [
                (
                    f"    let mut query = {Q}::Query::insert()"
                    f".into_table({table}).to_owned();"
                )
            ]
            if columns:
                self.helpers.add("arity")
                names = ", ".join(
                    f"{Q}::Name::runtime({literal(column)})" for column in columns
                )
                body.append(f"    query.columns(vec![{names}]);")
                body.append(
                    f"    arity(query.values(vec![{', '.join(values)}]).map(|_| ()))?;"
                )
            else:
                body.append("    query.or_default_values();")
        elif method == "update":
            body = [
                f"    let mut query = {Q}::Query::update().table({table}).to_owned();"
            ]
            if not columns:
                raise UnsupportedInstruction(
                    "a runtime model update assigns at least one field"
                )
            for column, value in zip(columns, values, strict=True):
                body.append(
                    f"    query.value({Q}::Name::runtime({literal(column)}), {value});"
                )
        else:
            body = [
                (
                    f"    let mut query = {Q}::Query::delete()"
                    f".from_table({table}).to_owned();"
                )
            ]
        if "predicate" in i:
            body.append(f"    query.cond_where({self.predicate(i['predicate'])});")
        return "{\n" + "\n".join(body) + "\n    query\n    }"

    def model_returning(self, node):
        d, i = node["data"], node["inputs"]
        model = self.nodes[i["query"]]["inputs"]["model"]
        items = []
        for _, logical, physical in self.model_projection(model, d["columns"]):
            # RETURNING names the written table's columns unqualified.
            column = f"{Q}::Name::runtime({literal(physical)})"
            expression = f"{Q}::SimpleExpr::from({Q}::Expr::col({column}))"
            alias = f"{Q}::Name::runtime({literal(logical)})"
            items.append(
                expression
                if logical == physical
                else f"{expression}.binary({Q}::BinOper::As, "
                f"{Q}::SimpleExpr::from({Q}::Expr::col({alias})))"
            )
        body = [
            f"    let mut query = {self.use(i['query'])};",
            (
                f"    query.returning({Q}::Query::returning()"
                f".exprs(vec![{', '.join(items)}]));"
            ),
        ]
        return "{\n" + "\n".join(body) + "\n    query\n    }"

    def model_result_value(self, node, registration):
        """Read a retained model's column, the way `Model::get` does."""
        self.helpers.add("decoded")
        d = node["data"]
        variant = VARIANTS.get(d["type"]["kind"])
        if variant is None or d["type"]["kind"] == "enum":
            # An enum's tag carries a qualified identity the Rust variant alone
            # cannot confirm, so the declared tag cannot be checked here.
            raise UnsupportedInstruction(
                "result.value has no compiled model read for " + d["type"]["kind"]
            )
        column = self.entity_column(registration, d["column"])
        rows = "r_" + d["step"]
        body = [
            (
                f"    let found = pgorm::Model::get("
                f"&decoded(&{rows}, {int(d['row'])}usize)?, {column});"
            ),
            f"    if !matches!(found, {Q}::Value::{variant}(_)) {{",
            f"        return Err(Error::Format({REPLAY}::FormatError::new(",
            '            "result reference type differs from the declared value tag",',
            "        )));",
            "    }",
        ]
        return "{\n" + "\n".join(body) + "\n    found\n    }"

    def result_value(self, node):
        """Read a prior row's column, the way `Row::try_get` does."""
        d = node["data"]
        tag = d["type"]
        step = self.scheduled.get(d["step"])
        if step is not None and step["op"] == "active.write":
            return self.model_result_value(
                node, self.registration(step["inputs"]["model"])
            )
        if step is not None and step["op"] == "fetch":
            kind = self.types[step["inputs"]["query"]]
            if kind == "entity_query":
                return self.model_result_value(
                    node, self.registration(step["inputs"]["query"])
                )
            if kind in TYPED_ROWS:
                raise UnsupportedInstruction(
                    "result.value reads one row's column; a " + kind + " row is a tuple"
                )
        if tag["kind"] not in VARIANTS or not readable(tag):
            raise UnsupportedInstruction(
                "result.value has no typed Rust read for " + tag["kind"]
            )
        rows = "r_" + d["step"]
        column = literal(d["column"])
        if tag["kind"] == "enum":
            payload = (
                f"{REPLAY}::codecs::declared_enum(&{rows}[{d['row']}], {column}, "
                f"{literal(tag['name'])}, {literal(tag['schema'])})?"
            )
            return f"{Q}::Value::String(Some(Box::new({payload})))"
        if tag["kind"] in CHECKED_READS:
            codec = CHECKED_READS[tag["kind"]]
            payload = f"{rows}[{d['row']}].try_get::<_, {codec}>({column})?.0"
        else:
            native = RESULT_READS[tag["kind"]]
            payload = f"{rows}[{d['row']}].try_get::<_, {native}>({column})?"
        if tag["kind"] in BOXED_READS:
            return f"{Q}::Value::{VARIANTS[tag['kind']]}(Some(Box::new({payload})))"
        return f"{Q}::Value::{VARIANTS[tag['kind']]}(Some({payload}))"
