"""The runner must refuse to call a run successful when work went unaccounted."""

import asyncio
import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

from pgorm_campaign import campaign_plan, campaign_runner, profiles
from pgorm_campaign.campaign_artifacts import Artifacts

ROOT = Path(__file__).resolve().parents[3]


def declared(program):
    """What the program itself says its outcome should be."""
    return (
        "expected-rejection"
        if any(
            item["oracle"] == "exact-error" for item in program.data()["observations"]
        )
        else "pass"
    )


def profile(**classes):
    """A narrow profile: two constructions, two live programs, nothing else."""
    document, digest = profiles.load()
    value = copy.deepcopy(document["profiles"]["smoke"])
    value["run_classes"]["construction"]["programs"] = 2
    value["run_classes"]["runtime"]["programs"] = 2
    for name in ("invalid", "control", "compile"):
        value["run_classes"][name] = {
            "included": False,
            "database": name != "compile",
            "oracle": name != "compile",
            "reason": "outside this check",
        }
    value["controls"]["required"] = False
    value["budgets"]["shrink"]["enabled"] = False
    for name, spec in classes.items():
        value["run_classes"][name].update(spec)
    return profiles.Profile("narrow", value, digest)


class Checker:
    """A stand-in oracle that reports whatever the test asked it to report."""

    def __init__(self, status=None, error=None, cleanup=()):
        self.status = status
        self.error = error
        self.cleanup = list(cleanup)
        self.seen = []

    async def run(self, program, timeout=None):
        self.seen.append(program.digest)
        data = program.data()
        report = {
            "program_sha256": program.digest,
            "status": self.status or declared(program),
            "comparisons": [
                {"step": "s0", "oracle": "reference", "equal": True, "reason": "equal"}
            ],
            "cleanup_errors": self.cleanup,
            "subject": {
                "program_sha256": program.digest,
                "builds": 0,
                "cleanup_errors": [],
                "trace": [],
                "steps": [
                    {
                        "id": step["id"],
                        "status": "observed",
                        "native_paths": ["pgorm::Connection::query_raw"],
                        "observation": {"kind": "record", "fields": []},
                    }
                    for step in data["steps"]
                ],
            },
        }
        if self.status == "defect":
            report["comparisons"] = [
                {
                    "step": data["steps"][0]["id"],
                    "oracle": "reference",
                    "equal": False,
                    "reason": "row multisets differ",
                }
            ]
        if self.error is not None:
            report["error"] = self.error
        return report


class Corrupting(Artifacts):
    """An evidence directory whose report will not read back as written."""

    def __init__(self, directory, target):
        super().__init__(directory)
        self.target = target

    def text(self, name, body, *, parse=False):
        if name == self.target:
            body = "{ this is not json"
        return super().text(name, body, parse=parse)


class Skipping(campaign_runner.Runner):
    """A runner that loses one scheduled item, as a dropped worker would."""

    def __init__(self, *args, skip, **kwargs):
        super().__init__(*args, **kwargs)
        self.skip = skip

    async def _program_item(self, name, item, checker):
        if item.id == self.skip:
            return
        await super()._program_item(name, item, checker)


def run(runner):
    return asyncio.run(runner.run())


def build(directory, *, checker=None, cleanup=None, artifacts=None, cls=None, **extra):
    selected = profile()
    plan = campaign_plan.schedule(selected)
    store = artifacts or Artifacts(directory)
    runner = (cls or campaign_runner.Runner)(
        selected,
        plan,
        store,
        checkers={0: checker or Checker()},
        cleanup=cleanup,
        root=ROOT,
        **extra,
    )
    return runner, plan


