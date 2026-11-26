"""Coverage is discharged by observed construction and dispatch, not by labels."""

import unittest

from pgorm_campaign import (
    campaign_coverage,
    campaign_plan,
    grammar,
    matrix,
    profiles,
    wire,
)


def executed(program, *, registration=None):
    """A synthetic report in which every instruction was actually dispatched."""
    data = program.data()
    trace = [
        {
            "id": node["id"],
            "operation": node["op"],
            "status": "constructed",
            "native_paths": ["pgorm_query::Value"],
            "observation": {},
        }
        for node in data["nodes"]
    ]
    if registration is not None:
        trace.append(
            {
                "id": "registered",
                "status": "constructed",
                "native_paths": ["pgorm::Entity"],
                "observation": {"registration": registration},
            }
        )
    return {
        "status": "pass",
        "subject": {
            "trace": trace,
            "steps": [
                {"id": step["id"], "status": "observed", "native_paths": ["pgorm"]}
                for step in data["steps"]
            ],
            "cleanup_errors": [],
            "builds": 0,
        },
        "comparisons": [],
    }


def unexecuted(program):
    return {"status": "incomplete", "subject": {"trace": [], "steps": []}}


class ObservedCoverageTest(unittest.TestCase):
    # [spec:pgorm:req:generative.verdict/test]
    def test_dispatched_operations_and_effects_are_observed(self):
        program = grammar.generate(20260913, 0, family="select").program
        tokens = campaign_coverage.observed(program.data(), executed(program))
        data = program.data()
        for node in data["nodes"]:
            self.assertIn("operation." + node["op"], tokens)
        for step in data["steps"]:
            self.assertIn("effect." + step["op"], tokens)

    # [spec:pgorm:req:generative.verdict/test]
    def test_a_program_that_never_ran_observes_nothing(self):
        program = grammar.generate(20260913, 0, family="select").program
        self.assertEqual(
            campaign_coverage.observed(program.data(), unexecuted(program)), set()
        )

    # [spec:pgorm:req:generative.verdict/test]
    def test_value_contexts_come_from_the_instruction(self):
        program = grammar.generate(20260913, 8, family="types").program
        tokens = campaign_coverage.observed(program.data(), executed(program))
        contexts = {
            token.split(".")[-1] for token in tokens if token.startswith("value.")
        }
        self.assertTrue(contexts)
        self.assertTrue(contexts <= {"literal", "bound", "typed-null", "result-decode"})

    # [spec:pgorm:req:generative.matrix/test]
    def test_every_decode_cell_is_generated(self):
        tokens = set()
        for index in range(400):
            program = grammar.generate(20260913, index, family="types").program
            tokens |= campaign_coverage.observed(program.data(), executed(program))
        declared = {
            token
            for token in matrix.obligations()
            if token.startswith("value.") and token.endswith(".result-decode")
        }
        self.assertEqual(declared - tokens, set())

    # [spec:pgorm:req:generative.verdict/test]
    def test_registration_evidence_is_attributed(self):
        program = grammar.generate(20260913, 3, family="entity").program
        tokens = campaign_coverage.observed(
            program.data(), executed(program, registration="campaign.Account")
        )
        self.assertIn("registered_entities.campaign.Account", tokens)


