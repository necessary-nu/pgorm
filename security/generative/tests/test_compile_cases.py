import unittest
from pathlib import Path

from pgorm_campaign import compile_crate, compile_suite
from pgorm_campaign.compile_case import (
    CompileCase,
    CompileCaseError,
    Expectation,
    rejects,
    validated,
)


def _case(identity="a", **overrides):
    fields = {
        "id": identity,
        "obligation": "rust-ownership",
        "verdict": "accept",
        "phase": "typeck",
        "source": "    pub fn f() {}",
    }
    fields.update(overrides)
    return CompileCase(**fields)


# [spec:pgorm:req:generative.compile-suite/test]
class CompileCaseTests(unittest.TestCase):
    def test_expectation_requires_a_code_or_message(self):
        with self.assertRaises(CompileCaseError):
            Expectation()
        with self.assertRaises(CompileCaseError):
            Expectation(codes=("not-a-code",))
        self.assertTrue(rejects("E0277").coded)
        self.assertFalse(rejects(message="lifetime").coded)

    def test_expectation_matches_code_then_message(self):
        coded = rejects("E0499", "E0502")
        self.assertTrue(coded.matches("E0502", "anything"))
        self.assertFalse(coded.matches("E0308", "anything"))
        worded = rejects(message="lifetime may not live long enough")
        self.assertTrue(worded.matches("", "lifetime may not live long enough"))
        self.assertFalse(worded.matches("", "mismatched types"))

    def test_rejecting_case_must_predict_its_rejection(self):
        with self.assertRaises(CompileCaseError):
            _case(verdict="reject")
        with self.assertRaises(CompileCaseError):
            _case(expects=rejects("E0277"))

    def test_only_generation_can_be_refused(self):
        with self.assertRaises(CompileCaseError):
            _case(verdict="refuse", expects=rejects(message="bad"))
        refused = _case(
            verdict="refuse",
            kind="codegen",
            source="",
            request={"id": "a", "sql": "CREATE TABLE t (id serial PRIMARY KEY);"},
            expects=rejects(message="bad"),
        )
        self.assertEqual(refused.verdict, "refuse")

    def test_unknown_phase_verdict_and_kind_refused(self):
        for field in ("verdict", "phase", "kind"):
            with self.assertRaises(CompileCaseError):
                _case(**{field: "nonsense"})

    def test_digest_tracks_content_not_identity(self):
        first = _case("a")
        again = _case("a")
        changed = _case("a", source="    pub fn g() {}")
        self.assertEqual(first.digest, again.digest)
        self.assertNotEqual(first.digest, changed.digest)

    def test_duplicate_case_identities_are_refused(self):
        with self.assertRaises(CompileCaseError):
            validated([_case("a"), _case("a")])
        self.assertEqual(len(validated([_case("a"), _case("b")])), 2)


# [spec:pgorm:req:generative.compile-suite/test]
class SuiteTests(unittest.TestCase):
    def test_every_compile_only_obligation_is_claimed(self):
        declared = compile_suite.compile_only()
        self.assertEqual(
            set(declared),
            {"new-rust-entities", "rust-ownership", "fresh-graph-generics"},
        )
        claimed = {case.obligation for case in compile_suite.cases()}
        self.assertEqual(claimed, set(declared))

    def test_suite_carries_positives_negatives_and_refusals(self):
        verdicts = {}
        for case in compile_suite.cases():
            verdicts[case.verdict] = verdicts.get(case.verdict, 0) + 1
        self.assertGreater(verdicts["accept"], 0)
        self.assertGreater(verdicts["reject"], 0)
        self.assertGreater(verdicts["refuse"], 0)

    def test_every_rejection_names_a_rustc_error_code(self):
        uncoded = [
            case.id
            for case in compile_suite.cases()
            if case.verdict == "reject" and not case.expects.coded
        ]
        # The region rejections rustc assigns no code to, and the derive's
        # own refusals, which reach rustc as `compile_error!` and so carry no
        # code either; anything else appearing here means a case reached for
        # prose where a code exists.
        self.assertEqual(
            sorted(uncoded),
            [
                "binder-brand-crossed",
                "entity-key-temporal-alone",
                "entity-relation-key-quoted",
                "entity-relation-period-half",
                "txn-escapes-closure",
            ],
        )

    def test_split_separates_codegen_from_source(self):
        source, codegen = compile_suite.split(compile_suite.cases())
        self.assertTrue(all(case.kind == "source" for case in source))
        self.assertTrue(all(case.kind == "codegen" for case in codegen))
        self.assertGreater(len(codegen), 0)

    def test_coverage_needs_every_case_satisfied(self):
        results = [
            {"obligation": "rust-ownership", "status": "compiled"},
            {"obligation": "rust-ownership", "status": "unrejected"},
            {"obligation": "new-rust-entities", "status": "expected-rejection"},
        ]
        coverage = compile_suite.coverage(results)
        self.assertFalse(coverage["rust-ownership"]["discharged"])
        self.assertTrue(coverage["new-rust-entities"]["discharged"])
        self.assertFalse(coverage["fresh-graph-generics"]["discharged"])


