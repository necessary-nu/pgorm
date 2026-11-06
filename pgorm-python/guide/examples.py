"""The Python guide's chapters and the examples in them, and how they run.

Every ```python block of a chapter, and of the package's README, is an
example: a whole program, run in a process of its own with this directory on
its path, so it can import the ``bakery`` module beside it. An example that reads ``DATABASE_URL`` runs
against a scratch database of its own, created on the server
``PGORM_TEST_DSN`` names (``DATABASE_URL`` when it is unset), filled with the
bakery and dropped when the example exits. A block fenced ```python registered runs only in an application build
that registers the bakery's Rust entities (``checks/guide.py``); every other
block runs against the standalone wheel.

    python examples.py run [CHAPTER ...]     run the chapters' examples
    python examples.py write DIRECTORY [CHAPTER ...]
                                             write each example as a module
    python examples.py typecheck DIRECTORY   check them with strict mypy
                                             against the package's source stubs

Both take ``--registered`` to select the registered examples instead.
"""

from __future__ import annotations

import argparse
import asyncio
from dataclasses import dataclass
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from urllib.parse import parse_qs, urlsplit, urlunsplit

GUIDE = Path(__file__).resolve().parent
RUST_CHAPTERS = GUIDE.parents[1] / "docs/guide/chapters.json"

# The words a python fence's info string may carry after the language.
MARKERS = frozenset({"registered"})


@dataclass(frozen=True)
class Chapter:
    id: str
    title: str
    file: str
    python_only: bool


@dataclass(frozen=True)
class Example:
    chapter: str
    index: int
    line: int
    code: str
    registered: bool

    @property
    def name(self) -> str:
        return f"{self.chapter.replace('-', '_')}_{self.index}"

    @property
    def needs_database(self) -> bool:
        return "DATABASE_URL" in self.code


# [spec:pgorm:req:guide.dialects]    the Python guide's chapters: the Rust guide's one for one, and its declared Python-only ones
def chapters() -> list[Chapter]:
    """The Python guide's chapters, in reading order."""
    listed = json.loads((GUIDE / "chapters.json").read_text())["chapters"]
    return [
        Chapter(entry["id"], entry["title"], entry["file"], entry.get("python_only", False))
        for entry in listed
    ]


def rust_chapters() -> list[dict[str, str]]:
    """The Rust guide's chapters, which the Python guide follows one for one."""
    return json.loads(RUST_CHAPTERS.read_text())["chapters"]


def fences(text: str) -> list[tuple[str, int, list[str]]]:
    """Each fenced block of a chapter: its info string, the line its fence
    opens on, and its lines."""
    blocks: list[tuple[str, int, list[str]]] = []
    open_block: tuple[str, int, list[str]] | None = None
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if open_block is None:
            if stripped.startswith("```"):
                open_block = (stripped[3:].strip(), number, [])
        elif stripped.startswith("```"):
            blocks.append(open_block)
            open_block = None
        else:
            open_block[2].append(line)
    if open_block is not None:
        raise ValueError(f"the fence opened on line {open_block[1]} is never closed")
    return blocks


def examples(chapter: Chapter) -> list[Example]:
    """A chapter's runnable examples, in the order they appear."""
    found: list[Example] = []
    for info, line, lines in fences((GUIDE / chapter.file).read_text()):
        words = info.split()
        if not words or words[0] != "python":
            continue
        unknown = set(words[1:]) - MARKERS
        if unknown:
            raise ValueError(f"{chapter.file}:{line}: unknown fence markers {sorted(unknown)}")
        found.append(Example(chapter.id, len(found) + 1, line, "\n".join(lines) + "\n",
                             "registered" in words[1:]))
    return found


# The package's README opens with a first query, which runs as the guide's do.
ENTRY_POINT = Chapter("readme", "pgorm for Python", "../README.md", True)


def all_examples(*, registered: bool, only: set[str] | None = None) -> list[Example]:
    return [
        example
        for chapter in [*chapters(), ENTRY_POINT]
        if only is None or chapter.id in only
        for example in examples(chapter)
        if example.registered == registered
    ]


def write(directory: Path, selected: list[Example]) -> list[Path]:
    """Write each example as a module of its own beside a copy of ``bakery``,
    for a type checker to read, replacing the modules a previous write left
    there, so an example since removed is not checked again."""
    directory.mkdir(parents=True, exist_ok=True)
    for stale in directory.glob("*.py"):
        stale.unlink()
    shutil.copyfile(GUIDE / "bakery.py", directory / "bakery.py")
    paths = []
    for example in selected:
        path = directory / f"{example.name}.py"
        path.write_text(example.code)
        paths.append(path)
    return paths


def _fnv1a(data: bytes) -> int:
    hash_ = 0x811C9DC5
    for byte in data:
        hash_ = ((hash_ ^ byte) * 0x01000193) & 0xFFFFFFFF
    return hash_


def _owner() -> int:
    try:
        host = Path("/proc/sys/kernel/hostname").read_text().strip()
    except OSError:
        host = socket.gethostname().strip()
    if not host:
        return 0
    user = next((os.environ[key] for key in ("USER", "LOGNAME", "USERNAME") if key in os.environ), "")
    return _fnv1a(f"{host}\n{user}".encode())


_RUN = (_owner(), os.getpid(), int(time.time()))
_minted = 0


