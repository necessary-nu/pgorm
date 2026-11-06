"""Check generated declarations against the extension loaded into this process."""

from .. import _native as native
from typing import Any


def check(expected: dict[str, Any]) -> None:
    actual = native.capabilities()
    for key in ("package_version", "pgorm_version"):
        if actual[key] != expected[key]:
            raise native.UnsupportedCapabilityError(
                "generated registrations require another pgorm build"
            )
    if actual["binding"]["registry_abi"] != expected["registry_abi"]:
        raise native.UnsupportedCapabilityError(
            "generated registrations require another binding registry ABI"
        )
    if actual["features"] != expected["features"]:
        raise native.UnsupportedCapabilityError(
            "generated registrations require another pgorm feature set"
        )
    # [spec:pgorm:req:bindings.codegen-disjoint]
    # [spec:pgorm:req:bindings.codegen-coverage]
    for family in ("entities", "graphs", "disjoints", "one_ofs", "sources"):
        installed = {entry["name"]: entry for entry in actual["registrations"][family]}
        for entry in expected.get(family, ()):
            if installed.get(entry["name"]) != entry:
                raise native.UnsupportedCapabilityError(
                    f"generated {family} declarations differ from the installed registry"
                )
