"""Build wheel and sdist in isolation, then verify each fresh installation."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import zipfile


def run(command, environment, *, capture=False):
    return subprocess.run(
        [str(part) for part in command],
        check=True,
        env=environment,
        text=True,
        capture_output=capture,
    )


def one(directory, pattern):
    files = list(directory.glob(pattern))
    if len(files) != 1:
        raise RuntimeError(f"expected one {pattern} artifact in {directory}")
    return files[0]


def build(source, destination, environment, *formats):
    # uv creates a fresh PEP 517 environment with the Maturin backend the
    # build requirements name.
    run(
        [
            "uv",
            "build",
            source,
            "--python",
            sys.executable,
            "--out-dir",
            destination,
            "--no-sources",
            "--config-setting",
            "build-args=--locked",
            *formats,
        ],
        environment,
    )


def check_install(root, wheel, output, environment):
    with tempfile.TemporaryDirectory(prefix="pgorm-install-") as temporary:
        install = Path(temporary)
        run(["uv", "venv", "--python", sys.executable, install], environment)
        python = install / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        run(
            [
                "uv",
                "pip",
                "install",
                "--no-index",
                "--no-cache",
                "--python",
                python,
                wheel,
            ],
            environment,
        )
        tests = root / "pgorm-python/tests"
        run(
            [python, "-I", "-m", "unittest", "discover", "-s", tests, "-v"], environment
        )
        run(
            [
                sys.executable,
                root / "pgorm-python/checks/typing.py",
                "--python",
                python,
                "--output",
                output / "typing",
            ],
            environment,
        )
        native = run(
            [python, "-I", "-c", "from pgorm import _native; print(_native.__file__)"],
            environment,
            capture=True,
        ).stdout.strip()
        inspector = "otool" if sys.platform == "darwin" else "ldd"
        command = (
            [inspector, "-L", native] if inspector == "otool" else [inspector, native]
        )
        linked = run(command, environment, capture=True).stdout
        (output / "linked-native.txt").write_text(linked)
        return {
            "wheel": wheel.name,
            "sha256": hashlib.sha256(wheel.read_bytes()).hexdigest(),
            "installed_tests": True,
            "typing": True,
            "tls": True,
            "native_links": inspector,
        }


def compare_packages(first, second):
    def payload(path):
        with zipfile.ZipFile(path) as wheel:
            return {
                name: wheel.read(name)
                for name in wheel.namelist()
                if (name.startswith("pgorm/") and not name.endswith((".so", ".pyd")))
                or name.endswith((".dist-info/METADATA", ".dist-info/WHEEL"))
            }

    if payload(first) != payload(second):
        raise RuntimeError(
            "wheel and sdist-built wheel have different Python files or package metadata"
        )


# [spec:pgorm:req:python.distribution]
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output", type=Path, default=Path("target/python-distribution")
    )
    options = parser.parse_args()
    if not all(os.environ.get(name) for name in ("PGORM_TEST_DSN", "PGORM_TEST_CA")):
        parser.error(
            "PGORM_TEST_DSN and PGORM_TEST_CA are required; use a tests/with_*_postgres.py wrapper"
        )
    root = Path(__file__).resolve().parents[2]
    output = options.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = output / "summary.json"
    report.write_text('{"passed": false, "status": "running"}\n')
    environment = {**os.environ, "PYTHONNOUSERSITE": "1", "PYO3_PYTHON": sys.executable}
    for name in ("PYTHONPATH", "MYPYPATH"):
        environment.pop(name, None)
    run([sys.executable, root / "pgorm-python/checks/notices.py"], environment)
    outcomes = {}
    with tempfile.TemporaryDirectory(prefix="pgorm-distribution-") as temporary:
        work = Path(temporary)
        wheel_env = {
            **environment,
            "CARGO_TARGET_DIR": str(root / "target/python-codegen-seed"),
        }
        build(root / "pgorm-python", work / "direct", wheel_env, "--wheel", "--sdist")
        wheel = one(work / "direct", "pgorm-*.whl")
        source = one(work / "direct", "pgorm-*.tar.gz")
        direct = output / "wheel"
        direct.mkdir(exist_ok=True)
        for artifact in (wheel, source):
            shutil.copyfile(artifact, direct / artifact.name)
        run(
            [sys.executable, root / "pgorm-python/tests/check_artifacts.py", direct],
            environment,
        )
        outcomes["wheel"] = check_install(root, wheel, direct, environment)
        # The wheel from the sdist shares the direct build's target directory.
        # The sdist's own crates still compile from its unpacked sources, whose
        # paths differ from the checkout's, so a file the sdist left out still
        # fails the build; only registry and git dependencies, which no sdist
        # carries, come from the first build.
        build(source, work / "from-source", wheel_env, "--wheel")
        rebuilt = one(work / "from-source", "pgorm-*.whl")
        compare_packages(wheel, rebuilt)
        source_output = output / "sdist"
        source_output.mkdir(exist_ok=True)
        shutil.copyfile(rebuilt, source_output / rebuilt.name)
        shutil.copyfile(source, source_output / source.name)
        run(
            [
                sys.executable,
                root / "pgorm-python/tests/check_artifacts.py",
                source_output,
            ],
            environment,
        )
        outcomes["sdist"] = check_install(root, rebuilt, source_output, environment)
    summary = {
        "passed": True,
        "python": platform.python_version(),
        "implementation": platform.python_implementation(),
        "system": platform.system(),
        "machine": platform.machine(),
        "platform": platform.platform(),
        "gil_disabled": bool(__import__("sysconfig").get_config_var("Py_GIL_DISABLED")),
        "fresh_installations": 2,
        "pep517_isolation": True,
        "python_payloads_identical": True,
        "outcomes": outcomes,
        "sdist_sha256": hashlib.sha256((direct / source.name).read_bytes()).hexdigest(),
    }
    report.write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
