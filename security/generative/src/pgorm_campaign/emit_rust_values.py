"""Rust names, escaped literals, tagged values and table identity."""

from . import wire

Q = "pgorm::pgorm_query"
PL = "pgorm::pipeline"
REPLAY = "pgorm_generative_replay"
PRELUDE = "pgorm::entity::prelude"

# `wire.SCALARS` names to the pgorm_query::Value variant carrying each one.
VARIANTS = {
    "bool": "Bool",
    "i8": "TinyInt",
    "i16": "SmallInt",
    "i32": "Int",
    "i64": "BigInt",
    "u32": "Unsigned",
    "u64": "BigUnsigned",
    "f32": "Float",
    "f64": "Double",
    "text": "String",
    "enum": "String",
    "char": "Char",
    "bytes": "Bytes",
    "json": "Json",
    "decimal": "Decimal",
    "uuid": "Uuid",
    "date": "Date",
    "time": "Time",
    "datetime": "DateTime",
    "datetime_utc": "DateTimeWithTimeZone",
    "ipnetwork": "IpNetwork",
    "mac_address": "MacAddress",
    "vector": "Vector",
    "interval": "Interval",
}
# `wire.RANGES` names to the pgorm_query::RangeType each one is.
RANGE_TYPES = {
    "int4range": "Int4",
    "int8range": "Int8",
    "numrange": "Numeric",
    "daterange": "Date",
    "tsrange": "Timestamp",
    "tstzrange": "TimestampTz",
}
INTEGER_SUFFIX = {
    "i8": "i8",
    "i16": "i16",
    "i32": "i32",
    "i64": "i64",
    "u32": "u32",
    "u64": "u64",
}
TEMPORAL_PARSERS = {
    "date": "parse_date",
    "time": "parse_time",
    "datetime": "parse_naive_datetime",
    "datetime_utc": "parse_datetime_utc",
}


class UnsupportedInstruction(Exception):
    """A catalog instruction that standalone Rust source cannot express."""


# [spec:pgorm:req:generative.replay]
def literal(text):
    """Escape text into a Rust string literal so hostile input keeps its quotes.

    The result is pure ASCII: every control character and every non-ASCII
    scalar leaves as ``\\u{..}``, so a fixture name carrying a quote, a
    backslash, NUL or a dollar-quoted tag cannot terminate the literal or
    smuggle source past it.
    """
    if not isinstance(text, str):
        raise UnsupportedInstruction("only text has a Rust string literal")
    parts = ['"']
    for character in text:
        point = ord(character)
        if character == '"':
            parts.append('\\"')
        elif character == "\\":
            parts.append("\\\\")
        elif 0x20 <= point <= 0x7E:
            parts.append(character)
        elif 0xD800 <= point <= 0xDFFF:
            raise UnsupportedInstruction("a lone surrogate has no Rust literal")
        else:
            parts.append(f"\\u{{{point:x}}}")
    parts.append('"')
    return "".join(parts)


def char_literal(text):
    if len(text) != 1:
        raise UnsupportedInstruction("a character payload holds one scalar")
    return f"'\\u{{{ord(text):x}}}'"


def integer(value, kind):
    return str(int(value)) + INTEGER_SUFFIX[kind]


def boolean(value):
    return "true" if value else "false"


def json_source(value):
    """Rebuild a JSON payload structurally; parsing text back would be lossy."""
    if value is None:
        return "serde_json::Value::Null"
    if isinstance(value, bool):
        return f"serde_json::Value::Bool({boolean(value)})"
    if isinstance(value, int):
        suffix = "i64" if value < 0 else "u64"
        return f"serde_json::Value::from({value}{suffix})"
    if isinstance(value, float):
        return f"serde_json::Value::from({float.__repr__(value)}f64)"
    if isinstance(value, str):
        return f"serde_json::Value::String({literal(value)}.to_owned())"
    if isinstance(value, list):
        items = ", ".join(json_source(item) for item in value)
        return f"serde_json::Value::Array(vec![{items}])"
    if isinstance(value, dict):
        if not value:
            return "serde_json::Value::Object(serde_json::Map::new())"
        items = ", ".join(
            f"({literal(key)}.to_owned(), {json_source(item)})"
            for key, item in value.items()
        )
        return f"serde_json::Value::Object(serde_json::Map::from_iter([{items}]))"
    raise UnsupportedInstruction("JSON payload has no Rust source")


