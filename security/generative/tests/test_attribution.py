import copy
import tempfile
import unittest
from pathlib import Path

from pgorm_campaign import attribution, control_programs, oracles, wire


def report(program, *, value="2", status="defect"):
    observation = {
        "kind": "rows",
        "rows": [
            {
                "kind": "record",
                "fields": [{"name": "id", "value": wire.scalar("i32", value)}],
            }
        ],
    }
    result = {
        "program_sha256": program.digest,
        "status": status,
        "subject": {
            "program_sha256": program.digest,
            "status": "executed",
            "steps": [
                {
                    "id": "s0",
                    "status": "observed",
                    "native_paths": ["pgorm::Connection::query_raw"],
                    "observation": observation,
                }
            ],
            "cleanup_errors": [],
        },
        "subject_state": {"tables": []},
        "reference_state": {"tables": []},
        "provenance": {
            "backend": "standalone-rust",
            "exit_code": 0,
            "source_sha256": "a" * 64,
            "lock_sha256": "b" * 64,
            "executable_sha256": "c" * 64,
        },
    }
    result["reference"] = copy.deepcopy(result["subject"])
    result["reference"]["steps"][0]["observation"]["rows"][0]["fields"][0]["value"][
        "data"
    ] = "1"
    result["comparisons"] = oracles.compare(
        program.data(),
        result["subject"],
        result["reference"],
        result["subject_state"],
        result["reference_state"],
    )
    return result


# [spec:pgorm:req:generative.attribution/test]
class AttributionTests(unittest.TestCase):
    def test_attribution_requires_comparable_completed_rust_evidence(self):
        program = control_programs.read()
        python = report(program)
        identity = {"source_sha256": "a" * 64}
        self.assertEqual(
            attribution.classify(program, python, identity)["classification"],
            "unattributed",
        )
        native = report(program)
        result = attribution.classify(program, python, identity, native)
        self.assertEqual(result["classification"], "native-reproduced")
        self.assertIn("reference-oracle", result["possible_causes"])
        self.assertFalse(result["library_change_authorized"])
        native = report(program, value="1", status="pass")
        self.assertEqual(
            attribution.classify(program, python, identity, native)["classification"],
            "binding-divergence",
        )
        for change in ("source", "program", "exit", "cleanup", "missing"):
            changed = copy.deepcopy(native)
            if change == "source":
                changed["provenance"]["source_sha256"] = "d" * 64
            if change == "program":
                changed["program_sha256"] = "d" * 64
            if change == "exit":
                changed["provenance"]["exit_code"] = 1
            if change == "cleanup":
                changed["cleanup_errors"] = ["failed"]
            if change == "missing":
                changed["subject"]["steps"] = []
            self.assertEqual(
                attribution.classify(program, python, identity, changed)[
                    "classification"
                ],
                "unattributed",
            )


# [spec:pgorm:req:generative.replay-parity/test]
class ParityTests(unittest.TestCase):
    def test_agreeing_rows_excuse_no_api_drift(self):
        program = control_programs.read()
        python = report(program)
        native = report(program)
        self.assertTrue(
            all(c["equal"] for c in attribution.parity(program, python, native))
        )
        drifted = copy.deepcopy(native)
        drifted["subject"]["steps"][0]["native_paths"] = ["pgorm::Selector"]
        checks = attribution.parity(program, python, drifted)
        self.assertFalse(all(check["equal"] for check in checks))
        self.assertIn("selected API paths differ", [c["reason"] for c in checks])

    def test_emitter_drift_makes_a_finding_unattributed(self):
        program = control_programs.read()
        python = report(program)
        drifted = report(program)
        drifted["subject"]["steps"][0]["native_paths"] = ["pgorm::pipeline::Pipeline"]
        result = attribution.classify(
            program, python, {"source_sha256": "a" * 64}, drifted
        )
        self.assertNotEqual(result["classification"], "native-reproduced")

    def test_a_step_without_recorded_paths_is_refused(self):
        program = control_programs.read()
        python = report(program)
        native = report(program)
        del native["subject"]["steps"][0]["native_paths"]
        with self.assertRaises(attribution.InvalidOracle):
            attribution.parity(program, python, native)


# [spec:pgorm:req:generative.attribution/test]
class RetentionTests(unittest.TestCase):
    def test_retention_never_overwrites_original_python_failure(self):
        program = control_programs.read()
        python = report(program)
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root) / "finding"
            attribution.retain(
                directory,
                program,
                python,
                {"source_sha256": "a" * 64},
                commands={"python": ["python", "replay.py"]},
            )
            original = (directory / "python.json").read_bytes()
            with self.assertRaises(FileExistsError):
                attribution.retain(
                    directory,
                    program,
                    report(program, status="pass"),
                    {},
                    commands={"python": ["python", "replay.py"]},
                )
            self.assertEqual((directory / "python.json").read_bytes(), original)
            self.assertTrue((directory / "fixture.json").exists())
            self.assertTrue((directory / "manifest.json").exists())
