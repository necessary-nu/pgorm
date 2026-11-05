"""Emit concrete wrappers from the installed, compiled application registry."""

import ast
import json
import keyword
from pathlib import Path
from os import PathLike
import re
from typing import Any

from .config import CodegenError, identifier, validate
from .source import Call, Code, bracket, flat, from_import, literal, string, union
from .types import FieldType, field_type

RESERVED_FIELDS = {
    "native",
    "entity_name",
    "tagged",
    "with_value",
    "into_active",
    "get",
    "keys",
    "values",
    "items",
    "property",
}


def attributes(
    entity: dict[str, Any], entry: dict[str, Any]
) -> list[tuple[dict[str, Any], str, FieldType]]:
    columns = {column["name"]: column for column in entity["columns"]}
    if set(entry["fields"]) - set(columns):
        raise CodegenError("field aliases name unknown compiled SQL columns")
    used = set(RESERVED_FIELDS)
    result = []
    for column in entity["columns"]:
        chosen = entry["fields"].get(column["name"])
        if chosen is None:
            chosen = column["name"]
            if not chosen.isidentifier() or keyword.iskeyword(chosen):
                chosen = column["json_key"]
        chosen = identifier(
            chosen, "field attribute; supply an explicit fields alias if needed"
        )
        if chosen in used:
            raise CodegenError(
                "field attributes collide with each other or a model method; supply fields aliases"
            )
        used.add(chosen)
        result.append((column, chosen, field_type(column)))
    return result


def entity_code(
    entry: dict[str, Any], entity: dict[str, Any]
) -> tuple[list[str], list[str]]:
    """An entity's model and active classes and its view, as the module's
    top-level blocks, and the stub's."""
    name = entry["python"]
    model, active = name + "Model", name + "Active"
    fields = attributes(entity, entry)
    registration = string(entry["name"])
    model_code = [
        f'class {model}(ModelView["{active}"]):',
        f"    _entity_name = {registration}",
    ]
    model_stub = [f"class {model}(ModelView[{active}]):"]
    for column, attribute, type_ in fields:
        model_code.extend(
            [
                "",
                "    @property",
                bracket(f"def {attribute}(", ["self"], ")", 4, f" -> {type_.annotation}:"),
                bracket(
                    "return cast(",
                    [type_.annotation, f"self[{string(column['name'])}]"],
                    ")",
                    8,
                ),
            ]
        )
        model_stub.extend(
            [
                "    @property",
                bracket(f"def {attribute}(", ["self"], ")", 4, f" -> {type_.annotation}: ..."),
            ]
        )
    active_code = [
        f"class {active}(ActiveView[{model}]):",
        f"    _entity_name = {registration}",
        f"    _model_class = {model}",
    ]
    active_stub = [f"class {active}(ActiveView[{model}]):"]
    for column, attribute, type_ in fields:
        annotation = (
            f"{type_.annotation} | p.Value" if type_.annotation != "Any" else "Any"
        )
        parameters = ["self", f"value: {annotation}"]
        active_code.extend(
            [
                "",
                bracket(f"def set_{attribute}(", parameters, ")", 4, f" -> {active}:"),
                bracket(
                    "return self.set(",
                    [string(column["name"]), type_.binding()],
                    ")",
                    8,
                ),
            ]
        )
        active_stub.append(
            bracket(f"def set_{attribute}(", parameters, ")", 4, f" -> {active}: ...")
        )
    view = "\n".join(
        [
            f"{model}._active_class = {active}",
            bracket(
                f"{name} = EntityView(", [f"p.entity({registration})", model, active], ")"
            ),
        ]
    )
    code = ["\n".join(model_code), "\n".join(active_code), view]
    stub = [
        "\n".join(model_stub),
        "\n".join(active_stub),
        f"{name}: EntityView[{model}, {active}]",
    ]
    return code, stub


def graph_code(
    entry: dict[str, Any], graph: dict[str, Any], entities: dict[str, dict[str, Any]]
) -> tuple[list[str], list[str]]:
    """A registered graph's row type, its decoder and its view, as the
    module's top-level blocks, and the stub's."""
    types, optional = [], []
    for source in graph["sources"]:
        if source["entity"] not in entities:
            raise CodegenError("graph source is missing from generated entity exports")
        types.append(entities[source["entity"]]["python"] + "Model")
        optional.append(source["slot"] == "Opt")
    name = entry["python"]
    row = name + "Row"
    if len(types) == 1:
        alias = f"{row}: TypeAlias = {types[0]}"
    else:
        slots = [
            type_ + (" | None" if may_be_absent else "")
            for type_, may_be_absent in zip(types, optional)
        ]
        alias = bracket(f"{row}: TypeAlias = tuple[", slots, "]")
    decoded = Call(
        "wrap_row(",
        ["row", flat(tuple(Code(type_) for type_ in types)), flat(tuple(optional))],
        ")",
    )
    decoder = "\n".join(
        [
            f"def _decode_{name}(row: Any) -> {row}:",
            bracket("return cast(", [row, decoded], ")", 4),
        ]
    )
    registration = string(entry["name"])
    view = bracket(f"{name} = GraphView(", [f"p.graph({registration})", f"_decode_{name}"], ")")
    return [alias, decoder, view], [alias, f"{name}: GraphView[{row}]"]