# [spec:pgorm:req:generative.compile-suite/test]
class BatchingTests(unittest.TestCase):
    def test_phases_and_verdicts_never_share_a_crate(self):
        cases = [
            _case("a", phase="typeck"),
            _case("b", phase="borrowck"),
            _case("c", verdict="reject", phase="typeck", expects=rejects("E0277")),
        ]
        batches = compile_crate.group(cases)
        self.assertEqual(len(batches), 3)
        for batch in batches:
            kinds = {(case.verdict, case.phase) for case in batch.cases}
            self.assertEqual(len(kinds), 1)

    def test_batches_are_bounded_in_size(self):
        cases = [_case(f"c{index}") for index in range(7)]
        batches = compile_crate.group(cases, limit=3)
        self.assertEqual([len(batch.placements) for batch in batches], [3, 3, 1])

    def test_rendered_lines_locate_each_case(self):
        cases = [_case("first"), _case("second", source="    pub fn g() {}\n")]
        batch = compile_crate.group(cases)[0]
        text = compile_crate.render_source(batch)
        lines = text.splitlines()
        for placement in batch.placements:
            body = "\n".join(lines[placement.start - 1 : placement.end])
            self.assertIn(placement.case.source.strip(), body)
            self.assertIsNotNone(batch.locate("src/lib.rs", placement.start))
        self.assertIsNone(batch.locate("src/lib.rs", 1))

    def test_crate_name_follows_batch_content(self):
        first = compile_crate.group([_case("a")])[0]
        same = compile_crate.group([_case("a")])[0]
        other = compile_crate.group([_case("a", source="    pub fn g() {}")])[0]
        self.assertEqual(
            compile_crate.batch_digest(first), compile_crate.batch_digest(same)
        )
        self.assertNotEqual(
            compile_crate.batch_digest(first), compile_crate.batch_digest(other)
        )

    def test_manifest_spells_only_known_dependencies(self):
        batch = compile_crate.group([_case("a", needs=("serde",))])[0]
        manifest = compile_crate.render_manifest(batch, crate="x-1", root="/tmp/pgorm")
        self.assertIn('serde = { version = "1"', manifest)
        broken = compile_crate.group([_case("a", needs=("nope",))])[0]
        with self.assertRaises(compile_crate.BatchError):
            compile_crate.render_manifest(broken, crate="x-1", root="/tmp/pgorm")

    def test_manifest_path_dependency_is_absolute(self):
        # A relative root resolves against the emitted crate's own directory,
        # so the dependency would point at the crate itself.
        batch = compile_crate.group([_case("a")])[0]
        manifest = compile_crate.render_manifest(batch, crate="x-1", root=".")
        spelled = manifest.split('pgorm = { path = "')[1].split('"')[0]
        self.assertTrue(Path(spelled).is_absolute())
        self.assertEqual(Path(spelled), Path.cwd().resolve())


if __name__ == "__main__":
    unittest.main()