class FamilyVariationTest(unittest.TestCase):
    """Family cells are discharged by construction evidence, like every token."""

    def tokens(self, family, index=0):
        mode = "invalid" if family == "rejection" else "valid"
        program = grammar.generate(20260913, index, family=family, mode=mode).program
        return campaign_coverage.observed(program.data(), executed(program))

    # [spec:pgorm:req:generative.matrix/test]
    def test_declared_families_are_reachable_from_generated_programs(self):
        # The types family draws one kind per program from the whole value
        # matrix, and some of its cells need one payload of one kind (a signed
        # zero is two of the float corpus's 26 patterns), so it takes more
        # programs than a family whose cells are structural.
        budgets = {"types": 400}
        reached = set()
        for family in (*grammar.FAMILIES, "rejection"):
            for index in range(budgets.get(family, 60)):
                reached |= self.tokens(family, index)
        declared = {
            item["id"] + "." + variation
            for item in matrix.load()["families"]
            for variation in item["variations"]
        }
        # Construction alone cannot evidence cells that need a live observation
        # (a decoded graph row, a stream's ending), so those are excluded here
        # and covered by the runtime campaign instead.
        observed_only = {
            "entities.hooks",
            "entities.upsert-inserted",
            "entities.upsert-updated",
            "graph.absent-source",
            "graph.model-decode",
            "sequences.stream-cancel",
            "sequences.stream-complete",
            "sequences.stream-early-close",
        }
        self.assertEqual(declared - reached - observed_only, set())

    def hooked(self, suffix):
        """An Account insert whose observed name is its declared name + suffix."""
        for index in range(60):
            program = grammar.generate(20260913, index, family="active").program
            data = program.data()
            step = data["steps"][0]
            if step["op"] != "active.write" or step["data"]["method"] != "insert":
                continue
            nodes = {node["id"]: node for node in data["nodes"]}
            node = nodes[step["inputs"]["model"]]
            while node["op"] == "active.set" and node["data"]["column"] != "name":
                node = nodes[node["inputs"]["model"]]
            if node["op"] != "active.set":
                continue
            declared = nodes[node["inputs"]["value"]]["data"]["value"]["data"]
            report = executed(program)
            report["subject"]["steps"][0]["observation"] = {
                "kind": "rows",
                "rows": [
                    {
                        "kind": "record",
                        "fields": [
                            {
                                "name": "name",
                                "value": wire.scalar("text", declared + suffix),
                            }
                        ],
                    }
                ],
            }
            return campaign_coverage.observed(data, report)
        self.fail("the active family never inserted a named Account")

    # [spec:pgorm:req:generative.matrix/test]
    def test_an_empty_batch_omits_no_write(self):
        seen = 0
        for index in range(80):
            generated = grammar.generate(20260913, index, mode="invalid")
            if generated.recipe()["rejection_case"] != "empty-batch":
                continue
            program = generated.program
            tokens = campaign_coverage.observed(program.data(), executed(program))
            self.assertIn("crud.empty-batch", tokens)
            self.assertNotIn("crud.omitted-write", tokens)
            seen += 1
        self.assertTrue(seen)

    # [spec:pgorm:req:generative.matrix/test]
    def test_observed_hook_rewrite_is_attributed(self):
        self.assertIn("entities.hooks", self.hooked("|hook"))
        self.assertNotIn("entities.hooks", self.hooked(""))
        self.assertNotIn("entities.hooks", self.hooked("|other"))

    def upserted(self, old):
        """An upsert whose one observed row's old version is `old`."""
        for index in range(60):
            program = grammar.generate(20260913, index, family="active").program
            data = program.data()
            positions = [
                position
                for position, step in enumerate(data["steps"])
                if step["op"] == "entity.upsert"
            ]
            if not positions:
                continue
            report = executed(program)
            record = {"kind": "record", "fields": []}
            report["subject"]["steps"][positions[0]]["observation"] = {
                "kind": "rows",
                "rows": [{"kind": "tuple", "items": [old, record]}],
            }
            return campaign_coverage.observed(data, report)
        self.fail("the active family never upserted")

    # [spec:pgorm:req:generative.matrix/test]
    def test_upsert_action_read_off_old_row(self):
        inserted = self.upserted({"kind": "absent"})
        self.assertIn("entities.upsert-inserted", inserted)
        self.assertNotIn("entities.upsert-updated", inserted)
        updated = self.upserted({"kind": "record", "fields": []})
        self.assertIn("entities.upsert-updated", updated)
        self.assertNotIn("entities.upsert-inserted", updated)

    # [spec:pgorm:req:generative.verdict/test]
    def test_an_unrun_program_attributes_no_variation(self):
        program = grammar.generate(20260913, 0, family="sources").program
        tokens = campaign_coverage.observed(program.data(), unexecuted(program))
        self.assertEqual({token for token in tokens if "." in token}, set())

    # [spec:pgorm:req:generative.matrix/test]
    def test_every_variation_token_is_a_declared_obligation(self):
        declared = matrix.obligations()
        families = {item["id"] for item in matrix.load()["families"]}
        for family in grammar.FAMILIES:
            for index in range(12):
                for token in self.tokens(family, index):
                    if token.split(".")[0] in families:
                        self.assertIn(token, declared)

    # [spec:pgorm:req:generative.verdict/test]
    def test_pipeline_source_registrations_are_attributed(self):
        for index in range(24):
            tokens = self.tokens("sources", index)
            found = {
                token for token in tokens if token.startswith("registered_sources.")
            }
            if found:
                self.assertTrue(found <= matrix.obligations())
                return
        self.fail("the sources family never attributed a registered source list")


