"""Build each batch and read per-case verdicts out of the compiler's own report.

Two failures wear the same exit status as a rejection and must not be scored as
one. A *toolchain* failure — an unresolvable lock, a missing dependency, a
linker that fell over — produces no diagnostic attributable to any case, and a
suite that read it as "the negatives were rejected" would report its strongest
result on the run where it learned nothing. An *unattributed* diagnostic is the
same problem one level down: something was rejected, but not demonstrably the
program that predicted it.

So attribution is positive and per case. A negative passes only when a
diagnostic inside its own line range names the rejection it predicted; silence
is never a pass.
"""

import json
import time
from pathlib import Path

from . import process
from .compile_crate import group, write

# Diagnostics for a whole batch run well past the default budget.
OUTPUT_LIMIT = 2**26


class RunnerError(RuntimeError):
    """A batch could not be built in a way that yields any verdict at all."""


def _diagnostics(stdout, manifest):
    """Errors cargo attributed to this crate, flattened to code plus location.

    Messages from path dependencies are dropped: pgorm's own warnings are not
    evidence about a generated program.
    """
    found = []
    finished = None
    # cargo reports an absolute, symlink-resolved manifest path; the caller's
    # may be relative. Comparing them as written silently drops every
    # diagnostic, which reads exactly like a build that said nothing.
    wanted = Path(manifest).resolve()
    for line in stdout.splitlines():
        try:
            record = json.loads(line)
        except ValueError:
            continue
        if record.get("reason") == "build-finished":
            finished = bool(record.get("success"))
            continue
        if record.get("reason") != "compiler-message":
            continue
        reported = record.get("manifest_path", "")
        if not reported or Path(reported).resolve() != wanted:
            continue
        message = record.get("message") or {}
        if message.get("level") != "error":
            continue
        code = (message.get("code") or {}).get("code") or ""
        text = message.get("message") or ""
        spans = [
            (span.get("file_name", ""), span.get("line_start", 0))
            for span in message.get("spans", ())
            if span.get("is_primary")
        ]
        found.append({"code": code, "message": text, "spans": spans or [("", 0)]})
    return found, finished


def _attribute(batch, diagnostics):
    """Route each diagnostic to the case whose source produced it."""
    per_case = {placement.case.id: [] for placement in batch.placements}
    orphans = []
    for diagnostic in diagnostics:
        placement = None
        for file_name, line in diagnostic["spans"]:
            placement = batch.locate(file_name, line)
            if placement is not None:
                break
        if placement is None:
            orphans.append(diagnostic)
        else:
            per_case[placement.case.id].append(diagnostic)
    return per_case, orphans


# [spec:pgorm:req:generative.compile-suite]
def _verdict(case, diagnostics):
    """Score one case against the diagnostics that named its own source."""
    if case.verdict == "accept":
        if diagnostics:
            return {
                "status": "unexpected-rejection",
                "detail": diagnostics[0]["message"],
                "codes": sorted({item["code"] for item in diagnostics if item["code"]}),
            }
        return {"status": "compiled", "detail": "", "codes": []}
    if not diagnostics:
        return {
            "status": "unrejected",
            "detail": "the program compiled where a rejection was expected",
            "codes": [],
        }
    for diagnostic in diagnostics:
        if case.expects.matches(diagnostic["code"], diagnostic["message"]):
            return {
                "status": "expected-rejection",
                "detail": diagnostic["message"],
                "codes": [diagnostic["code"]] if diagnostic["code"] else [],
                "coded": bool(diagnostic["code"]),
            }
    return {
        "status": "misrejected",
        "detail": diagnostics[0]["message"],
        "codes": sorted({item["code"] for item in diagnostics if item["code"]}),
    }


# [spec:pgorm:req:generative.compile-suite]
async def lock(manifest, *, timeout=1800):
    """Resolve a rendered batch's lockfile, which its build then reads locked.

    Offline: the batch depends on the checkout under test by path, and every
    other crate it reaches has to be the one already fetched for that
    checkout. prqlc and pgsql-syntax arrive through pgorm's own git dependencies
    — a batch is its own workspace, so a patch table in any other manifest
    would not reach it.
    """
    await process.run(
        "cargo",
        "generate-lockfile",
        "--offline",
        "--manifest-path",
        manifest,
        timeout=timeout,
    )


async def build_batch(batch, directory, *, root, target, generated=None, timeout=1800):
    """Render, lock and build one batch; return its raw compiler report."""
    emitted = write(batch, directory, root=root, generated=generated)
    manifest = emitted["manifest"]
    await lock(manifest, timeout=timeout)
    started = time.monotonic()
    result = await process.run(
        "cargo",
        "build",
        "--locked",
        "--offline",
        "--message-format=json",
        "--manifest-path",
        manifest,
        "--target-dir",
        Path(target),
        timeout=timeout,
        check=False,
        output_limit=OUTPUT_LIMIT,
    )
    diagnostics, finished = _diagnostics(result.stdout, manifest)
    return {
        "crate": emitted["crate"],
        "manifest": str(manifest),
        "exit_code": result.returncode,
        "succeeded": finished,
        "stderr": result.stderr[-4096:],
        "diagnostics": diagnostics,
        "seconds": time.monotonic() - started,
    }


# [spec:pgorm:req:generative.compile-suite]
def score(batch, built):
    """Turn one built batch into per-case verdicts plus a batch-level status."""
    per_case, orphans = _attribute(batch, built["diagnostics"])
    # Nothing compiled and nothing was said about any case: the build never
    # reached the programs, so no case in it was tested.
    toolchain = built["exit_code"] != 0 and not built["diagnostics"]
    results = []
    for placement in batch.placements:
        case = placement.case
        if toolchain:
            results.append(
                {
                    **case.data(),
                    "status": "toolchain-failure",
                    "detail": built["stderr"].strip().splitlines()[-1:] or [""],
                    "codes": [],
                }
            )
            continue
        results.append({**case.data(), **_verdict(case, per_case[case.id])})
    return {
        **batch.data(),
        "crate": built["crate"],
        "exit_code": built["exit_code"],
        "seconds": built["seconds"],
        "toolchain_failure": toolchain,
        "unattributed": orphans,
        "results": results,
    }


async def run(
    cases, directory, *, root, target, generated=None, limit=24, timeout=1800
):
    """Build every batch the cases fall into and score them all."""
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    scored = []
    for index, batch in enumerate(group(cases, limit=limit)):
        built = await build_batch(
            batch,
            directory / f"batch-{index:02d}",
            root=root,
            target=target,
            generated=generated,
            timeout=timeout,
        )
        scored.append(score(batch, built))
    return scored


__all__ = ["OUTPUT_LIMIT", "RunnerError", "build_batch", "lock", "run", "score"]
