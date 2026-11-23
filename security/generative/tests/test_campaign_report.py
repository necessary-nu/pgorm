"""The report keeps run classes in separate fields and carries no credentials."""

import unittest

from pgorm_campaign import campaign_plan, campaign_report, campaign_verdict, profiles


def records(plan):
    return [
        {
            **item.data(),
            "verdict": item.expected,
            "native_paths": ["pgorm::ConnectionTrait::query_raw"]
            if item.run_class in ("runtime", "invalid", "control")
            else [],
        }
        for item in plan.items
    ]


def assemble(profile, plan, items, **overrides):
    arguments = {
        "identity": {"profile": profile.identity(), "class_claims": profiles.claims()},
        "postgres": {
            "image": "postgres:19.0-trixie",
            "settings": {"timezone": "UTC"},
        },
        "coverage": {
            "obligations": "scheduled-families",
            "claim": "sampled",
            "declared_total": 2,
            "declared_satisfied": 2,
            "declared_missing": [],
            "satisfied": True,
            "observed_total": 2,
            "full_matrix": {
                "total": 451,
                "satisfied": 10,
                "outstanding": 441,
                "outstanding_sample": [],
                "claimed": False,
            },
        },
        "controls": {"passed": True, "expected": 22, "executed": 22},
        "compile_document": None,
        "verdicts": campaign_verdict.counts(items),
        "timing": {"total_seconds": 12.5},
        "aggregate": {
            "passed": True,
            "faults": [],
            "fault_kinds": [],
            "all_work_accounted": True,
        },
        "artifacts": {"directory": "target/run", "files": {}, "faults": []},
    }
    arguments.update(overrides)
    return campaign_report.assemble(profile, plan, items, **arguments)


class ReportShapeTest(unittest.TestCase):
    def setUp(self):
        self.profile = profiles.select("smoke")
        self.plan = campaign_plan.schedule(self.profile)
        self.items = records(self.plan)
        self.document = assemble(self.profile, self.plan, self.items)

    # [spec:pgorm:req:generative.profiles/test]
    def test_each_run_class_has_a_named_count(self):
        counts = self.document["counts"]
        self.assertEqual(
            counts["construction_only_programs_constructed"]["scheduled"], 150
        )
        self.assertEqual(counts["live_database_programs_checked"]["scheduled"], 30)
        self.assertEqual(counts["invalid_input_programs_checked"]["scheduled"], 6)
        self.assertEqual(counts["control_programs_checked"]["scheduled"], 23)

    # [spec:pgorm:req:generative.profiles/test]
    def test_the_report_never_totals_across_run_classes(self):
        numbers = [
            entry["recorded"]
            for key, entry in self.document["counts"].items()
            if key != "note"
        ]
        self.assertNotIn(sum(numbers), numbers)
        for field in campaign_report.CLASS_FIELDS.values():
            self.assertNotIn(field, ("programs", "total", "checked"))

    # [spec:pgorm:req:generative.artifacts/test]
    def test_the_report_records_identity_and_seeds(self):
        self.assertEqual(self.document["profile"]["name"], "smoke")
        self.assertEqual(self.document["seeds"]["items"], [20260913])
        self.assertEqual(self.document["postgres"]["image"], "postgres:19.0-trixie")
        self.assertIn("construction", self.document["class_claims"])

    # [spec:pgorm:req:generative.artifacts/test]
    def test_campaign_builds_are_counted_apart(self):
        self.assertEqual(self.document["builds"]["extension_builds_during_campaign"], 0)
        built = assemble(
            self.profile,
            self.plan,
            [{**record, "builds": 1} for record in self.items],
        )
        self.assertEqual(
            built["builds"]["extension_builds_during_campaign"], len(self.items)
        )

    # [spec:pgorm:req:generative.verdict/test]
    def test_native_evidence_counts_live_items_only(self):
        evidence = self.document["native_evidence"]
        self.assertEqual(evidence["live_items"], 59)
        self.assertEqual(evidence["live_items_with_native_dispatch"], 59)
        self.assertEqual(evidence["distinct_native_paths"], 1)

    # [spec:pgorm:req:generative.profiles/test]
    def test_an_excluded_compile_suite_records_its_reason(self):
        section = self.document["compile"]
        self.assertFalse(section["included"])
        self.assertTrue(section["reason"])
        self.assertEqual(section["compile_cases"], 0)

    # [spec:pgorm:req:generative.verdict/test]
    def test_compile_counts_keep_their_own_prefix(self):
        profile = profiles.select("full")
        plan = campaign_plan.schedule(profile)
        document = assemble(
            profile,
            plan,
            records(plan),
            compile_document={
                "passed": True,
                "compile_cases": 30,
                "compile_positive": 12,
                "defects": [],
                "faults": [],
                "unattributed": 0,
            },
        )
        section = document["compile"]
        self.assertEqual(section["compile_cases"], 30)
        self.assertTrue(
            all(key.startswith("compile_") for key in section if key[0] == "c")
        )
        self.assertNotIn("live_database_programs_checked", section)


class CredentialTest(unittest.TestCase):
    # [spec:pgorm:req:generative.artifacts/test]
    def test_a_leaked_connection_string_fails_the_report(self):
        profile = profiles.select("smoke")
        plan = campaign_plan.schedule(profile)
        document = assemble(
            profile,
            plan,
            records(plan),
            postgres={"dsn": "postgresql://campaign:hunter2@127.0.0.1:5432/worker"},
        )
        self.assertFalse(document["passed"])
        self.assertFalse(document["credentials_omitted"])
        self.assertTrue(document["aggregate"]["faults"])

    # [spec:pgorm:req:generative.artifacts/test]
    def test_a_clean_report_declares_credentials_omitted(self):
        profile = profiles.select("smoke")
        plan = campaign_plan.schedule(profile)
        document = assemble(profile, plan, records(plan))
        self.assertTrue(document["credentials_omitted"])
        self.assertEqual(campaign_report.credentials_absent(document), [])


class RenderTest(unittest.TestCase):
    # [spec:pgorm:req:generative.verdict/test]
    def test_render_names_classes_and_matrix_gap(self):
        profile = profiles.select("smoke")
        plan = campaign_plan.schedule(profile)
        text = campaign_report.render(assemble(profile, plan, records(plan)))
        self.assertIn("construction_only_programs_constructed", text)
        self.assertIn("live_database_programs_checked", text)
        self.assertIn("441 full-matrix obligations outstanding", text)
        self.assertIn("passed", text)


if __name__ == "__main__":
    unittest.main()