class RetiredCellTest(unittest.TestCase):
    """A withdrawn value cell leaves the matrix, and says why in the matrix."""

    # [spec:pgorm:req:generative.matrix/test]
    def test_retired_cells_are_not_obligations(self):
        declared = matrix.obligations()
        for kind in ("u64", "vector", "char"):
            self.assertNotIn("value." + kind + ".result-decode", declared)
            self.assertIn("value." + kind + ".literal", declared)

    # [spec:pgorm:req:generative.matrix/test]
    def test_every_retired_cell_records_a_reason(self):
        for item in matrix.load()["value_exclusions"]:
            self.assertTrue(item["reason"].strip())

    # [spec:pgorm:req:generative.matrix/test]
    def test_a_retired_cell_names_a_real_cell(self):
        document = matrix.load()
        for broken in (
            {"space": "value", "kind": "nope", "context": "literal", "reason": "x"},
            {"space": "value", "kind": "u64", "context": "nope", "reason": "x"},
            {"space": "nope", "kind": "u64", "context": "literal", "reason": "x"},
            {"space": "value", "kind": "u64", "context": "literal", "reason": ""},
        ):
            with self.subTest(broken=broken):
                candidate = dict(document, value_exclusions=[broken])
                with self.assertRaises(wire.FormatError):
                    matrix.retired(candidate)


class DeclaredCoverageTest(unittest.TestCase):
    def setUp(self):
        self.profile = profiles.select("smoke")
        self.plan = campaign_plan.schedule(self.profile)

    # [spec:pgorm:req:generative.profiles/test]
    def test_smoke_requires_each_scheduled_family(self):
        required = campaign_coverage.required(self.profile, self.plan)
        self.assertIn("family.select", required)
        self.assertIn("family.rejection", required)
        self.assertIn("class.control", required)
        self.assertNotIn("operation.value", required)

    # [spec:pgorm:req:generative.profiles/test]
    def test_full_requires_the_whole_matrix(self):
        profile = profiles.select("full")
        required = campaign_coverage.required(profile, campaign_plan.schedule(profile))
        self.assertIn("operation.value", required)
        self.assertIn("value.bool.literal", required)
        self.assertGreater(len(required), 400)

    # [spec:pgorm:req:generative.verdict/test]
    def test_missing_family_leaves_coverage_unsatisfied(self):
        required = campaign_coverage.required(self.profile, self.plan)
        result = campaign_coverage.assess(
            self.profile, self.plan, required - {"family.pipeline"}
        )
        self.assertFalse(result["satisfied"])
        self.assertEqual(result["declared_missing"], ["family.pipeline"])

    # [spec:pgorm:req:generative.artifacts/test]
    def test_smoke_records_the_full_matrix_gap(self):
        required = campaign_coverage.required(self.profile, self.plan)
        result = campaign_coverage.assess(self.profile, self.plan, required)
        self.assertTrue(result["satisfied"])
        self.assertFalse(result["full_matrix"]["claimed"])
        self.assertGreater(result["full_matrix"]["outstanding"], 0)
        self.assertIn("does not establish", result["claim"])


if __name__ == "__main__":
    unittest.main()
