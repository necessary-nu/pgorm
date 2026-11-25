"""Inspect wheel/source contents: python check_artifacts.py ARTIFACT_DIR."""

from pathlib import Path, PurePosixPath
import json
import posixpath
import sys
import tarfile
import tomllib
import zipfile


# The binding's own lockfile in this checkout, which the source distribution
# has to carry: Cargo.toml asks for a compatible PyO3, the lockfile fixes one.
CHECKOUT_LOCK = tomllib.loads(
    (Path(__file__).resolve().parents[1] / "Cargo.lock").read_text()
)["package"]


def pyo3_releases(packages: list[dict]) -> list[str]:
    return sorted(p["version"] for p in packages if p["name"] == "pyo3")


# [spec:pgorm:req:bindings.optional/test]
# [spec:pgorm:req:python.package/test]
# [spec:pgorm:req:python.distribution/test]
# [spec:pgorm:req:turso.python+1/test]
def check_source(path: Path) -> None:
    with tarfile.open(path) as source:
        names = source.getnames()
        root = names[0].split("/")[0]
        forbidden = {"security", "tests_cfg", ".nplan", ".codex", "plan"}
        for name in names:
            assert not forbidden.intersection(PurePosixPath(name).parts), name
            assert "__pycache__" not in PurePosixPath(name).parts and not name.endswith(
                ".pyc"
            ), name
            assert "this-session-is-being-continued" not in name, name
        project_file = source.extractfile(f"{root}/pyproject.toml")
        assert project_file is not None
        project = tomllib.loads(project_file.read().decode())
        native_manifest = project["tool"]["maturin"]["manifest-path"]
        native_root = PurePosixPath(native_manifest).parent
        assert f"{root}/{native_root}/src/lib.rs" in names
        native_file = source.extractfile(f"{root}/{native_manifest}")
        assert native_file is not None
        native = tomllib.loads(native_file.read().decode())
        assert {"pgorm/turso", "pgorm-bindings/turso", "pgorm-driver/turso"} <= set(native["features"]["turso"])
        driver = posixpath.normpath(f"{native_root}/{native['dependencies']['pgorm-driver']['path']}")
        for file in ("src/config/endpoint.rs", "src/native/worker.rs", "src/native/codecs.rs"):
            assert f"{root}/{driver}/{file}" in names
        lock_file = source.extractfile(f"{root}/{native_root}/Cargo.lock")
        assert lock_file is not None
        packages = tomllib.loads(lock_file.read().decode())["package"]
        assert pyo3_releases(packages) == pyo3_releases(CHECKOUT_LOCK), packages
        native = [package for package in packages if package["name"] == "turso_pg"]
        expected = [package for package in CHECKOUT_LOCK if package["name"] == "turso_pg"]
        assert native == expected and len(native) == 1, native
        assert any(name.endswith("/examples/application.py") for name in names)
        assert any(name.endswith("/licenses/supplemental.json") for name in names)
        assert any(name.endswith("/pgorm/THIRD_PARTY_NOTICES.txt") for name in names)


# [spec:pgorm:req:bindings.optional/test]
# [spec:pgorm:req:python.package/test]
# [spec:pgorm:req:python.distribution/test]
# [spec:pgorm:req:turso.python+1/test]
def check_wheel(path: Path) -> None:
    with zipfile.ZipFile(path) as wheel:
        names = wheel.namelist()
        assert not any("__pycache__" in name or name.endswith(".pyc") for name in names)
        assert "pgorm/__init__.py" in names
        assert "pgorm/py.typed" in names
        assert any(
            n.startswith("pgorm/_native.") and n.endswith((".so", ".pyd"))
            for n in names
        )
        assert any(n.endswith("/LICENSE-MIT") for n in names)
        assert any(n.endswith("/LICENSE-APACHE") for n in names)
        assert not any(
            "security/" in n or "tests_cfg/" in n or "fixtures/" in n for n in names
        )
        dependencies = json.loads(wheel.read("pgorm/DEPENDENCIES.json"))
        assert dependencies["python_runtime_dependencies"] == []
        notices = wheel.read("pgorm/THIRD_PARTY_NOTICES.txt").decode()
        assert dependencies["packages"]
        for package in dependencies["packages"]:
            assert package["license"] and package["notices"], package["name"]
            for notice in package["notices"]:
                assert notice["sha256"] in notices
        parser = {
            package["name"]: package
            for package in dependencies["packages"]
            if package["source"].startswith("git+https://github.com/necessary-nu/pgsql-rs.git")
        }
        assert {"pgsql-syntax", "pgsql-parser", "pg19-parser"} <= parser.keys()
        for package in parser.values():
            sources = {notice["source"] for notice in package["notices"]}
            assert {"pgsql-rs/LICENSE", "pgsql-rs/LICENSE-PostgreSQL"} <= sources
        assert dependencies["optional_features"] == ["turso"]
        engine = [package for package in dependencies["packages"]
                  if package["source"].startswith("git+https://github.com/tursodatabase/turso")]
        assert any(package["name"] == "turso_pg" for package in engine)
        for package in engine:
            assert "turso/LICENSE.md" in {notice["source"] for notice in package["notices"]}
        metadata = wheel.read(
            next(name for name in names if name.endswith(".dist-info/METADATA"))
        ).decode()
        assert "License-Expression: MIT OR Apache-2.0" in metadata
        assert "License-File:" in metadata and "THIRD_PARTY_NOTICES.txt" in metadata


if __name__ == "__main__":
    directory = Path(sys.argv[1])
    sources = sorted(directory.glob("pgorm-*.tar.gz"))
    wheels = sorted(directory.glob("pgorm-*.whl"))
    assert sources, "source distribution is missing"
    assert wheels, "wheel is missing"
    for artifact in sources:
        check_source(artifact)
    for artifact in wheels:
        check_wheel(artifact)
    print(
        f"Validated {len(sources)} source distribution(s) and {len(wheels)} wheel(s)."
    )