def scoped_name(label: str) -> str:
    """A database name for ``label`` unique to this process and this call:
    ``{label}_{seq}__{owner}_{pid}_{stamp}``, the label cut to fit
    PostgreSQL's 63 bytes. pgorm-python's suite starts a server of its own,
    but the examples also run on a server other runs share, so they take the
    names the live suites sharing one mint (``docs/spec/testing.md``)."""
    global _minted
    owner, pid, stamp = _RUN
    tail = f"_{_minted}__{owner:08x}_{pid}_{stamp:08x}"
    _minted += 1
    cut = label
    while len((cut + tail).encode()) > 63:
        cut = cut[:-1]
    return cut + tail


def test_server() -> str | None:
    """The server the examples run against, as the JavaScript suite's
    ``dsn()`` reads it: ``PGORM_TEST_DSN``, else ``DATABASE_URL``. A URL
    naming no database names ``postgres``, which the scratch databases are
    created from, and one that says nothing of TLS gains ``sslmode=disable``:
    a pgorm pool verifies TLS unless its URL says otherwise, and the suites'
    servers are reached in plaintext, the TLS tests reading ``PGORM_TEST_CA``
    instead. A URL that names its sslmode keeps it."""
    server = os.environ.get("PGORM_TEST_DSN") or os.environ.get("DATABASE_URL")
    if not server:
        return None
    parts = urlsplit(server)
    path = parts.path if parts.path not in ("", "/") else "/postgres"
    query = parts.query
    if "sslmode" not in parse_qs(query):
        query = f"{query}&sslmode=disable" if query else "sslmode=disable"
    return urlunsplit((parts.scheme, parts.netloc, path, query, parts.fragment))


def _database_url(server: str, name: str) -> str:
    parts = urlsplit(server)
    return urlunsplit((parts.scheme, parts.netloc, "/" + name, parts.query, parts.fragment))


async def _scratch(server: str, name: str) -> str:
    import pgorm

    sys.path.insert(0, str(GUIDE))
    import bakery

    async with pgorm.Pool(server, max_size=1) as admin:
        await admin.execute(pgorm.RawSQL(f'CREATE DATABASE "{name}"', []))
    url = _database_url(server, name)
    async with pgorm.Pool(url, max_size=1) as pool:
        await bakery.create(pool)
        await bakery.seed(pool)
    return url


async def _drop(server: str, name: str) -> None:
    import pgorm

    # The database is this run's own, and so is every session in it: a
    # statement an example cancelled may still be running there.
    async with pgorm.Pool(server, max_size=1) as admin:
        await admin.execute(pgorm.RawSQL(f'DROP DATABASE IF EXISTS "{name}" WITH (FORCE)', []))


@dataclass(frozen=True)
class Outcome:
    example: Example
    returncode: int
    output: str


async def run(example: Example, server: str | None, python: str = sys.executable) -> Outcome:
    """Run one example in a process of its own, in a scratch bakery database
    when it reads ``DATABASE_URL``."""
    environment = {**os.environ, "PYTHONPATH": str(GUIDE), "PYTHONNOUSERSITE": "1"}
    name = None
    if example.needs_database:
        if server is None:
            raise RuntimeError("PGORM_TEST_DSN names the server the guide's examples run against")
        name = scoped_name(f"guide_{example.name}")
        environment["DATABASE_URL"] = await _scratch(server, name)
    try:
        with tempfile.TemporaryDirectory(prefix="pgorm-guide-") as directory:
            path = Path(directory) / f"{example.name}.py"
            path.write_text(example.code)
            process = await asyncio.create_subprocess_exec(
                python, str(path), cwd=directory, env=environment,
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT,
            )
            output, _ = await process.communicate()
            assert process.returncode is not None
            return Outcome(example, process.returncode, output.decode(errors="replace"))
    finally:
        if name is not None:
            assert server is not None
            await _drop(server, name)


async def run_all(selected: list[Example], server: str | None, *, concurrency: int = 4,
                  python: str = sys.executable) -> list[Outcome]:
    """Run examples a few at a time, each in a database of its own."""
    gate = asyncio.Semaphore(concurrency)

    async def one(example: Example) -> Outcome:
        async with gate:
            return await run(example, server, python)

    return list(await asyncio.gather(*(one(example) for example in selected)))


def _main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--registered", action="store_true", help="the examples a registration build runs")
    commands = parser.add_subparsers(dest="command", required=True)
    running = commands.add_parser("run")
    running.add_argument("chapters", nargs="*")
    running.add_argument("--python", default=sys.executable)
    writing = commands.add_parser("write")
    writing.add_argument("directory", type=Path)
    writing.add_argument("chapters", nargs="*")
    checking = commands.add_parser("typecheck")
    checking.add_argument("directory", type=Path)
    options = parser.parse_args()
    if options.command == "typecheck":
        modules = options.directory / "examples"
        write(modules, all_examples(registered=options.registered))
        environment = {**os.environ, "MYPYPATH": str(GUIDE.parent / "python")}
        command = ["uv", "tool", "run", "--from", "mypy@latest", "mypy", "--strict",
                   "--cache-dir", str(options.directory / "cache"), str(modules)]
        raise SystemExit(subprocess.run(command, env=environment).returncode)
    if options.command == "write":
        selected = all_examples(registered=options.registered, only=set(options.chapters) or None)
        for path in write(options.directory, selected):
            print(path)
        return
    selected = all_examples(registered=options.registered, only=set(options.chapters) or None)
    outcomes = asyncio.run(run_all(selected, test_server(), python=options.python))
    failed = [outcome for outcome in outcomes if outcome.returncode]
    for outcome in failed:
        example = outcome.example
        print(f"--- {example.chapter} example {example.index} (line {example.line}) failed")
        print(outcome.output)
    print(f"{len(outcomes) - len(failed)} of {len(outcomes)} examples passed")
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    _main()