# [spec:pgorm:req:bindings.codegen-disjoint]
def disjoint_code(
    entry: dict[str, Any], disjoint: dict[str, Any], entities: dict[str, dict[str, Any]]
) -> tuple[list[str], list[str]]:
    """A registered disjoint enum's row type, a union over its kinds, and its
    view: a kinded enum's writable, an unkinded one's, with no kind field,
    read-only. The module's top-level blocks, and the stub's."""

    def model(name: str, what: str) -> str:
        if name not in entities:
            raise CodegenError(
                f"the disjoint enum's {what} is missing from generated entity exports"
            )
        return str(entities[name]["python"])

    base = model(disjoint["base"], "base")
    kinds = [
        (kind["value"], kind["model"] and model(kind["model"], "kind"))
        for kind in disjoint["kinds"]
    ]
    name = entry["python"]
    row = name + "Row"
    alias = union(
        f"{row}: TypeAlias = ",
        [
            f"tuple[{base}Model, Literal[{string(value)}], {kind + 'Model' if kind else 'None'}]"
            for value, kind in kinds
        ],
    )
    decoded = {value: Code(kind + "Model" if kind else "None") for value, kind in kinds}
    if disjoint["kind_field"] is None:
        view, arguments, constructor = "DisjointView", [row], "DisjointView"
    else:
        values = ", ".join(string(value) for value, _ in kinds)
        actives = " | ".join(kind + "Active" for _, kind in kinds if kind) or "Never"
        view = "WritableDisjointView"
        arguments = [row, f"Literal[{values}]", f"{base}Active", actives]
        constructor = "WritableDisjointView"
    decoder = "\n".join(
        [
            f"def _decode_{name}(row: Any) -> {row}:",
            bracket(
                "return cast(",
                [row, Call("wrap_disjoint_row(", ["row", f"{base}Model", flat(decoded)], ")")],
                ")",
                4,
            ),
        ]
    )
    registration = string(entry["name"])
    statement = bracket(
        f"{name} = {constructor}(",
        [f"p.disjoint_entity({registration})", f"_decode_{name}"],
        ")",
    )
    return [alias, decoder, statement], [alias, bracket(f"{name}: {view}[", arguments, "]")]


# The names the module and its stub may import, by module, and the
# expression of each that the generated code is searched for.
IMPORTS: list[tuple[str | None, list[tuple[str, str]]]] = [
    ("datetime", [("date", "date"), ("datetime", "datetime"), ("time", "time")]),
    ("decimal", [("Decimal", "Decimal")]),
    (
        "typing",
        [
            ("Any", "Any"),
            ("Literal", "Literal"),
            ("Never", "Never"),
            ("TypeAlias", "TypeAlias"),
            ("cast", "cast"),
        ],
    ),
    ("uuid", [("UUID", "UUID")]),
    (None, [("pgorm as p", "p")]),
    (
        "pgorm._registered",
        [
            ("ActiveView", "ActiveView"),
            ("EntityView", "EntityView"),
            ("GraphView", "GraphView"),
            ("ModelView", "ModelView"),
            ("wrap_row", "wrap_row"),
        ],
    ),
    ("pgorm._registered.compat", [("check as _check", "_check")]),
    (
        "pgorm._registered.disjoints",
        [
            ("DisjointView", "DisjointView"),
            ("WritableDisjointView", "WritableDisjointView"),
            ("wrap_disjoint_row", "wrap_disjoint_row"),
        ],
    ),
]


# [spec:pgorm:req:bindings.codegen-layout]
def import_block(body: str) -> str:
    """The imports ``body`` uses, grouped as isort groups them — the
    standard library, then pgorm — and sorted within each group."""
    standard: list[str] = []
    pgorm: list[str] = []
    for module, names in IMPORTS:
        used = [
            name
            for name, expression in names
            if re.search(rf"(?<![\w.]){re.escape(expression)}\b", body)
        ]
        if not used:
            continue
        group = pgorm if module is None or module.startswith("pgorm") else standard
        if module is None:
            group.extend(f"import {name}" for name in used)
        else:
            group.append(from_import(module, used))
    groups = ["from __future__ import annotations", "\n".join(standard), "\n".join(pgorm)]
    return "\n\n".join(group for group in groups if group)


