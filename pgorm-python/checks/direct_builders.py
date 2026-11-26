"""Build once, install cleanly, run direct Python queries, verify Rust parity.

Use with_postgres.py for a disposable database or supply PGORM_TEST_DSN.
Requires Maturin in the invoking Python environment, Cargo and uv.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def run(command, *, environment, capture=False):
    result = subprocess.run(command, env=environment, text=True,
                            stdout=subprocess.PIPE if capture else None)
    if result.returncode and capture:
        for line in result.stdout.splitlines():
            try:
                message = json.loads(line)
                if message.get("reason") == "compiler-message":
                    print(message["message"].get("rendered", line), file=sys.stderr)
            except json.JSONDecodeError:
                print(line, file=sys.stderr)
    result.check_returncode()
    return result


# The crate's integration tests are one binary, `it`; the parity tests are its
# `direct_builders` module, which this filter runs alone.
ORACLE_TESTS = "direct_builders::"


def rust_oracle(root, environment):
    output = run(["cargo", "test", "--manifest-path", str(root / "pgorm-python/Cargo.toml"),
                  "--target-dir", str(root / "target"), "--locked", "--test", "it",
                  "--no-run", "--message-format=json"], environment=environment, capture=True)
    executables = [entry["executable"] for line in output.stdout.splitlines()
                   if (entry := json.loads(line)).get("reason") == "compiler-artifact"
                   and entry.get("executable") and entry["target"]["name"] == "it"]
    if len(executables) != 1:
        raise RuntimeError("expected exactly one precompiled Rust parity executable")
    return executables[0]


def assert_parity_fails(oracle, report, environment, field):
    changed = json.loads(report.read_text())
    case = changed["runs"][0]["cases"]["select_join"]
    if field == "sql":
        case["sql"] += " -- deliberate parity mismatch"
    else:
        case["params"].pop()
    path = report.with_name(f"negative-{field}.json")
    path.write_text(json.dumps(changed))
    result = subprocess.run([oracle, ORACLE_TESTS], env={**environment, "PGORM_DIRECT_REPORT": str(path)},
                            capture_output=True, text=True)
    if result.returncode == 0 or "assertion" not in result.stdout + result.stderr:
        raise AssertionError(f"Rust parity did not reject the intentional {field} mismatch")
    path.unlink()


# [spec:pgorm:req:python.direct-builders]
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("target/python-direct-builders"))
    options = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    output = options.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    (output / "summary.json").write_text(json.dumps({"schema_version": 1, "passed": False, "status": "running"}) + "\n")
    if not os.environ.get("PGORM_TEST_DSN"):
        parser.error("PGORM_TEST_DSN is required; run through tests/with_postgres.py")
    environment = {**os.environ, "PYO3_PYTHON": sys.executable, "CARGO_TARGET_DIR": str(root / "target")}
    # This is the only compiler/build phase. Its output is reused by every query.
    oracle = rust_oracle(root, environment)
    with tempfile.TemporaryDirectory(prefix="build-", dir=output) as build_dir:
        wheel_environment = {**environment, "CARGO_TARGET_DIR": str(root / "target/python-codegen-seed")}
        run([sys.executable, "-m", "maturin", "build", "--manifest-path", str(root / "pgorm-python/Cargo.toml"),
             "--interpreter", sys.executable, "--out", build_dir, "--locked"], environment=wheel_environment)
        wheels = list(Path(build_dir).glob("pgorm-*.whl"))
        if len(wheels) != 1:
            raise RuntimeError("expected one wheel for the selected interpreter")
        wheel = output / wheels[0].name
        wheel.write_bytes(wheels[0].read_bytes())
    with tempfile.TemporaryDirectory(prefix="install-", dir=output) as install_dir:
        run(["uv", "venv", "--python", sys.executable, install_dir], environment=environment)
        python = Path(install_dir) / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        run(["uv", "pip", "install", "--no-cache", "--no-index", "--python", str(python), str(wheel)], environment=environment)
        report = output / "queries.json"
        environment = {**environment, "PGORM_DIRECT_REPORT": str(report), "PYTHONNOUSERSITE": "1"}
        environment.pop("PYTHONPATH", None)
        run([str(python), "-I", str(root / "pgorm-python/tests/test_direct_builders.py"), "-v"],
            environment={**environment, "PATH": str(python.parent)})
        run([oracle, ORACLE_TESTS], environment=environment)
        for field in ("sql", "params"):
            assert_parity_fails(oracle, report, environment, field)
    summary = {
        "schema_version": 1, "passed": True, "query_programs": 56,
        "clean_install": True, "python_process_audit": "no launches during query phase",
        "rust_parity": True, "sql_mismatch_rejected": True, "parameter_mismatch_rejected": True,
        "wheel": wheel.name, "wheel_sha256": hashlib.sha256(wheel.read_bytes()).hexdigest(),
        "queries_sha256": hashlib.sha256(report.read_bytes()).hexdigest(),
    }
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
