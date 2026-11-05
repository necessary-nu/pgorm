"""Run pgorm-python's Rust unit tests against a CPython the test binary can link.

The unit tests embed an interpreter, so their binary links libpython, which
pyo3 finds through the interpreter it builds against: ``PYO3_PYTHON``, or
``python3`` on ``PATH``. A wheel's build enables ``extension-module``, which
leaves libpython unlinked; the crate's default features do not, so the tests
build without it. A python-build-standalone interpreter (rye's, an older
uv's) reports as its ``LIBDIR`` the directory it was built in, which no other
machine has, and the link fails on it. So this runs the tests against the
first interpreter whose ``LIBDIR`` holds a libpython: ``PYO3_PYTHON`` when it
is set, then ``python3.14`` down to ``python3.11`` and ``python3`` on
``PATH``, then the one running this script.

They build in a target directory of their own. pyo3 rebuilds itself and
everything over it whenever the interpreter changes, and the other
python-native checks build against whichever ``python3`` is first on
``PATH``, so sharing their directory would rebuild both on every run.
"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
PROBE = (
    "import json, sys, sysconfig; print(json.dumps({"
    "'libdir': sysconfig.get_config_var('LIBDIR'), 'version': sys.version.split()[0]}))"
)


def candidates():
    """The interpreters to try, in order, each named once."""
    names = [os.environ.get("PYO3_PYTHON")]
    names += [shutil.which(f"python3.{minor}") for minor in range(14, 10, -1)]
    names += [shutil.which("python3"), sys.executable]
    seen = set()
    for name in names:
        if name and name not in seen:
            seen.add(name)
            yield name


def linkable(interpreter):
    """The ``LIBDIR`` of ``interpreter`` when it holds a libpython, else ``None``."""
    try:
        probe = subprocess.run(
            [interpreter, "-c", PROBE], capture_output=True, text=True, check=True
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    libdir = json.loads(probe.stdout)["libdir"]
    if not libdir or not Path(libdir).is_dir():
        return None
    if not any(entry.name.startswith("libpython") for entry in Path(libdir).iterdir()):
        return None
    return libdir


def main():
    for interpreter in candidates():
        libdir = linkable(interpreter)
        if libdir is not None:
            break
    else:
        print("no CPython on this machine has a libpython to link the unit tests against", file=sys.stderr)
        return 1
    print(f"pgorm-python unit tests against {interpreter} ({libdir})", file=sys.stderr)
    environment = dict(os.environ, PYO3_PYTHON=interpreter)
    # The binary loads libpython when it runs; a shared build outside the
    # loader's default paths is found here.
    for variable in ("LD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH"):
        environment[variable] = os.pathsep.join(
            part for part in (libdir, os.environ.get(variable)) if part
        )
    command = [
        "cargo", "test", "--manifest-path", str(ROOT / "pgorm-python" / "Cargo.toml"),
        "--locked", "--target-dir", str(ROOT / "target" / "python-unit"), "--lib",
    ]
    return subprocess.run(command, env=environment).returncode


if __name__ == "__main__":
    sys.exit(main())
