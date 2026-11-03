"""Build an application wheel and verify native entity and graph registrations."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def run(command, environment):
    subprocess.run(command, env=environment, check=True)


def materialize(root, destination):
    fixture = root / "pgorm-python/tests/application-binding"
    shutil.copytree(fixture / "src", destination / "src")
    shutil.copytree(
        root / "pgorm-python/python",
        destination / "python",
        ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
    )
    manifest = (fixture / "Cargo.toml").read_text()
    manifest = manifest.replace(
        'path = "../.."', "path = " + json.dumps(str(root / "pgorm-python"))
    )
    manifest = manifest.replace('path = "../../.."', "path = " + json.dumps(str(root)))
    (destination / "Cargo.toml").write_text(manifest)
    shutil.copyfile(fixture / "Cargo.lock", destination / "Cargo.lock")
    shutil.copyfile(fixture / "pyproject.toml", destination / "pyproject.toml")


def reject_execution_mismatch(project, output, environment):
    report = json.loads((output / "execution.json").read_text())
    report["connection"]["runtime"]["inserted"] += 1
    changed = output / "execution-mismatch.json"
    changed.write_text(json.dumps(report))
    result = subprocess.run(
        [
            "cargo",
            "run",
            "--manifest-path",
            str(project / "Cargo.toml"),
            "--locked",
            "--bin",
            "execution_oracle",
        ],
        capture_output=True,
        text=True,
        env={**environment, "PGORM_EXECUTION_REPORT": str(changed)},
    )
    changed.unlink()
    if result.returncode == 0 or "differs from native Rust" not in result.stderr:
        raise RuntimeError(
            "execution oracle did not reject the deliberately incorrect Python outcome"
        )


# [spec:pgorm:req:bindings.entities+4/test]
# [spec:pgorm:req:bindings.entity-graphs+3/test]
# [spec:pgorm:req:bindings.pipeline-sources+3/test]
# [spec:pgorm:req:bindings.surface+2/test]
# [spec:pgorm:req:bindings.disjoint+1/test]
# [spec:pgorm:req:bindings.disjoint-unkinded/test]
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("target/python-entities"))
    options = parser.parse_args()
    if not os.environ.get("PGORM_TEST_DSN"):
        parser.error("PGORM_TEST_DSN is required; run through tests/with_postgres.py")
    root = Path(__file__).resolve().parents[2]
    output = options.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    (output / "summary.json").write_text('{"passed": false, "status": "running"}\n')
    environment = {
        **os.environ,
        "PYO3_PYTHON": sys.executable,
        "CARGO_TARGET_DIR": str(root / "target"),
        "PYTHONNOUSERSITE": "1",
    }
    environment.pop("PYTHONPATH", None)
    environment["PGORM_GRAPH_REPORT"] = str(output / "cursor-queries.json")
    environment["PGORM_EXECUTION_REPORT"] = str(output / "execution.json")
    (output / "cursor-queries.json").unlink(missing_ok=True)
    (output / "execution.json").unlink(missing_ok=True)
    with tempfile.TemporaryDirectory(prefix="pgorm-entity-project-") as directory:
        project = Path(directory)
        materialize(root, project)
        run(
            [
                "cargo",
                "test",
                "--manifest-path",
                str(project / "Cargo.toml"),
                "--locked",
                "--lib",
            ],
            environment,
        )
        run(
            [
                sys.executable,
                "-m",
                "maturin",
                "build",
                "--manifest-path",
                str(project / "Cargo.toml"),
                "--interpreter",
                sys.executable,
                "--out",
                str(project / "dist"),
                "--locked",
            ],
            environment,
        )
        wheels = list((project / "dist").glob("pgorm-*.whl"))
        if len(wheels) != 1:
            raise RuntimeError("expected one downstream application wheel")
        wheel = output / wheels[0].name
        shutil.copyfile(wheels[0], wheel)
        run(
            ["uv", "venv", "--python", sys.executable, str(project / "venv")],
            environment,
        )
        python = (
            project
            / "venv"
            / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        )
        run(
            [
                "uv",
                "pip",
                "install",
                "--no-index",
                "--no-cache",
                "--python",
                str(python),
                str(wheel),
            ],
            environment,
        )
        run(
            [
                str(python),
                "-I",
                str(root / "pgorm-python/tests/registered_entities.py"),
                "-v",
            ],
            environment,
        )
        # The surface test's registered-entity descriptions run only against
        # a module that registers entities.
        run(
            [
                str(python),
                "-I",
                "-m",
                "unittest",
                "discover",
                "-s",
                str(root / "pgorm-python/tests"),
                "-p",
                "test_surface.py",
                "-v",
            ],
            environment,
        )
        run(
            [
                str(python),
                "-I",
                "-m",
                "unittest",
                "discover",
                "-s",
                str(root / "pgorm-python/tests"),
                "-p",
                "registered_graphs.py",
                "-v",
            ],
            environment,
        )
        run(
            [
                "cargo",
                "run",
                "--manifest-path",
                str(project / "Cargo.toml"),
                "--locked",
                "--bin",
                "graph_oracle",
            ],
            environment,
        )
        run(
            [
                str(python),
                "-I",
                "-m",
                "unittest",
                "discover",
                "-s",
                str(root / "pgorm-python/tests"),
                "-p",
                "registered_sources.py",
                "-v",
            ],
            environment,
        )
        run(
            [
                str(python),
                "-I",
                str(root / "pgorm-python/tests/registered_temporal.py"),
                "-v",
            ],
            environment,
        )
        run(
            [
                str(python),
                "-I",
                str(root / "pgorm-python/tests/registered_disjoint.py"),
                "-v",
            ],
            environment,
        )
        run(
            [
                str(python),
                "-I",
                str(root / "pgorm-python/tests/registered_versions.py"),
                "-v",
            ],
            environment,
        )
        run(
            [
                str(python),
                "-I",
                str(root / "pgorm-python/tests/registered_schema.py"),
                "-v",
            ],
            environment,
        )
        run(
            [
                str(python),
                "-I",
                str(root / "pgorm-python/tests/registered_transactions.py"),
                "-v",
            ],
            environment,
        )
        run(
            [
                str(python),
                "-I",
                str(root / "pgorm-python/tests/registered_pools.py"),
                "-v",
            ],
            environment,
        )
        run(
            [
                str(python),
                "-I",
                str(root / "pgorm-python/tests/registered_execution.py"),
            ],
            environment,
        )
        run(
            [
                "cargo",
                "run",
                "--manifest-path",
                str(project / "Cargo.toml"),
                "--locked",
                "--bin",
                "execution_oracle",
            ],
            environment,
        )
        reject_execution_mismatch(project, output, environment)
    (output / "summary.json").write_text(
        json.dumps(
            {
                "passed": True,
                "registered_entities": [
                    "app.Account",
                    "app.Auction",
                    "app.Booking",
                    "app.Listing",
                    "app.Membership",
                    "app.Note",
                    "app.Old",
                    "app.Pallet",
                    "app.Parcel",
                    "app.Room",
                    "app.Sale",
                    "app.Shipment",
                    "app.Stay",
                    "app.Temper",
                    "app.Till",
                ],
                "registered_disjoints": ["app.Listings", "app.Shipments"],
                "graph_source_arities": list(range(1, 8)),
                "pipeline_source_arities": list(range(1, 7)),
                "rust_cursor_parity_cases": 8,
                "registered_temporal": True,
                "registered_versions": True,
                "registered_schema": True,
                "registered_transactions": True,
                "registered_pools": True,
                "rust_execution_parity": True,
                "execution_mismatch_rejected": True,
                "wheel": wheel.name,
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
