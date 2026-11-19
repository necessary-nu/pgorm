"""Pipeline expressions interpreted against independently tracked source columns."""

from dataclasses import dataclass

from .comparison import InvalidOracle
from .reference_literal import literal
from .reference_sql import SQL, binary, bound, join
from .reference_values import quote


@dataclass(frozen=True)
class Expression:
    operation: str
    inputs: dict
    data: dict

    def name(self):
        if self.operation in ("named", "alias"):
            return self.data["name"]
        if self.operation == "column":
            return self.data["column"]
        raise InvalidOracle("reference pipeline requires named computed output columns")

    def sql(self, context, over=None):
        i, d, operation = self.inputs, self.data, self.operation
        values = {
            key: [value.sql(context, over) for value in item]
            if isinstance(item, list)
            else item.sql(context, over)
            for key, item in i.items()
            if key != "value" or operation not in ("value", "bind")
        }
        if operation in ("column", "alias"):
            key = (d.get("source"), d.get("column", d.get("name")))
            if key not in context:
                raise InvalidOracle(
                    "pipeline reference cannot resolve column: " + str(key)
                )
            return context[key]
        if operation in ("value", "bind"):
            return (
                literal(i["value"], pipeline=True)
                if operation == "value"
                else bound(i["value"])
            )
        if operation == "named":
            return values["value"]
        if operation == "cast":
            types = {
                "smallint": "int2",
                "integer": "int4",
                "bigint": "int8",
                "real": "float4",
                "double": "float8",
                "float8": "float8",
                "numeric": "numeric",
                "text": "text",
                "boolean": "bool",
                "date": "date",
                "timestamp": "timestamp",
                "timestamptz": "timestamptz",
                "interval": "interval",
                "uuid": "uuid",
                "json": "json",
                "jsonb": "jsonb",
            }
            if d["name"] not in types:
                raise InvalidOracle("unsupported reference pipeline cast type")
            return "(" + values["value"] + ")::" + quote(types[d["name"]])
        if operation == "binary":
            return binary(values["left"], values["right"], d["operator"])
        if operation == "unary":
            value, operator = values["value"], d["operator"]
            if operator in ("neg", "not"):
                return ("(-" if operator == "neg" else "(NOT ") + value + ")"
            if operator in ("is_null", "is_not_null"):
                return (
                    "("
                    + value
                    + (" IS NULL)" if operator == "is_null" else " IS NOT NULL)")
                )
            return value + " " + operator.upper()
        if operation == "membership":
            return (
                "(" + values["value"] + " IN (" + join(values["items"]) + "))"
                if values["items"]
                else SQL(("FALSE",))
            )
        if operation == "function":
            return function(d["name"], values["arguments"], over)
        raise InvalidOracle("independent pipeline expression uncovered: " + operation)


def function(name, arguments, over):
    functions = {
        "sum": "SUM",
        "min": "MIN",
        "max": "MAX",
        "average": "AVG",
        "count": "COUNT",
        "count_distinct": "COUNT",
        "count_rows": "COUNT",
        "row_number": "ROW_NUMBER",
        "first": "FIRST_VALUE",
        "last": "LAST_VALUE",
        "rank": "RANK",
        "rank_dense": "DENSE_RANK",
    }
    if name in ("rank", "rank_dense"):
        if over is None:
            raise InvalidOracle("ranking requires an explicit window context")
        # The ranked column identifies a series; partition/order determine its
        # peer groups. PostgreSQL's ranking functions take no scalar argument.
        arguments = []
    args = SQL(("*",)) if name == "count_rows" else join(arguments)
    if name == "count_distinct":
        args = "DISTINCT " + args
    result = functions[name] + "(" + args + ")"
    if over is not None:
        result += " OVER (" + over.for_function(name) + ")"
    if name == "sum" and over is None:
        result = "COALESCE(" + result + ", 0)"
    return result


@dataclass(frozen=True)
class Window:
    base: SQL
    frame: SQL
    explicit: bool

    def for_function(self, name):
        # An unframed window still frames these with the whole window, not
        # SQL's default frame, which would end `last` at the current row.
        framed = name in (
            "sum",
            "min",
            "max",
            "average",
            "count",
            "count_rows",
            "count_distinct",
            "first",
            "last",
        )
        return self.base + self.frame if self.explicit or framed else self.base


def window(i, d, context):
    result = SQL()
    if i["partition"]:
        result += "PARTITION BY " + join([item.sql(context) for item in i["partition"]])
    if i["order"]:
        result += " ORDER BY " + join([item.sql(context) for item in i["order"]])
    frame = SQL(
        (
            " ROWS BETWEEN "
            + boundary(d.get("start"), True)
            + " AND "
            + boundary(d.get("end"), False),
        )
    )
    return Window(result, frame, "start" in d or "end" in d)


def boundary(value, start):
    if value is None:
        return "UNBOUNDED PRECEDING" if start else "UNBOUNDED FOLLOWING"
    if value == 0:
        return "CURRENT ROW"
    return str(abs(value)) + (" PRECEDING" if value < 0 else " FOLLOWING")
