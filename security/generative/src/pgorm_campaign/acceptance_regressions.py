"""Run the named regressions, and show each one failing without its fix.

A regression test that passes proves nothing on its own: a test asserting the
defect's behaviour would also pass. What makes it a regression is that removing
the fix makes it fail, so each entry here is run twice — once in this checkout,
and once in a throwaway worktree with the fix taken back out.

The counterfactual worktree is scratch. It is created under the build output,
never in the source tree, and it is removed whether or not the experiment
worked. Nothing here edits the checkout it was launched from.
"""

import shutil
import time
from pathlib import Path

from . import process

VERSION = 1

# How the fix is taken back out for the counterfactual. `revert-paths` restores
# the production files as they were immediately before the fix landed, which is
# exact when nothing has touched them since; `swap-dependency` rewrites one
# dependency's declaration in each named manifest and leaves every other line
# alone, which is how a fix that is a dependency revision is taken out.
REMOVALS = {
    "deduplicated_relations_still_combine": {
        "kind": "revert-paths",
        "commit": "08e1ec95^",
        "paths": ["src/pipeline/builder.rs"],
        "detail": "restore src/pipeline/builder.rs from before the fix commit",
    },
    "a_joined_deduplicated_relation_compiles_once": {
        "kind": "swap-dependency",
        "paths": ["Cargo.toml", "pgorm-sql-check/Cargo.toml"],
        "dependency": "prqlc",
        "declaration": '{ version = "=0.13.14", default-features = false }',
        "detail": "depend on registry prqlc 0.13.14 in place of the pinned fork",
    },
}


def _present(root, path, test):
    """The named test really is defined at the path the report will cite."""
    source = Path(root) / path
    return source.is_file() and ("fn " + test + "(") in source.read_text()


async def _test(module, *, cwd, target, timeout, locked=True, offline=False):
    """Run exactly one test by path, reporting its status rather than raising.

    The counterfactual worktree has no `Cargo.lock` — the root lock is not
    tracked — so it cannot be `--locked`. Distinguishing "the test ran and
    failed" from "cargo never got as far as the test" is the whole point of
    `ran`: a resolution error would otherwise read as the regression firing.
    """
    command = ["cargo", "test", "--lib", "--target-dir", str(target)]
    command += ["--locked"] if locked else []
    command += ["--offline"] if offline else []
    result = await process.run(
        *command, module, "--", "--exact", cwd=str(cwd), timeout=timeout, check=False
    )
    output = result.stdout + result.stderr
    return {
        "exit_code": result.returncode,
        "passed": result.returncode == 0,
        "ran": "test result:" in output,
        "tail": output[-2000:],
    }


async def _remove(worktree, removal):
    """Take the fix back out of a scratch worktree, in place."""
    if removal["kind"] == "revert-paths":
        await process.run(
            "git",
            "checkout",
            removal["commit"],
            "--",
            *removal["paths"],
            cwd=str(worktree),
            timeout=120,
        )
        return
    key = removal["dependency"] + " = "
    for path in removal["paths"]:
        manifest = worktree / path
        lines = manifest.read_text().splitlines(keepends=True)
        swapped = [
            key + removal["declaration"] + "\n" if line.startswith(key) else line
            for line in lines
        ]
        if swapped == lines:
            raise RuntimeError(
                "counterfactual dependency not found: "
                + removal["dependency"]
                + " in "
                + path
            )
        manifest.write_text("".join(swapped))


# [spec:pgorm:req:generative.acceptance]
async def counterfactual(entry, *, root, scratch, target, timeout):
    """Show the named test failing in a worktree with the fix taken back out."""
    removal = REMOVALS.get(entry["test"])
    if removal is None:
        return {"attempted": False, "reason": "no removal is declared for this fix"}
    worktree = Path(scratch) / entry["test"]
    if worktree.exists():
        shutil.rmtree(worktree, ignore_errors=True)
    try:
        await process.run(
            "git",
            "worktree",
            "add",
            "--detach",
            str(worktree),
            "HEAD",
            cwd=str(root),
            timeout=300,
        )
    except Exception as error:
        return {"attempted": False, "reason": type(error).__name__ + ": " + str(error)}
    try:
        shutil.copyfile(Path(root) / "Cargo.lock", worktree / "Cargo.lock")
        await _remove(worktree, removal)
        # Swapping a dependency changes resolution, so that one has to be
        # allowed to reach the network; reverting a source file does not.
        outcome = await _test(
            entry["module"],
            cwd=worktree,
            target=target,
            timeout=timeout,
            locked=False,
            offline=removal["kind"] == "revert-paths",
        )
        return {
            "attempted": True,
            "removal": removal["detail"],
            "kind": removal["kind"],
            "exit_code": outcome["exit_code"],
            "reached_the_test": outcome["ran"],
            "failed": outcome["ran"] and not outcome["passed"],
            "reason": "the named regression fails without its fix"
            if outcome["ran"] and not outcome["passed"]
            else "the test still passes without the fix"
            if outcome["ran"]
            else "cargo never reached the test; this shows nothing",
            "tail": outcome["tail"],
        }
    except Exception as error:
        return {"attempted": True, "reason": type(error).__name__ + ": " + str(error)}
    finally:
        shutil.rmtree(worktree, ignore_errors=True)
        await process.run(
            "git", "worktree", "prune", cwd=str(root), timeout=60, check=False
        )


# [spec:pgorm:req:generative.acceptance]
async def verify(
    entries,
    *,
    root,
    scratch,
    target,
    scratch_target=None,
    timeout=3600,
    counterfactuals=True,
):
    """Run every named regression here, and optionally without its fix.

    The counterfactual builds a different pgorm, so it gets its own target
    directory: sharing one would evict this checkout's artifacts and make the
    experiment's cost land on whatever built next.
    """
    scratch_target = scratch_target or Path(str(target) + "-counterfactual")
    results = []
    for entry in entries:
        started = time.monotonic()
        found = _present(root, entry["path"], entry["test"])
        result = {
            "test": entry["test"],
            "module": entry["module"],
            "path": entry["path"],
            "defined_at_named_path": found,
            "executed": False,
            "passed": False,
            "reason": "the named test is not defined at the named path",
        }
        if found:
            outcome = await _test(
                entry["module"], cwd=root, target=target, timeout=timeout, offline=True
            )
            result.update(
                executed=True,
                passed=outcome["passed"],
                exit_code=outcome["exit_code"],
                reason="passes in this checkout"
                if outcome["passed"]
                else "the named regression does not pass here",
            )
            result["counterfactual"] = (
                await counterfactual(
                    entry,
                    root=root,
                    scratch=scratch,
                    target=scratch_target,
                    timeout=timeout,
                )
                if counterfactuals
                else {"attempted": False, "reason": "counterfactuals were skipped"}
            )
        result["seconds"] = time.monotonic() - started
        results.append(result)
    return results


__all__ = ["REMOVALS", "VERSION", "counterfactual", "verify"]
