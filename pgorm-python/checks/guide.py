"""Build the Python guide's registration of the bakery and run its examples.

The guide's registrations chapter describes an application that registers the
bakery's Rust entities, the ones ``pgorm-guide-support`` declares, through
``pgorm.codegen``. Its ```json application.json block is that application's
configuration. This check builds the standalone wheel the generator runs
from, scaffolds the application, builds its probe wheel, emits the typed
module, builds the application wheel and installs it in a fresh environment,
as ``checks/codegen.py`` does, then runs every example the guide fences
```python registered there, each in a scratch database holding the bakery,
and checks them with mypy in strict mode.

Run it with maturin installed in this interpreter:

    python pgorm-python/tests/with_postgres.py python pgorm-python/checks/guide.py
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from codegen import build, install, run

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "pgorm-python/guide"))

import examples  # noqa: E402

CONFIGURATION = "json application.json"


def configuration():
    """The registrations chapter's application.json, its entity crate the
    guide's support crate."""
    chapter = next(chapter for chapter in examples.chapters() if chapter.id == "registrations")
    blocks = [
        lines
        for info, _, lines in examples.fences((examples.GUIDE / chapter.file).read_text())
        if info == CONFIGURATION
    ]
    if len(blocks) != 1:
        raise RuntimeError(f"{chapter.file} holds one ```{CONFIGURATION} block, not {len(blocks)}")
    description = json.loads("\n".join(blocks[0]))
    description["entity_crate"] = str(ROOT / "pgorm-guide-support")
    return description


# [spec:pgorm:req:guide.dialects/test]    the guide's registered examples run against a build registering the bakery
def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path, default=Path("target/python-guide"))
    options = parser.parse_args()
    if not os.environ.get("PGORM_TEST_DSN"):
        parser.error("PGORM_TEST_DSN is required; run through tests/with_postgres.py")
    output = options.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    (output / "summary.json").write_text('{"passed": false, "status": "running"}\n')
    environment = {
        **os.environ,
        "PYO3_PYTHON": sys.executable,
        "CARGO_TARGET_DIR": str(ROOT / "target"),
        "PYTHONNOUSERSITE": "1",
    }
    environment.pop("PYTHONPATH", None)
    # The standalone and the application builds enable different module
    # initializers, so each keeps a target directory of its own.
    seed_environment = {**environment, "CARGO_TARGET_DIR": str(ROOT / "target/python-codegen-seed")}
    selected = examples.all_examples(registered=True)
    if not selected:
        raise RuntimeError("the guide has no registered examples to run")
    with tempfile.TemporaryDirectory(prefix="pgorm-guide-") as directory:
        temporary = Path(directory)
        description = configuration()
        config = temporary / "application.json"
        config.write_text(json.dumps(description, indent=2) + "\n")
        project = temporary / "application"
        seed = install(build(ROOT / "pgorm-python/Cargo.toml", temporary / "seed-dist",
                             seed_environment, locked=True), temporary / "seed", environment)
        run([seed, "-I", "-m", "pgorm.codegen", "scaffold", config,
             "--output", project, "--pgorm-source", ROOT], environment)
        # Reuse the tested dependency resolution; only the application's own
        # packages are added to it.
        shutil.copyfile(ROOT / "pgorm-python/Cargo.lock", project / "Cargo.lock")
        probe = install(build(project / "Cargo.toml", temporary / "probe-dist", environment, locked=False),
                        temporary / "probe", environment)
        run([probe, "-I", "-m", "pgorm.codegen", "emit", project], environment)
        final = install(build(project / "Cargo.toml", temporary / "final-dist", environment, locked=True),
                        temporary / "final", environment)

        # The harness runs in the application's environment, so the scratch
        # databases are made by the same package the examples import.
        run([final, ROOT / "pgorm-python/guide/examples.py", "--registered", "run"], environment)

        typed = temporary / "typed"
        paths = examples.write(typed, selected)
        launcher = ["uv", "tool", "run", "--from", "mypy@latest", "mypy"]
        run([*launcher, "--strict", "--no-incremental", "--no-pretty", "--show-error-codes",
             "--python-executable", final, "--cache-dir", temporary / "mypy-cache",
             typed / "bakery.py", *paths], environment)
        checker = subprocess.run([*launcher, "--version"], env=environment, text=True,
                                 capture_output=True, check=True).stdout.split(" (")[0]
    summary = {
        "passed": True,
        "module": description["module"],
        "registered_entities": len(description["entities"]),
        "registered_examples": len(selected),
        "type_checker": checker,
    }
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