# [spec:pgorm:req:bindings.codegen]
# [spec:pgorm:req:bindings.codegen-disjoint]
# [spec:pgorm:req:bindings.codegen-layout]
# [spec:pgorm:req:python.codegen+3]
def emit(project: str | PathLike[str]) -> Path:
    from .. import capabilities

    project = Path(project).resolve()
    try:
        description = json.loads((project / "application.json").read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise CodegenError(
            "project must contain a scaffolded application.json"
        ) from error
    raw_keys = {"schema_version", "module", "entity_crate", "entities", "graphs", "disjoints"}
    if not isinstance(description, dict) or set(description) - raw_keys - {
        "entity_package",
        "compatibility",
    }:
        raise CodegenError("invalid scaffold metadata")
    compatibility = description.get("compatibility")
    if not isinstance(compatibility, dict) or set(compatibility) != {
        "package_version",
        "pgorm_version",
        "registry_abi",
    }:
        raise CodegenError("invalid scaffold compatibility metadata")
    description = {
        **validate(
            {key: value for key, value in description.items() if key in raw_keys},
            project,
        ),
        "compatibility": compatibility,
    }
    actual = capabilities()
    expected = description["compatibility"]
    if (
        any(
            actual[key] != expected[key] for key in ("package_version", "pgorm_version")
        )
        or actual["binding"]["registry_abi"] != expected["registry_abi"]
    ):
        raise CodegenError("installed native build is incompatible with this scaffold")
    expected = {**expected, "features": actual["features"]}
    registrations = {}
    for family in ("entities", "graphs", "disjoints"):
        registrations[family] = {
            entry["name"]: entry for entry in actual["registrations"][family]
        }
        requested = [entry["name"] for entry in description[family]]
        if set(requested) != set(registrations[family]):
            raise CodegenError(
                "install the scaffold's native wheel before emitting its concrete wrappers"
            )
        expected[family] = [registrations[family][name] for name in requested]
    code: list[str] = [
        f"_COMPATIBILITY = {literal(expected, 0, len('_COMPATIBILITY = '))}\n_check(_COMPATIBILITY)"
    ]
    stubs: list[str] = []
    names = []
    entities = {entry["name"]: entry for entry in description["entities"]}
    for family in ("entities", "graphs", "disjoints"):
        for entry in description[family]:
            registered = registrations[family][entry["name"]]
            if family == "entities":
                generated, stub = entity_code(entry, registered)
                names.extend(
                    [entry["python"], entry["python"] + "Model", entry["python"] + "Active"]
                )
            else:
                generated, stub = (graph_code if family == "graphs" else disjoint_code)(
                    entry, registered, entities
                )
                names.extend([entry["python"], entry["python"] + "Row"])
            code.extend(generated)
            stubs.extend(stub)
    code.append(f"__all__ = {literal(names, 0, len('__all__ = '))}")
    code_body = "\n\n\n".join(code)
    stub_body = "\n\n".join(stubs)
    code_text = "\n".join(
        [
            "# Generated by pgorm.codegen; regenerate from the compiled registry.",
            import_block(code_body),
            "",
            "",
            code_body,
            "",
        ]
    )
    stub_text = "\n".join(
        [
            "# Generated by pgorm.codegen from the compiled application registry.",
            import_block(stub_body),
            "",
            stub_body,
            "",
        ]
    )
    ast.parse(code_text)
    ast.parse(stub_text)
    module = identifier(description["module"], "application module")
    destination = project / "python/pgorm"
    if not destination.is_dir():
        raise CodegenError("scaffold's Python package directory is missing")
    existing = any(
        (destination / (module + suffix)).exists() for suffix in ("", ".py", ".pyi")
    )
    if existing:
        try:
            previous = json.loads(
                (destination / "_application_module.json").read_text()
            )["module"]
        except (OSError, KeyError, json.JSONDecodeError) as error:
            raise CodegenError(
                "application module would overwrite an existing Python API"
            ) from error
        if previous != module:
            raise CodegenError("application module would overwrite another Python API")
    (destination / (module + ".py")).write_text(code_text)
    (destination / (module + ".pyi")).write_text(stub_text)
    (destination / "_application_module.json").write_text(
        json.dumps({"module": module}) + "\n"
    )
    return destination / (module + ".py")
