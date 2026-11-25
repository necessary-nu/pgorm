"""Build and test installed default and experimental-native Python wheels."""

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


# [spec:pgorm:req:turso.python+1/test]
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("target/python-turso"))
    parser.add_argument("--target-dir", type=Path, default=Path("target/python-dev"))
    options = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    output = options.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    environment = {**os.environ, "PYO3_PYTHON": sys.executable, "CARGO_TARGET_DIR": str(options.target_dir.resolve())}
    environment.pop("PYTHONPATH", None)
    report = output / "summary.json"
    report.write_text('{"passed": false}\n')
    run([sys.executable, root / "pgorm-python/checks/notices.py"], environment)
    maturin = ["uv", "run", "--python", sys.executable, "--with", "maturin>=1.15,<2", "maturin"]
    for old in output.glob("pgorm-*.tar.gz"):
        old.unlink()
    run([*maturin, "sdist", "--manifest-path", root / "pgorm-python/Cargo.toml", "--out", output], environment)
    sources = list(output.glob("pgorm-*.tar.gz"))
    if len(sources) != 1:
        raise RuntimeError("expected one source distribution")
    outcomes = {}
    for native in (False, True):
        name = "turso" if native else "default"
        destination = output / name
        destination.mkdir(exist_ok=True)
        for old in destination.glob("*.whl"):
            old.unlink()
        features = "extension-module,turso" if native else "extension-module"
        run([*maturin, "build", "--manifest-path", root / "pgorm-python/Cargo.toml",
             "--locked", "--features", features, "--interpreter", sys.executable,
             "--out", destination], environment)
        wheels = list(destination.glob("*.whl"))
        if len(wheels) != 1:
            raise RuntimeError(f"expected one {name} wheel")
        shutil.copyfile(sources[0], destination / sources[0].name)
        run([sys.executable, root / "pgorm-python/tests/check_artifacts.py", destination], environment)
        with tempfile.TemporaryDirectory(prefix=f"pgorm-{name}-installed-") as temporary:
            installed = Path(temporary)
            run(["uv", "venv", "--python", sys.executable, installed], environment)
            python = installed / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
            run(["uv", "pip", "install", "--python", python, "--no-deps", wheels[0]], environment)
            tests = {**environment, "PGORM_EXPECT_TURSO": "1" if native else "0"}
            for target in ("test_turso.py", "test_package.py", "test_signatures.py"):
                run([python, "-I", root / "pgorm-python/tests" / target, "-v"], tests)
        outcomes[name] = {"wheel": wheels[0].name, "installed_tests": True, "package_contents": True}
    report.write_text(json.dumps({"passed": True, "source_distribution": sources[0].name, "outcomes": outcomes}, indent=2) + "\n")


if __name__ == "__main__":
    main()
