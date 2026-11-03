"""Emit concrete wrappers from the installed, compiled application registry."""

import ast
import json
import keyword
from pathlib import Path
from os import PathLike
from typing import Any

from .config import CodegenError, identifier, validate
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
    name = entry["python"]
    model, active = name + "Model", name + "Active"
    fields = attributes(entity, entry)
    code = [
        f'class {model}(ModelView["{active}"]):',
        f"    _entity_name = {entry['name']!r}",
    ]
    stub = [f"class {model}(ModelView[{active}]):"]
    for column, attribute, type_ in fields:
        code.extend(
            [
                "",
                "    @property",
                f"    def {attribute}(self) -> {type_.annotation}:",
                f"        return cast({type_.annotation}, self[{column['name']!r}])",
            ]
        )
        stub.extend(
            ["    @property", f"    def {attribute}(self) -> {type_.annotation}: ..."]
        )
    code.extend(
        [
            "",
            f"class {active}(ActiveView[{model}]):",
            f"    _entity_name = {entry['name']!r}",
            f"    _model_class = {model}",
        ]
    )
    stub.extend(["", f"class {active}(ActiveView[{model}]):"])
    for column, attribute, type_ in fields:
        annotation = (
            f"{type_.annotation} | p.Value" if type_.annotation != "Any" else "Any"
        )
        signature = f"    def set_{attribute}(self, value: {annotation}) -> {active}"
        code.extend(
            [
                "",
                signature + ":",
                f"        return self.set({column['name']!r}, {type_.binding()})",
            ]
        )
        stub.append(signature + ": ...")
    code.extend(
        [
            "",
            f"{model}._active_class = {active}",
            f"{name}: EntityView[{model}, {active}] = EntityView(p.entity({entry['name']!r}), {model}, {active})",
            "",
        ]
    )
    stub.extend(["", f"{name}: EntityView[{model}, {active}]", ""])
    return code, stub


def graph_code(
    entry: dict[str, Any], graph: dict[str, Any], entities: dict[str, dict[str, Any]]
) -> tuple[list[str], list[str]]:
    types, optional = [], []
    for source in graph["sources"]:
        if source["entity"] not in entities:
            raise CodegenError("graph source is missing from generated entity exports")
        types.append(entities[source["entity"]]["python"] + "Model")
        optional.append(source["slot"] == "Opt")
    name = entry["python"]
    row = name + "Row"
    shape = (
        types[0]
        if len(types) == 1
        else "tuple["
        + ", ".join(
            type_ + (" | None" if may_be_absent else "")
            for type_, may_be_absent in zip(types, optional)
        )
        + "]"
    )
    models = "(" + ", ".join(types) + ",)"
    code = [
        f"{row}: TypeAlias = {shape}",
        "",
        f"def _decode_{name}(row: Any) -> {row}:",
        f"    return cast({row}, wrap_row(row, {models}, {tuple(optional)!r}))",
        "",
        f"{name}: GraphView[{row}] = GraphView(p.graph({entry['name']!r}), _decode_{name})",
        "",
    ]
    stub = [f"{row}: TypeAlias = {shape}", f"{name}: GraphView[{row}]", ""]
    return code, stub


# [spec:pgorm:req:bindings.codegen-disjoint]
def disjoint_code(
    entry: dict[str, Any], disjoint: dict[str, Any], entities: dict[str, dict[str, Any]]
) -> tuple[list[str], list[str]]:
    """A registered disjoint enum's row type, a union over its kinds, and its
    view: a kinded enum's writable, an unkinded one's, with no kind field,
    read-only."""

    def model(name: str, what: str) -> str:
        if name not in entities:
            raise CodegenError(f"the disjoint enum's {what} is missing from generated entity exports")
        return str(entities[name]["python"])

    base = model(disjoint["base"], "base")
    kinds = [(kind["value"], kind["model"] and model(kind["model"], "kind")) for kind in disjoint["kinds"]]
    name = entry["python"]
    row = name + "Row"
    shape = " | ".join(
        f"tuple[{base}Model, Literal[{value!r}], {kind + 'Model' if kind else 'None'}]"
        for value, kind in kinds
    )
    decoded = "{" + ", ".join(
        f"{value!r}: {kind + 'Model' if kind else 'None'}" for value, kind in kinds
    ) + "}"
    if disjoint["kind_field"] is None:
        view = f"DisjointView[{row}]"
        constructor = "DisjointView"
    else:
        values = ", ".join(repr(value) for value, _ in kinds)
        actives = " | ".join(kind + "Active" for _, kind in kinds if kind) or "Never"
        view = f"WritableDisjointView[{row}, Literal[{values}], {base}Active, {actives}]"
        constructor = "WritableDisjointView"
    code = [
        f"{row}: TypeAlias = {shape}",
        "",
        f"def _decode_{name}(row: Any) -> {row}:",
        f"    return cast({row}, wrap_disjoint_row(row, {base}Model, {decoded}))",
        "",
        f"{name}: {view} = {constructor}(p.disjoint_entity({entry['name']!r}), _decode_{name})",
        "",
    ]
    stub = [f"{row}: TypeAlias = {shape}", f"{name}: {view}", ""]
    return code, stub


# [spec:pgorm:req:bindings.codegen]
# [spec:pgorm:req:bindings.codegen-disjoint]
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
    imports = [
        "from __future__ import annotations",
        "",
        "from datetime import date, datetime, time",
        "from decimal import Decimal",
        "from uuid import UUID",
        "from typing import Any, Literal, Never, TypeAlias, cast",
        "import pgorm as p",
        "from pgorm._registered import ActiveView, EntityView, GraphView, ModelView, wrap_row",
        "from pgorm._registered.disjoints import DisjointView, WritableDisjointView, wrap_disjoint_row",
        "",
    ]
    code = [
        "# Generated by pgorm.codegen; regenerate from the compiled registry.",
        *imports,
        "from pgorm._registered.compat import check as _check",
        f"_COMPATIBILITY = {expected!r}",
        "_check(_COMPATIBILITY)",
        "",
    ]
    stubs = [
        "# Generated by pgorm.codegen from the compiled application registry.",
        *imports,
    ]
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
    code.append(f"__all__ = {names!r}\n")
    code_text = "\n".join(code)
    stub_text = "\n".join(stubs)
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
