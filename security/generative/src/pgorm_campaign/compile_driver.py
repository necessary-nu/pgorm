"""Drive `pgorm-codegen` out of process, and keep its refusals separable.

The generator is Rust, so Python reaches it through a small binary that reports
per case. Three outcomes come back and they mean three different things:

    generated       — source exists; the compiler is next to judge it
    generator-error — pgorm-codegen refused at its own boundary
    driver-error    — the harness handed it something it could not read

Only the first two are evidence about pgorm. The third is a harness fault and
is raised, never scored: a suite that quietly counted its own bugs as library
behaviour would report exactly the coverage it failed to obtain.
"""

import json
from pathlib import Path

from . import process

# The driver is a detached crate, on the terms the replay harness is detached.
CRATE = Path("security/generative/compile")


class DriverError(RuntimeError):
    """The codegen driver could not be built, run, or believed."""


async def build(*, root=Path("."), target=Path("target"), timeout=1800):
    """Compile the driver once, sharing the repository target directory.

    A private target directory would make this a cold build of the parser and
    the PRQL compiler — minutes, every run, for a binary whose inputs have not
    changed.
    """
    root = Path(root).resolve()
    manifest = root / CRATE / "Cargo.toml"
    if not manifest.exists():
        raise DriverError("the codegen driver is missing from the checkout")
    if not (manifest.parent / "Cargo.lock").exists():
        await process.run(
            "cargo",
            "generate-lockfile",
            "--offline",
            "--manifest-path",
            manifest,
            timeout=timeout,
        )
    await process.run(
        "cargo",
        "build",
        "--quiet",
        "--locked",
        "--offline",
        "--manifest-path",
        manifest,
        "--target-dir",
        Path(target),
        timeout=timeout,
    )
    executable = Path(target) / "debug" / "pgorm-generative-codegen"
    if not executable.exists():
        raise DriverError("the codegen driver did not produce a binary")
    return executable


# [spec:pgorm:req:generative.compile-suite]
async def generate(executable, cases, *, timeout=300):
    """Run every codegen case in one process and index the outcomes by id."""
    requests = [case.request for case in cases]
    if not requests:
        return {}
    result = await process.run(
        executable,
        input=json.dumps({"cases": requests}),
        timeout=timeout,
        check=False,
        environment={"PATH": "/usr/bin:/bin", "TZ": "UTC"},
        output_limit=2**26,
    )
    if result.returncode != 0:
        raise DriverError("the codegen driver exited " + str(result.returncode))
    try:
        payload = json.loads(result.stdout)
    except ValueError as error:
        raise DriverError("the codegen driver printed no report: " + str(error))
    outcomes = {entry["id"]: entry for entry in payload.get("cases", ())}
    missing = [case.id for case in cases if case.id not in outcomes]
    if missing:
        raise DriverError("the codegen driver skipped " + missing[0])
    for entry in outcomes.values():
        if entry["outcome"] == "driver-error":
            raise DriverError(
                "the harness mis-described a case: " + (entry.get("message") or "")
            )
    return outcomes


# [spec:pgorm:req:generative.compile-suite]
def classify(case, outcome):
    """Score one codegen case against what generation actually did.

    A `refuse` case is satisfied only by a generator refusal that says what the
    case predicted. Generation succeeding is then a defect — the library
    accepted an input it documents itself as rejecting — and is reported as
    such rather than deferred to a build that would probably pass.
    """
    produced = outcome["outcome"]
    message = outcome.get("message") or ""
    if case.verdict == "refuse":
        if produced != "generator-error":
            return {
                "status": "unrefused",
                "detail": "generation succeeded where a refusal was expected",
            }
        if not case.expects.matches("", message):
            return {
                "status": "misrefused",
                "detail": "refused, but not as predicted: " + message,
            }
        return {"status": "expected-refusal", "detail": message}
    if produced != "generated":
        return {
            "status": "generator-failed",
            "detail": "generation failed where source was expected: " + message,
        }
    return {"status": "generated", "detail": ""}


__all__ = ["CRATE", "DriverError", "build", "classify", "generate"]