def type_name(name, schema):
    result = f"{Q}::TypeName::new({Q}::Name::runtime({literal(name)}))"
    if schema is not None:
        result += f".schema({Q}::Name::runtime({literal(schema)}))"
    return result


def type_ref(name, schema):
    local = f"{Q}::IntoName::into_name({Q}::Name::runtime({literal(name)}))"
    if schema is None:
        return f"{Q}::extension::TypeRef::Type({local})"
    outer = f"{Q}::IntoName::into_name({Q}::Name::runtime({literal(schema)}))"
    return f"{Q}::extension::TypeRef::SchemaType({outer}, {local})"


def array_type(tag):
    kind = tag["kind"]
    if kind == "array":
        raise UnsupportedInstruction("nested arrays have no Rust value")
    return f"{Q}::ArrayType::{VARIANTS[kind]}"


def enum_cast(tag):
    """The qualified cast a tagged enum value carries, as the binding adds it."""
    if tag["kind"] == "enum":
        return type_name(tag["name"], tag["schema"])
    if tag["kind"] == "array" and tag["element"]["kind"] == "enum":
        element = tag["element"]
        return type_name(element["name"], element["schema"]) + ".array()"
    return None


# [spec:pgorm:req:generative.replay]
class ValueEmitter:
    """Emit a declared value, and the identity of a table or column."""

    def scalar(self, tag, data):
        kind = tag["kind"]
        if kind in INTEGER_SUFFIX:
            return integer(data, kind)
        if kind == "bool":
            return boolean(data)
        if kind in ("f32", "f64"):
            bits = "u32" if kind == "f32" else "u64"
            return f"{kind}::from_bits(0x{data}{bits})"
        if kind in ("text", "enum"):
            return f"Box::new({literal(data)}.to_owned())"
        if kind == "char":
            return char_literal(data)
        if kind in ("bytes", "mac_address"):
            items = ", ".join(f"{byte}u8" for byte in data)
            if kind == "bytes":
                return f"Box::new(vec![{items}])"
            self.helpers.add("parsed")
            octets = ":".join(f"{byte:02x}" for byte in data)
            return (
                f"Box::new(parsed::<{PRELUDE}::MacAddress>"
                f'({literal(octets)}, "MAC address")?)'
            )
        if kind == "json":
            return f"Box::new({json_source(data)})"
        if kind == "decimal":
            self.helpers.add("decimal")
            whole, _, fraction = data.partition(".")
            units = int(whole + fraction)
            return f"Box::new(decimal({units}i128, {len(fraction)}u32)?)"
        if kind == "uuid":
            number = int(data.replace("-", ""), 16)
            return f"Box::new({PRELUDE}::Uuid::from_u128(0x{number:032x}u128))"
        if kind == "ipnetwork":
            self.helpers.add("parsed")
            return (
                f"Box::new(parsed::<{PRELUDE}::IpNetwork>"
                f'({literal(data)}, "IP network")?)'
            )
        if kind in TEMPORAL_PARSERS:
            text = literal(wire.temporal_text(data))
            return f"Box::new({REPLAY}::wire::{TEMPORAL_PARSERS[kind]}({text})?)"
        if kind == "vector":
            items = ", ".join(f"f32::from_bits(0x{item}u32)" for item in data)
            return f"Box::new({PRELUDE}::Vector::from(vec![{items}]))"
        if kind == "interval":
            months, days, microseconds = wire.interval_fields(data)
            return (
                f"Box::new({Q}::Interval::new("
                f"{months}i32, {days}i32, {microseconds}i64))"
            )
        raise UnsupportedInstruction("value kind has no Rust source: " + kind)

    def value_source(self, snapshot):
        """Rebuild a tagged value through the public Rust Value constructors."""
        wire.validate(snapshot)
        tag = snapshot["type"]
        if tag["kind"] == "array":
            element = array_type(tag["element"])
            if snapshot["sql_null"]:
                return f"{Q}::Value::Array({element}, None)"
            items = ", ".join(self.value_source(item) for item in snapshot["data"])
            return f"{Q}::Value::Array({element}, Some(Box::new(vec![{items}])))"
        if tag["kind"] in RANGE_TYPES:
            range_type = f"{Q}::RangeType::{RANGE_TYPES[tag['kind']]}"
            if snapshot["sql_null"]:
                return f"{Q}::Value::Range({range_type}, None)"
            bounds = self.range_source(tag, snapshot["data"])
            return f"{Q}::Value::Range({range_type}, Some(Box::new({bounds})))"
        variant = VARIANTS.get(tag["kind"])
        if variant is None:
            raise UnsupportedInstruction("value kind has no Rust value variant")
        if snapshot["sql_null"]:
            return f"{Q}::Value::{variant}(None)"
        return f"{Q}::Value::{variant}(Some({self.scalar(tag, snapshot['data'])}))"

    def range_source(self, tag, data):
        """A `Range` of the bound values, each built as its own scalar is."""
        if data == {"empty": True}:
            return f"{Q}::Range::Empty"
        sides = []
        for side, closed in (("lower", "["), ("upper", "]")):
            if data[side] is None:
                sides.append("std::ops::Bound::Unbounded")
                continue
            bound = self.value_source(wire.scalar(wire.RANGES[tag["kind"]], data[side]))
            which = "Included" if closed in data["bounds"] else "Excluded"
            sides.append(f"std::ops::Bound::{which}({bound})")
        return f"{Q}::Range::new({sides[0]}, {sides[1]})"

    def value_tag(self, reference):
        """The declared tag of a value-producing node, for its enum cast."""
        node = self.nodes[reference]
        if node["op"] == "value":
            return node["data"]["value"]["type"]
        if node["op"] == "result.value":
            return node["data"]["type"]
        return None

    def coerce(self, reference):
        """A value or expression as a SimpleExpr, the way `coerce` does."""
        if self.types[reference] != "value":
            return self.use(reference)
        return self.bound_value(reference)

    def bound_value(self, reference, *, mode="bound"):
        inner = (
            f"{Q}::SimpleExpr::Constant({self.use(reference)})"
            if mode == "literal"
            else f"{Q}::Expr::value({self.use(reference)})"
        )
        cast = enum_cast(self.value_tag(reference) or {"kind": "text"})
        return f"{inner}.cast_as_type({cast})" if cast else inner

    def alias_of(self, node, key):
        """An identifier that is either declared data or a runtime name."""
        if key in node["inputs"]:
            return self.use(node["inputs"][key])
        return f"{Q}::Name::runtime({literal(node['data'][key])})"

    # -- table and column identity ------------------------------------------

    def table_source(self, node):
        name = self.alias_of(node, "name")
        inner = f"{Q}::IntoName::into_name({name})"
        schema = node["data"].get("schema")
        if schema is None:
            table = f"{Q}::TableName::Table({inner})"
        else:
            outer = f"{Q}::IntoName::into_name({Q}::Name::runtime({literal(schema)}))"
            table = f"{Q}::TableName::SchemaTable({outer}, {inner})"
        result = f"{Q}::NamedTable::from({table})"
        alias = node["data"].get("alias")
        if alias is not None:
            result += f".alias({Q}::Name::runtime({literal(alias)}))"
        return result

    def table_identity(self, reference):
        """A table node's static schema and name, when both are declared."""
        node = self.nodes[reference]
        if "name" not in node["data"]:
            raise UnsupportedInstruction(
                "a pipeline source needs a statically named table"
            )
        return (
            node["data"].get("schema"),
            node["data"]["name"],
            node["data"].get("alias"),
        )

    def table_column(self, reference, column):
        """A column qualified by a table node, the way `Table::col` qualifies."""
        table = self.nodes[reference]
        alias = table["data"].get("alias")
        schema = table["data"].get("schema")
        if alias is not None:
            qualifier = (
                f"{Q}::IntoName::into_name({Q}::Name::runtime({literal(alias)}))"
            )
            return f"{Q}::ColumnRef::TableColumn({qualifier}, {column})"
        name = f"{Q}::IntoName::into_name({self.alias_of(table, 'name')})"
        if schema is None:
            return f"{Q}::ColumnRef::TableColumn({name}, {column})"
        outer = f"{Q}::IntoName::into_name({Q}::Name::runtime({literal(schema)}))"
        return f"{Q}::ColumnRef::SchemaTableColumn({outer}, {name}, {column})"

    def column_source(self, node):
        column = f"{Q}::IntoName::into_name({self.alias_of(node, 'name')})"
        if "table" not in node["inputs"]:
            reference = f"{Q}::ColumnRef::Column({column})"
        else:
            reference = self.table_column(node["inputs"]["table"], column)
        return f"{Q}::SimpleExpr::from({Q}::Expr::col({reference}))"