class CompleteRunTest(unittest.TestCase):
    # [spec:pgorm:req:generative.verdict/test]
    def test_a_complete_run_records_every_item(self):
        with tempfile.TemporaryDirectory() as directory:
            runner, plan = build(directory)
            document = run(runner)
            self.assertTrue(document["passed"], document["aggregate"]["faults"])
            self.assertEqual(document["work"]["recorded_total"], len(plan.items))
            self.assertEqual(document["aggregate"]["faults"], [])
            self.assertTrue(document["aggregate"]["all_work_accounted"])

    # [spec:pgorm:req:generative.profiles/test]
    def test_construction_counts_are_not_database_counts(self):
        with tempfile.TemporaryDirectory() as directory:
            runner, _ = build(directory)
            counts = run(runner)["counts"]
            self.assertEqual(
                counts["construction_only_programs_constructed"]["recorded"], 2
            )
            self.assertFalse(
                counts["construction_only_programs_constructed"]["reached_database"]
            )
            self.assertTrue(
                counts["live_database_programs_checked"]["reached_database"]
            )
            self.assertNotIn("total", counts)
            self.assertIn("never summed", counts["note"])

    # [spec:pgorm:req:generative.artifacts/test]
    def test_the_report_records_identity_and_omits_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            runner, _ = build(directory)
            document = run(runner)
            self.assertTrue(document["credentials_omitted"])
            self.assertEqual(document["versions"], {})
            self.assertEqual(document["seeds"]["items"], [20260913])
            self.assertTrue(Path(directory, "campaign.json").exists())
            self.assertTrue(Path(directory, "plan.json").exists())
            written = json.loads(Path(directory, "campaign.json").read_text())
            self.assertEqual(written["kind"], "campaign-run")

    # [spec:pgorm:req:generative.artifacts/test]
    def test_construction_evidence_states_what_it_is(self):
        with tempfile.TemporaryDirectory() as directory:
            runner, _ = build(directory)
            run(runner)
            body = json.loads(Path(directory, "construction/programs.json").read_text())
            self.assertIn("constructor calls only", body["claim"])
            self.assertEqual(body["construction_only_programs_constructed"], 2)
            self.assertEqual(len(body["programs"]), 2)


class FailurePathTest(unittest.TestCase):
    def kinds(self, document):
        return set(document["aggregate"]["fault_kinds"])

    # [spec:pgorm:req:generative.verdict/test]
    def test_empty_discovery_fails_the_run(self):
        with tempfile.TemporaryDirectory() as directory:
            selected = profile()
            runner = campaign_runner.Runner(
                selected,
                campaign_plan.Plan("narrow", ()),
                Artifacts(directory),
                checkers={0: Checker()},
                root=ROOT,
            )
            document = run(runner)
            self.assertFalse(document["passed"])
            self.assertIn("empty-discovery", self.kinds(document))

    # [spec:pgorm:req:generative.verdict/test]
    def test_a_skipped_item_fails_the_run(self):
        with tempfile.TemporaryDirectory() as directory:
            runner, plan = build(
                directory, cls=Skipping, skip=plan_first(directory) or "runtime-1"
            )
            document = run(runner)
            self.assertFalse(document["passed"])
            self.assertIn("skipped-work", self.kinds(document))
            self.assertEqual(document["work"]["recorded_total"], len(plan.items) - 1)

    # [spec:pgorm:req:generative.verdict/test]
    def test_a_malformed_artifact_fails_the_run(self):
        with tempfile.TemporaryDirectory() as directory:
            store = Corrupting(directory, "runtime/runtime-0.oracle.json")
            runner, _ = build(directory, artifacts=store)
            document = run(runner)
            self.assertFalse(document["passed"])
            self.assertIn("artifact-malformed", self.kinds(document))

    # [spec:pgorm:req:generative.verdict/test]
    def test_a_cleanup_failure_fails_the_run(self):
        async def cleanup():
            raise RuntimeError("owned fixture cleanup failed")

        with tempfile.TemporaryDirectory() as directory:
            runner, _ = build(directory, cleanup=cleanup)
            document = run(runner)
            self.assertFalse(document["passed"])
            self.assertIn("cleanup-failure", self.kinds(document))

    # [spec:pgorm:req:generative.verdict/test]
    def test_reported_cleanup_errors_fail_the_run(self):
        async def cleanup():
            return ["container removal failed"]

        with tempfile.TemporaryDirectory() as directory:
            runner, _ = build(directory, cleanup=cleanup)
            document = run(runner)
            self.assertIn("cleanup-failure", self.kinds(document))

    # [spec:pgorm:req:generative.verdict/test]
    def test_a_native_panic_fails_the_run(self):
        with tempfile.TemporaryDirectory() as directory:
            checker = Checker(
                status="incomplete",
                error={"class": "UnexpectedNativePanic", "cause": "panicked at"},
            )
            runner, _ = build(directory, checker=checker)
            document = run(runner)
            self.assertFalse(document["passed"])
            self.assertIn("native-panic", self.kinds(document))

    # [spec:pgorm:req:generative.verdict/test]
    def test_a_deadline_fails_the_run(self):
        with tempfile.TemporaryDirectory() as directory:
            checker = Checker(
                status="incomplete", error={"class": "TimeoutError", "cause": ""}
            )
            runner, _ = build(directory, checker=checker)
            document = run(runner)
            self.assertFalse(document["passed"])
            self.assertIn("deadline", self.kinds(document))

    # [spec:pgorm:req:generative.verdict/test]
    def test_a_missing_checker_stops_the_run(self):
        with tempfile.TemporaryDirectory() as directory:
            selected = profile()
            runner = campaign_runner.Runner(
                selected,
                campaign_plan.schedule(selected),
                Artifacts(directory),
                checkers={},
                root=ROOT,
            )
            with self.assertRaises(campaign_runner.RunnerError):
                run(runner)

    # [spec:pgorm:req:generative.verdict/test]
    def test_unsatisfied_coverage_fails_the_run(self):
        with tempfile.TemporaryDirectory() as directory:
            checker = Checker(status="incomplete")
            runner, _ = build(directory, checker=checker)
            document = run(runner)
            self.assertFalse(document["passed"])
            self.assertIn("coverage-unsatisfied", self.kinds(document))
            self.assertTrue(document["coverage"]["declared_missing"])


