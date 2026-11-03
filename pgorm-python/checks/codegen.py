"""Build, generate and reinstall an application package, then exercise its API."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def run(command, environment):
    subprocess.run([str(part) for part in command], env=environment, check=True)


def build(manifest, output, environment, *, locked):
    command = [
        sys.executable,
        "-m",
        "maturin",
        "build",
        "--manifest-path",
        manifest,
        "--interpreter",
        sys.executable,
        "--out",
        output,
    ]
    if locked:
        command.append("--locked")
    run(command, environment)
    wheels = list(output.glob("pgorm-*.whl"))
    if len(wheels) != 1:
        raise RuntimeError("expected one application wheel")
    return wheels[0]


def install(wheel, destination, environment):
    run(["uv", "venv", "--python", sys.executable, destination], environment)
    python = destination / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    run(
        ["uv", "pip", "install", "--no-index", "--no-cache", "--python", python, wheel],
        environment,
    )
    return python


# [spec:pgorm:req:bindings.codegen/test]
# [spec:pgorm:req:bindings.codegen-disjoint/test]
# [spec:pgorm:req:python.typing+2/test]
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("target/python-codegen"))
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
    with tempfile.TemporaryDirectory(prefix="pgorm-codegen-") as directory:
        temporary = Path(directory)
        # Standalone and downstream dependency builds emit different module initializers.
        seed_environment = {
            **environment,
            "CARGO_TARGET_DIR": str(root / "target/python-codegen-seed"),
        }
        seed_wheel = build(
            root / "pgorm-python/Cargo.toml",
            temporary / "seed-dist",
            seed_environment,
            locked=True,
        )
        seed = install(seed_wheel, temporary / "seed", environment)
        run(
            [
                seed,
                "-I",
                "-m",
                "unittest",
                "discover",
                "-s",
                root / "pgorm-python/tests",
                "-p",
                "test_codegen.py",
                "-v",
            ],
            environment,
        )
        project = temporary / "application"
        run(
            [
                seed,
                "-I",
                "-m",
                "pgorm.codegen",
                "scaffold",
                root / "pgorm-python/tests/codegen-entities/application.json",
                "--output",
                project,
                "--pgorm-source",
                root,
            ],
            environment,
        )
        # Reuse the tested dependency resolution; only the new application package is added.
        shutil.copyfile(root / "pgorm-python/Cargo.lock", project / "Cargo.lock")
        probe_wheel = build(
            project / "Cargo.toml", temporary / "probe-dist", environment, locked=False
        )
        probe = install(probe_wheel, temporary / "probe", environment)
        run([probe, "-I", "-m", "pgorm.codegen", "emit", project], environment)
        first = (project / "python/pgorm/app.py").read_bytes()
        run([probe, "-I", "-m", "pgorm.codegen", "emit", project], environment)
        if (project / "python/pgorm/app.py").read_bytes() != first:
            raise RuntimeError("generation was not deterministic")
        final_wheel = build(
            project / "Cargo.toml", temporary / "final-dist", environment, locked=True
        )
        wheel = output / final_wheel.name
        shutil.copyfile(final_wheel, wheel)
        final = install(wheel, temporary / "final", environment)
        run([final, "-I", root / "pgorm-python/tests/test_signatures.py", "-v"], environment)
        run(
            [
                final,
                "-I",
                "-m",
                "unittest",
                "discover",
                "-s",
                root / "pgorm-python/tests",
                "-p",
                "generated_application.py",
                "-v",
            ],
            environment,
        )
        launcher = ["uv", "tool", "run", "--from", "mypy@latest", "mypy"]
        mypy = [
            *launcher,
            "--strict",
            "--python-executable",
            str(final),
            "--cache-dir",
            str(temporary / "mypy-cache"),
        ]
        checker = subprocess.run(
            [*launcher, "--version"],
            env=environment,
            text=True,
            capture_output=True,
            check=True,
        ).stdout.split(" (")[0]
        run([*mypy, root / "pgorm-python/tests/codegen_types.py"], environment)
        run([*mypy, "--package", "pgorm"], environment)
        invalid = subprocess.run(
            [*mypy, str(root / "pgorm-python/tests/codegen_types_invalid.py")],
            env=environment,
            text=True,
            capture_output=True,
        )
        print(invalid.stdout, end="")
        expected = (
            "arg-type",
            "assignment",
            "attr-defined",
            "union-attr",
            "arg-type",
            "arg-type",
            "attr-defined",
        )
        if (
            invalid.returncode != 1
            or invalid.stdout.count("error:") != len(expected)
            or not all(f"[{code}]" in invalid.stdout for code in expected)
        ):
            raise RuntimeError(
                "generated types did not reject the expected invalid programs: "
                + invalid.stderr
            )
        # A final application package can scaffold another build without copying stale wrappers.
        second = temporary / "second"
        run(
            [
                final,
                "-I",
                "-m",
                "pgorm.codegen",
                "scaffold",
                root / "pgorm-python/tests/codegen-entities/application.json",
                "--output",
                second,
                "--pgorm-source",
                root,
            ],
            environment,
        )
        if (second / "python/pgorm/app.py").exists():
            raise RuntimeError("scaffold copied an old application module")
        for name in ("app.py", "app.pyi"):
            shutil.copyfile(project / "python/pgorm" / name, output / name)
        shutil.copyfile(project / "Cargo.lock", output / "Cargo.lock")
    (output / "summary.json").write_text(
        json.dumps(
            {
                "passed": True,
                "wheel": wheel.name,
                "fresh_installs": 3,
                "generated_entities": 10,
                "generated_graphs": 3,
                "generated_disjoints": 2,
                "database_cases": 7,
                "deterministic_emission": True,
                "compatibility_checked": True,
                "type_checker": checker,
                "invalid_typing_cases": 7,
                "installed_signatures": True,
                "package_typing": True,
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
