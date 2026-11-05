"""Lossless native observations; correctness comparisons belong to the oracles."""

from . import wire


def row(value):
    if value is None:
        return {"kind": "absent"}
    if isinstance(value, tuple):
        return {"kind": "tuple", "items": [row(item) for item in value]}
    # A native row or model, not a dict: `keys()` is the accessor its stubs
    # declare, and nothing here relies on how the object iterates.
    fields = [
        {"name": name, "value": wire.validate(value.tagged(name).snapshot())}
        for name in value.keys()  # noqa: SIM118
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


def compiled(value):
    return {
        "kind": "compiled",
        "sql": value.sql,
        "parameters": [wire.validate(item.snapshot()) for item in value.params],
    }


def error(value):
    return {
        "kind": "error",
        "class": type(value).__name__,
        "cause": str(value),
        "sqlstate": getattr(value, "sqlstate", None),
    }


def construction(value, p):
    result = {"python_type": type(value).__name__}
    if isinstance(value, p.Value):
        result["value"] = wire.validate(value.snapshot())
    elif isinstance(value, str):
        result["name"] = value
    elif isinstance(value, p.RawSQL):
        result["inline_sql"] = value.inline_sql()
    elif isinstance(value, p.ActiveModel):
        entity = p.entity(value.entity_name)
        result["entity"] = value.entity_name
        result["states"] = {
            column["name"]: str(value.get(column["name"]).state)
            for column in entity.describe()["columns"]
        }
    elif isinstance(value, (p.Entity, p.EntityGraph)):
        result["registration"] = value.name
    return result