class RetentionTest(unittest.TestCase):
    # [spec:pgorm:req:generative.verdict/test]
    def test_a_defect_produces_a_retained_finding(self):
        with tempfile.TemporaryDirectory() as directory:
            runner, _ = build(directory, checker=Checker(status="defect"))
            document = run(runner)
            self.assertFalse(document["passed"])
            self.assertEqual(len(document["findings"]), 2)
            self.assertEqual(document["findings"][0]["verdict"], "defect")
            self.assertIn("unexpected-verdict", document["aggregate"]["fault_kinds"])

    # [spec:pgorm:req:generative.artifacts/test]
    @unittest.skipIf(
        sys.version_info < (3, 12), "reproducer emission needs relative_to(walk_up=)"
    )
    def test_a_defect_retains_program_and_replay_commands(self):
        with tempfile.TemporaryDirectory() as directory:
            runner, _ = build(directory, checker=Checker(status="defect"))
            document = run(runner)
            self.assertFalse(document["passed"])
            self.assertTrue(document["findings"])
            finding = document["findings"][0]
            self.assertEqual(finding["run_class"], "runtime")
            retained = Path(finding["directory"])
            self.assertTrue((retained / "replay/program.json").exists())
            self.assertTrue((retained / "replay/python.py").exists())
            self.assertTrue((retained / "replay/rust/src/main.rs").exists())
            self.assertTrue((retained / "attribution/manifest.json").exists())
            self.assertTrue((retained / "shrink.json").exists())
            self.assertIn("rust_replay", finding["commands"])
            self.assertIn("python_replay", finding["commands"])

    # [spec:pgorm:req:generative.artifacts/test]
    def test_retained_evidence_carries_no_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            runner, _ = build(directory, checker=Checker(status="defect"))
            document = run(runner)
            body = json.dumps(document)
            self.assertNotIn("postgresql://", body)
            self.assertTrue(document["credentials_omitted"])


def plan_first(_directory):
    """The identity of the first live item, so a test can lose exactly it."""
    selected = profile()
    return campaign_plan.schedule(selected).by_class("runtime")[0].id


if __name__ == "__main__":
    unittest.main()
