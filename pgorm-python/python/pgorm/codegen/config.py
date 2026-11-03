"""Validated build inputs for a concrete application binding crate."""

from copy import deepcopy
import builtins
import keyword
from pathlib import Path
from os import PathLike
import re
import tomllib
from typing import Any
import unicodedata


class CodegenError(ValueError):
    """The application build description is invalid or incompatible."""


def identifier(value: object, what: str) -> str:
    if isinstance(value, str):
        value = unicodedata.normalize("NFKC", value)
    if (
        not isinstance(value, str)
        or not value.isidentifier()
        or keyword.iskeyword(value)
        or value.startswith("_")
    ):
        raise CodegenError(f"{what} must be a public Python identifier")
    return value


def rust_path(value: object) -> str:
    if not isinstance(value, str) or not re.fullmatch(
        r"(?:r#)?[A-Za-z_][A-Za-z0-9_]*(?:::(?:r#)?[A-Za-z_][A-Za-z0-9_]*)*", value
    ):
        raise CodegenError(
            "Rust paths require named modules and items, without expressions or generics"
        )
    special = {"crate", "self", "super", "Self"}
    if any(part.removeprefix("r#") in special for part in value.split("::")):
        raise CodegenError("Rust paths are relative to the application entity crate")
    keywords = {
        "as",
        "async",
        "await",
        "break",
        "const",
        "continue",
        "dyn",
        "else",
        "enum",
        "extern",
        "false",
        "fn",
        "for",
        "gen",
        "if",
        "impl",
        "in",
        "let",
        "loop",
        "match",
        "mod",
        "move",
        "mut",
        "pub",
        "ref",
        "return",
        "static",
        "struct",
        "trait",
        "true",
        "type",
        "unsafe",
        "use",
        "where",
        "while",
        "yield",
        "try",
    }
    if any(part in keywords for part in value.split("::")):
        raise CodegenError("Rust keyword item names require raw identifiers")
    return value


def registration(value: object) -> str:
    if (
        not isinstance(value, str)
        or not value
        or "\0" in value
        or len(value.encode("utf-8")) > 255
    ):
        raise CodegenError("registration names require 1–255 UTF-8 bytes without NUL")
    return value


def validate(data: object, base: str | PathLike[str]) -> dict[str, Any]:
    if not isinstance(data, dict) or set(data) - {
        "schema_version",
        "module",
        "entity_crate",
        "entities",
        "graphs",
        "disjoints",
    }:
        raise CodegenError("unknown or invalid application configuration")
    if type(data.get("schema_version")) is not int or data["schema_version"] != 1:
        raise CodegenError("application configuration requires schema_version 1")
    result: dict[str, Any] = deepcopy(data)
    result["module"] = identifier(data.get("module", "app"), "application module")
    crate = data.get("entity_crate")
    if not isinstance(crate, str):
        raise CodegenError(
            "entity_crate must name the directory of an existing Rust package"
        )
    crate_path = (Path(base) / crate).resolve()
    try:
        manifest = tomllib.loads((crate_path / "Cargo.toml").read_text())
        package = manifest["package"]["name"]
    except (OSError, KeyError, tomllib.TOMLDecodeError) as error:
        raise CodegenError("entity_crate must contain a Cargo.toml package") from error
    if not isinstance(package, str) or not re.fullmatch(r"[A-Za-z0-9_-]+", package):
        raise CodegenError("invalid application Rust package name")
    result["entity_crate"] = str(crate_path)
    result["entity_package"] = package
    exports = set(vars(builtins)) | {
        "p",
        "Any",
        "TypeAlias",
        "cast",
        "date",
        "datetime",
        "time",
        "Decimal",
        "UUID",
        "ModelView",
        "ActiveView",
        "EntityView",
        "GraphView",
        "wrap_row",
        "DisjointView",
        "WritableDisjointView",
        "wrap_disjoint_row",
        "Literal",
        "Never",
    }
    # [spec:pgorm:req:bindings.codegen-disjoint]
    for family in ("entities", "graphs", "disjoints"):
        entries = data.get(family, [])
        if not isinstance(entries, list):
            raise CodegenError(f"{family} must be a list")
        names = set()
        types = set()
        normalized = []
        for entry in entries:
            allowed = (
                {"name", "rust", "python", "fields"}
                if family == "entities"
                else {"name", "rust", "python"}
            )
            if not isinstance(entry, dict) or set(entry) - allowed:
                raise CodegenError(f"invalid {family} entry")
            name = registration(entry.get("name"))
            path = rust_path(entry.get("rust"))
            export = identifier(entry.get("python"), "registration export")
            symbols = (
                {export, export + "Model", export + "Active"}
                if family == "entities"
                else {export, export + "Row"}
            )
            if (
                name in names
                or exports.intersection(symbols)
                or (family != "graphs" and path in types)
            ):
                raise CodegenError(
                    "registration names, entity types and Python exports must be unique"
                )
            names.add(name)
            types.add(path)
            exports.update(symbols)
            current: dict[str, Any] = {"name": name, "rust": path, "python": export}
            if family == "entities":
                fields = entry.get("fields", {})
                if not isinstance(fields, dict) or not all(
                    isinstance(key, str) for key in fields
                ):
                    raise CodegenError(
                        "field aliases must map SQL names to Python attributes"
                    )
                current["fields"] = {
                    key: identifier(value, "field alias")
                    for key, value in fields.items()
                }
            normalized.append(current)
        result[family] = normalized
    if not result["entities"]:
        raise CodegenError("register at least one application entity")
    return result


def rust_string(value: str) -> str:
    escapes = {"\\": "\\\\", '"': '\\"', "\n": "\\n", "\r": "\\r", "\t": "\\t"}
    return (
        '"'
        + "".join(
            escapes.get(
                char,
                f"\\u{{{ord(char):x}}}" if ord(char) < 32 or ord(char) == 127 else char,
            )
            for char in value
        )
        + '"'
    )
