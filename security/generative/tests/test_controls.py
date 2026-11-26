import copy
import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from pgorm_campaign import comparison, control_mutations, control_verdict, oracles, wire
from pgorm_campaign.control_catalog import REQUIRED_FAMILIES, catalog
from pgorm_campaign.controls import Controls


def baseline_report(spec):
    rows = [
        {
            "kind": "record",
            "fields": [{"name": "id", "value": wire.scalar("i32", str(value))}],
        }
        for value in (1, 2, 4)
    ]
    report = {
        "program_sha256": spec.program.digest,
        "status": "executed",
        "steps": [
            {
                "id": "s0",
                "status": "observed",
                "native_paths": ["pgorm::Connection::query_raw"],
                "observation": {"kind": "rows", "rows": rows},
            }
        ],
        "cleanup_errors": [],
    }
    return {
        "program_sha256": spec.program.digest,
        "status": "pass",
        "subject": report,
        "reference": copy.deepcopy(report),
        "subject_state": {"tables": []},
        "reference_state": {"tables": []},
    }


# [spec:pgorm:req:generative.controls/test]
class ControlTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.spec = next(spec for spec in catalog() if spec.id == "missing-row")
        self.baseline = baseline_report(self.spec)
        self.controls = Controls(SimpleNamespace())
        self.controls.checker.run = AsyncMock(return_value=self.baseline)

    async def test_intended_mutation_requires_a_passing_baseline(self):
        result = await self.controls.run(self.spec)
        self.assertEqual(result["status"], "valid-control")
        self.assertTrue(control_verdict.verified(self.spec, result))
        self.baseline["status"] = "defect"
        result = await self.controls.run(self.spec)
        self.assertEqual(result["status"], "invalid-control")
        self.assertEqual(result["dispatch_count"], 0)

    async def test_inactive_native_dispatch_invalidates_control(self):
        self.baseline["subject"]["steps"][0]["native_paths"] = []
        result = await self.controls.run(self.spec)
        self.assertEqual(result["status"], "invalid-control")
        self.assertEqual(result["dispatch_count"], 0)

    async def test_noop_and_weakened_oracle_fail_closed(self):
        with patch.object(
            control_mutations,
            "observation",
            side_effect=lambda report, identity: report,
        ):
            result = await self.controls.run(self.spec)
        self.assertEqual(result["status"], "invalid-control")
        self.assertIn("no observable change", result["reason"])
        with patch.object(
            comparison, "rows", return_value=comparison.Comparison(True, "equal")
        ):
            result = await self.controls.run(self.spec)
        self.assertEqual(result["status"], "invalid-control")
        self.assertIn("intended control failure", result["reason"])

    async def test_arbitrary_failure_is_not_control_detection(self):
        with patch.object(
            control_mutations,
            "observation",
            side_effect=RuntimeError("inactive implementation"),
        ):
            result = await self.controls.run(self.spec)
        self.assertEqual(result["status"], "incomplete")
        self.assertEqual(result["dispatch_count"], 0)
        self.baseline["reference"]["program_sha256"] = "wrong-program"
        result = await self.controls.run(self.spec)
        self.assertEqual(result["status"], "invalid-control")

    async def test_summary_rechecks_evidence_and_expected_work(self):
        result = await self.controls.run(self.spec)
        required = {self.spec.family}
        self.assertTrue(
            control_verdict.summary([self.spec], [result], required=required)["passed"]
        )
        for specs, reports in (
            ([], []),
            ([self.spec], []),
            ([self.spec, self.spec], [result, result]),
            ([self.spec], [{"id": self.spec.id, "status": "valid-control"}]),
        ):
            self.assertFalse(
                control_verdict.summary(specs, reports, required=required)["passed"]
            )
        changed = copy.deepcopy(result)
        changed["comparisons"][0]["equal"] = True
        self.assertFalse(
            control_verdict.summary([self.spec], [changed], required=required)["passed"]
        )
        self.assertFalse(control_verdict.summary([self.spec], [result])["passed"])

    def test_control_marker_cannot_replace_native_evidence(self):
        subject = control_verdict.subject(self.baseline["subject"], self.spec.id)
        with self.assertRaises(comparison.InvalidOracle):
            oracles.compare(
                self.spec.program.data(), subject, self.baseline["reference"], {}, {}
            )
        with self.assertRaises(comparison.InvalidOracle):
            oracles.compare(
                self.spec.program.data(),
                subject,
                self.baseline["reference"],
                {},
                {},
                control="different-control",
            )

    def test_catalog_covers_all_mandatory_control_families(self):
        specs = catalog()
        self.assertEqual(len(specs), len({spec.id for spec in specs}))
        self.assertEqual({spec.family for spec in specs}, REQUIRED_FAMILIES)
