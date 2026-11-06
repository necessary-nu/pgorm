"""Check an installed pgorm wheel's types, signatures and runnable application."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


def run(command, environment, *, capture=False):
    result = subprocess.run(
        [str(part) for part in command],
        env=environment,
        text=True,
        capture_output=capture,
    )
    if result.returncode:
        # A captured child's traceback would otherwise be lost to the caller's log.
        if capture:
            print(result.stderr, end="", file=sys.stderr)
        raise subprocess.CalledProcessError(
            result.returncode, result.args, result.stdout, result.stderr
        )
    return result


def reject_invalid(command, source, environment):
    expected = [
        (number, match[1])
        for number, line in enumerate(source.read_text().splitlines(), 1)
        if (match := re.search(r"# expected-error: ([\w-]+)", line))
    ]
    result = subprocess.run(
        [*map(str, command), str(source)],
        env=environment,
        text=True,
        capture_output=True,
    )
    print(result.stdout, end="")
    diagnostics = re.findall(
        r"^.*?:(\d+): error: .*?\[([\w-]+)\]$", result.stdout, re.MULTILINE
    )
    actual = [(int(line), code) for line, code in diagnostics]
    if result.returncode != 1 or not expected or actual != expected:
        raise RuntimeError(
            f"expected type diagnostics {expected}, received {actual}: {result.stderr}"
        )
    return len(expected)


def installed_metadata(python, environment):
    program = """
import hashlib, importlib.metadata, json, pathlib, platform, pgorm
from pgorm import _native
root = pathlib.Path(pgorm.__file__).resolve().parent
assert (root / 'py.typed').is_file()
assert pathlib.Path(_native.__file__).resolve().parent == root
assert root == pathlib.Path(importlib.metadata.distribution('pgorm').locate_file('pgorm')).resolve()
print(json.dumps({'version': pgorm.__version__, 'python': platform.python_version(),
    'native': str(_native.__file__), 'stubs': {
        str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.rglob('*.pyi'))}}))
"""
    return json.loads(
        run([python, "-I", "-c", program], environment, capture=True).stdout
    )


def guide_examples(root, directory):
    """Write the guide's examples that run on the standalone wheel as modules
    beside the bakery they import, for mypy to check as the guide's reader
    would run them."""
    sys.path.insert(0, str(root / "pgorm-python/guide"))
    import examples

    paths = examples.write(directory, examples.all_examples(registered=False))
    return [directory / "bakery.py", *paths]


# [spec:pgorm:req:python.typing+2]
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--python",
        type=Path,
        default=Path(sys.executable),
        help="interpreter with the wheel installed",
    )
    parser.add_argument("--output", type=Path, default=Path("target/python-typing"))
    options = parser.parse_args()
    if not os.environ.get("PGORM_TEST_DSN"):
        parser.error("PGORM_TEST_DSN is required; run through tests/with_postgres.py")
    root = Path(__file__).resolve().parents[2]
    output = options.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = output / "summary.json"
    report.write_text('{"passed": false, "status": "running"}\n')
    # Keep the venv executable path: resolving its symlink selects the base interpreter.
    python = options.python.absolute()
    environment = {**os.environ, "PYTHONNOUSERSITE": "1"}
    for name in ("PYTHONPATH", "MYPYPATH"):
        environment.pop(name, None)
    metadata = installed_metadata(python, environment)
    run(
        [python, "-I", root / "pgorm-python/tests/test_signatures.py", "-v"],
        environment,
    )
    application = root / "pgorm-python/examples/application.py"
    with tempfile.TemporaryDirectory(prefix="pgorm-typing-") as temporary:
        launcher = ["uv", "tool", "run", "--from", "mypy@latest", "mypy"]
        mypy = [
            *launcher,
            "--strict",
            "--no-incremental",
            "--no-pretty",
            "--no-color-output",
            "--show-error-codes",
            "--python-executable",
            str(python),
            "--cache-dir",
            temporary,
        ]
        checker = run(
            [*launcher, "--version"], environment, capture=True
        ).stdout.split(" (")[0]
        run([*mypy, application], environment)
        guide = guide_examples(root, Path(temporary) / "guide")
        run([*mypy, *guide], environment)
        run([*mypy, "--package", "pgorm"], environment)
        # [spec:pgorm:req:bindings.models-from-sql/test]    the module models_from_sql writes type-checks under strict mypy
        run([*mypy, root / "pgorm-python/tests/models_from_sql/models.py"], environment)
        rejected = reject_invalid(
            mypy, root / "pgorm-python/tests/public_types_invalid.py", environment
        )
    result = json.loads(
        run([python, "-I", application], environment, capture=True).stdout
    )
    summary = {
        "passed": True,
        "installed": metadata,
        "native_signatures": True,
        "type_checker": checker,
        "invalid_typing_cases": rejected,
        "generated_models": True,
        "application": result,
        "guide_examples": len(guide) - 1,
    }
    report.write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
