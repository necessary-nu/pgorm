"""Public scalar-expression and identifier instruction dispatch."""

import operator

from .native_values import materialize

ARITHMETIC = {
    "add": operator.add,
    "sub": operator.sub,
    "mul": operator.mul,
    "div": operator.truediv,
    "mod": operator.mod,
    "and": operator.and_,
    "or": operator.or_,
}


def binary(left, right, operation):
    if operation in ARITHMETIC:
        return ARITHMETIC[operation](left, right)
    return getattr(left, operation)(right)


def unary(value, operation):
    if operation == "not":
        return ~value
    if operation == "neg":
        return -value
    return getattr(value, operation)()


def dispatch(name, i, d, p):
    match name:
        case "value":
            return materialize(d["value"], p), ["pgorm_query::Value"]
        case "name":
            return p.Identifier(i["value"].value), ["pgorm_query::Name"]
        case "table":
            options = {key: value for key, value in d.items() if key != "name"}
            return p.Table(i.get("name", d.get("name")), **options), [
                "pgorm_query::TableRef"
            ]
        case "expr.column":
            column = i.get("name", d.get("name"))
            result = i["table"].col(column) if "table" in i else p.col(column)
            return result, ["pgorm_query::Expr::col"]
        case "expr.value":
            function, path = (
                (p.literal, "pgorm_query::SimpleExpr::Constant")
                if d["mode"] == "literal"
                else (p.bind, "pgorm_query::Expr::value")
            )
            return function(i["value"]), [path]
        case "expr.binary":
            left = p.bind(i["left"]) if isinstance(i["left"], p.Value) else i["left"]
            return binary(left, i["right"], d["operator"]), ["pgorm_query::SimpleExpr"]
        case "expr.unary":
            return unary(i["value"], d["operator"]), ["pgorm_query::SimpleExpr"]
        case "expr.membership":
            method = "is_not_in" if d["negated"] else "is_in"
            return getattr(i["value"], method)(i["items"]), [
                "pgorm_query::Expr::" + method
            ]
        case "expr.pattern":
            method = d["method"]
            pattern = i["pattern"].value
            path = "pgorm_query::SimpleExpr::" + method
            if method in ("like", "not_like", "ilike", "not_ilike"):
                pattern = p.LikeExpr(pattern)
                if d.get("escape") is not None:
                    pattern = pattern.escape(d["escape"])
                path = "pgorm_query::LikeExpr"
            return getattr(i["value"], method)(pattern), [path]
        case "expr.call":
            return p.call(d["name"], *i["arguments"]), ["pgorm_query::Func"]
        case "expr.cast":
            kind = p.TypeName(d["name"], schema=d.get("schema"))
            return i["value"].cast(kind, array=d.get("array", False)), [
                "pgorm_query::SimpleExpr::cast_as_type"
            ]
        case "expr.alias":
            return i["value"].as_(i.get("name", d.get("name"))), [
                "pgorm_query::SelectStatement::expr_as"
            ]
        case "expr.order":
            nulls = (
                None
                if d["nulls"] == "default"
                else getattr(p.Nulls, d["nulls"].title())
            )
            return getattr(i["value"], d["direction"])(nulls=nulls), [
                "pgorm_query::OrderedStatement"
            ]
        case "condition":
            result = getattr(p.Condition, d["mode"])(*i["items"])
            return (~result if d.get("negated") else result), ["pgorm_query::Condition"]
    raise RuntimeError("inactive expression dispatch: " + name)
