"""Verify the complete native Python API using installed artifacts and Rust oracles."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def run(command, environment, log):
    print("Running " + log.stem, flush=True)
    with log.open("w") as output:
        result = subprocess.run(
            [str(part) for part in command],
            env=environment,
            stdout=output,
            stderr=subprocess.STDOUT,
        )
    if result.returncode:
        print("\n".join(log.read_text().splitlines()[-50:]), file=sys.stderr)
        raise RuntimeError(f"{log.stem} failed; see {log}")


def rust_without_python(root, output, environment):
    graph = json.loads(
        subprocess.check_output(
            [
                "cargo",
                "metadata",
                "--locked",
                "--format-version",
                "1",
                "--manifest-path",
                str(root / "Cargo.toml"),
            ],
            env=environment,
            text=True,
        )
    )
    packages = {package["name"] for package in graph["packages"]}
    if "pgorm-python" in packages or any(name.startswith("pyo3") for name in packages):
        raise RuntimeError(
            "default Rust workspace resolves Python binding dependencies"
        )
    with tempfile.TemporaryDirectory(prefix="pgorm-no-python-") as temporary:
        directory = Path(temporary)
        log = directory / "invoked"
        for name in ("python", "python3", "python3.14", "pip", "pip3", "maturin"):
            path = directory / name
            path.write_text(
                "#!/bin/sh\n"
                'printf "%s\\t%s\\t%s\\t%s\\n" "${0##*/}" "${CARGO_PKG_NAME-}" '
                '"${CARGO_MANIFEST_DIR-}" "$*" >> "$PGORM_PYTHON_TOOL_LOG"\n'
                "exit 97\n"
            )
            path.chmod(0o755)
        blocked = {
            **{
                key: value
                for key, value in environment.items()
                if key != "CARGO_MANIFEST_DIR" and not key.startswith("CARGO_PKG_")
            },
            "PATH": str(directory) + os.pathsep + environment["PATH"],
            "PGORM_PYTHON_TOOL_LOG": str(log),
            "PYO3_PYTHON": str(directory / "python"),
            "PYTHON_SYS_EXECUTABLE": str(directory / "python"),
        }
        run(
            [
                "cargo",
                "build",
                "--workspace",
                "--locked",
                "--manifest-path",
                root / "Cargo.toml",
            ],
            blocked,
            output / "rust-without-python.log",
        )
        workspace, dependencies = python_invocations(log, graph)
        if workspace:
            raise RuntimeError(
                "default Rust build invoked Python tooling: " + json.dumps(workspace)
            )
    return {
        "build_passed": True,
        "python_dependencies": False,
        "python_tools_invoked_by_workspace": False,
        "dependency_python_probes": dependencies,
    }


def python_invocations(log, graph):
    """Split the stubs' records into pgorm's own invocations and dependencies' probes.

    Cargo gives every build script and compiler run the CARGO_MANIFEST_DIR of
    the package it builds, and a tool either one launches inherits it, so each
    record names the package whose build step reached Python. A dependency's
    record is a probe the build survived: the stub failed as Python would on a
    host without it, and the build still passed, so that dependency builds the
    same way there. libc's build script is one: it runs `emcc -dumpversion` on
    every host, and Homebrew's emcc is a Python program. Every other record, a
    workspace package's or one no package can be named for, is pgorm's build
    reaching for Python.
    """
    members = set(graph["workspace_members"])
    dependencies = {
        Path(package["manifest_path"]).resolve().parent
        for package in graph["packages"]
        if package["id"] not in members
    }
    workspace, probes = [], []
    for line in log.read_text().splitlines() if log.exists() else []:
        fields = line.split("\t", 3)
        if len(fields) != 4:
            workspace.append({"record": line})
            continue
        tool, package, directory, arguments = fields
        record = {"tool": tool, "package": package or None, "arguments": arguments}
        if directory and Path(directory).resolve() in dependencies:
            probes.append(record)
        else:
            workspace.append(record)
    return workspace, probes


def phase(root, name, output, environment):
    destination = output / name
    run(
        [
            sys.executable,
            root / f"pgorm-python/checks/{name}.py",
            "--output",
            destination,
        ],
        environment,
        output / (name + ".log"),
    )
    report = destination / "summary.json"
    value = json.loads(report.read_text())
    if value.get("passed") is not True:
        raise RuntimeError(f"{name} did not record passing evidence")
    return {
        "report": str(report.relative_to(output)),
        "sha256": hashlib.sha256(report.read_bytes()).hexdigest(),
    }


# [spec:pgorm:req:python.acceptance+1]
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("target/python-acceptance"))
    options = parser.parse_args()
    if not all(os.environ.get(name) for name in ("PGORM_TEST_DSN", "PGORM_TEST_CA")):
        parser.error(
            "PGORM_TEST_DSN and PGORM_TEST_CA are required; use tests/with_local_postgres.py or with_postgres.py"
        )
    root = Path(__file__).resolve().parents[2]
    output = options.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = output / "summary.json"
    report.write_text('{"passed": false, "status": "running"}\n')
    environment = {
        **os.environ,
        "PYO3_PYTHON": sys.executable,
        "CARGO_TARGET_DIR": str(root / "target"),
        "PYTHONNOUSERSITE": "1",
    }
    for name in ("PYTHONPATH", "MYPYPATH"):
        environment.pop(name, None)
    rust = rust_without_python(root, output, environment)
    outcomes = {"direct_builders": phase(root, "direct_builders", output, environment)}
    environment["PGORM_DIRECT_REPORT"] = str(output / "direct_builders/queries.json")
    run(
        [
            "cargo",
            "test",
            "--manifest-path",
            root / "pgorm-python/Cargo.toml",
            "--locked",
            "--lib",
            "--tests",
        ],
        environment,
        output / "native-parity.log",
    )
    # Later test runs must not overwrite the direct builder oracle's recorded report.
    environment.pop("PGORM_DIRECT_REPORT")
    for name in ("entities", "codegen", "guide", "distribution"):
        outcomes[name] = phase(root, name, output, environment)
    run(["nplan", "spec", "validate"], environment, output / "nspec.log")
    summary = {
        "passed": True,
        "rust_without_python": rust,
        "native_parity": True,
        "nspec_valid": True,
        "phases": outcomes,
        "transport": "in-process",
    }
    report.write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
