import copy
import unittest

from pgorm_campaign import wire
from pgorm_campaign.catalog import EFFECTS, OPERATIONS
from pgorm_campaign.grammar import FAMILIES, generate
from pgorm_campaign.grammar_pipeline import Column, Pipeline
from pgorm_campaign.grammar_rejection import RULES
from pgorm_campaign.grammar_state import Limits, State
from pgorm_campaign.parameters import input_ids
from pgorm_campaign.program import Program
from pgorm_campaign.refusals import EMPTY_INSERT


def _reachable(nodes, step):
    """Every node a step's inputs reach, through the nodes' own inputs."""
    pending, reached = list(input_ids(step["inputs"])), []
    while pending:
        node = nodes[pending.pop()]
        reached.append(node)
        pending.extend(input_ids(node["inputs"]))
    return reached


def _written_from(data, step):
    """The step a written model's chain of reads and writes began with, and
    the entities that step names."""
    nodes = {node["id"]: node for node in data["nodes"]}
    steps = {item["id"]: item for item in data["steps"]}
    while step["op"] == "active.write" and step["data"]["method"] != "insert":
        result = next(
            node for node in _reachable(nodes, step) if node["op"] == "entity.result"
        )
        step = steps[result["data"]["step"]]
    reached = _reachable(nodes, step)
    return step, {node["data"]["name"] for node in reached if node["op"] == "entity"}


# [spec:pgorm:req:generative.grammar/test]
class GrammarTests(unittest.TestCase):
    def test_all_families_cover_catalog_within_budgets(self):
        operations, effects, writes, conflicts, modes = (
            set(),
            set(),
            set(),
            set(),
            set(),
        )
        for family in FAMILIES:
            for index in range(150):
                generated = generate(
                    20260911,
                    index,
                    family=family,
                    limits=Limits(depth=3, stages=5, nodes=128),
                )
                program = generated.program.data()
                self.assertLessEqual(len(program["nodes"]), 128)
                self.assertEqual(Program.from_dict(program), generated.program)
                operations.update(node["op"] for node in program["nodes"])
                effects.update(step["op"] for step in program["steps"])
                writes.update(
                    node["data"]["method"]
                    for node in program["nodes"]
                    if node["op"] == "model.write"
                )
                conflicts.update(
                    node["data"]["action"]
                    for node in program["nodes"]
                    if node["op"] == "insert.conflict"
                )
                modes.update(
                    step["data"]["mode"]
                    for step in program["steps"]
                    if step["op"] == "begin"
                )
        self.assertEqual(operations, set(OPERATIONS))
        self.assertEqual(effects, set(EFFECTS))
        self.assertEqual(writes, {"insert", "update", "delete"})
        self.assertEqual(conflicts, {"nothing", "update"})
        self.assertIn("read_only", modes)

    def test_window_functions_receive_their_public_arguments(self):
        seen = set()
        for index in range(100):
            program = generate(20260911, index, family="relational").program.data()
            if not any(n["op"] == "pipeline.window" for n in program["nodes"]):
                continue
            for node in program["nodes"]:
                if node["op"] == "pipeline.function":
                    name = node["data"]["name"]
                    seen.add(name)
                    self.assertEqual(
                        len(node["inputs"]["arguments"]),
                        0 if name == "row_number" else 1,
                    )
        self.assertTrue({"rank", "rank_dense", "row_number"} <= seen)

    def test_group_keys_leave_the_aggregate_input_scope(self):
        for index in range(150):
            program = generate(20260911, index, family="relational").program.data()
            nodes = {node["id"]: node for node in program["nodes"]}
            keys = {
                nodes[ref]["data"]["name"]
                for node in program["nodes"]
                if node["op"] == "pipeline.group"
                for ref in node["inputs"]["keys"]
            }
            if keys:
                for node in program["nodes"]:
                    if node["op"] == "pipeline.function":
                        for ref in node["inputs"]["arguments"]:
                            self.assertNotIn(nodes[ref]["data"]["name"], keys)

    def test_schema_renames_preserve_earlier_insert_columns(self):
        program = generate(20260911, 0, family="schema").program.data()
        create = next(
            node for node in program["nodes"] if node["op"] == "schema.create"
        )
        insert = next(node for node in program["nodes"] if node["op"] == "insert")
        self.assertEqual(
            insert["data"]["columns"],
            [column["name"] for column in create["data"]["columns"]],
        )

    def test_generated_programs_vary_structure_and_real_inputs(self):
        for family in ("select", "sequence", "pipeline"):
            outputs = [generate(101, index, family=family) for index in range(300)]
            self.assertEqual(len({item.program.digest for item in outputs}), 300)
            self.assertGreater(
                len({item.recipe()["structure_sha256"] for item in outputs}), 100
            )
            self.assertEqual(outputs[11], generate(101, 11, family=family))
            for output in outputs:
                self.assertEqual(
                    Program.from_dict(output.program.data()), output.program
                )
                self.assertFalse(output.recipe()["expected_rejections"])
                self.assertLessEqual(len(output.program.data()["nodes"]), 256)

    def test_valid_pipelines_never_draw_refused_identifiers(self):
        # pipeline.errors+5: a `"` in a pipeline identifier is refused at
        # into_sql, so a valid program carrying one could never pass.
        for family in ("pipeline", "relational", "sets", "sources"):
            for index in range(120):
                program = generate(20260913, index, family=family).program.data()
                for node in program["nodes"]:
                    if not node["op"].startswith("pipeline."):
                        continue
                    for value in node["data"].values():
                        names = value if isinstance(value, list) else [value]
                        for name in names:
                            if isinstance(name, str):
                                self.assertNotIn('"', name, (family, index))

    def test_budget_changes_still_produce_valid_programs(self):
        for index in range(300):
            output = generate(
                101,
                index,
                family="pipeline",
                limits=Limits(depth=3, stages=5, nodes=128),
            )
            self.assertLessEqual(len(output.program.data()["nodes"]), 128)
        for limits in ({"depth": 4}, {"stages": 0}, {"nodes": 400}, {"depth": True}):
            with self.assertRaises(ValueError):
                Limits(**limits)

    def test_rejection_profiles_require_exact_declared_errors(self):
        causes = {
            "division": "sqlstate:22012",
            "not-null": "sqlstate:23502",
            "duplicate": "sqlstate:23505",
            "empty-batch": EMPTY_INSERT,
            "missing-vector-type": "sqlstate:42704",
        }
        cases = set()
        for index in range(120):
            result = generate(17, index, mode="invalid")
            recipe = result.recipe()
            case = recipe["rejection_case"]
            cases.add(case)
            self.assertEqual(recipe["mode"], "invalid")
            self.assertEqual(len(recipe["expected_rejections"]), 1)
            error = recipe["expected_rejections"][0]["error"]
            if case == "unquotable-identifier":
                self.assertEqual(error["class"], "ConstructionError")
                self.assertRegex(error["cause"], r'^identifier `origin[^`]*"[^`]*` ')
            elif case == "unsigned-overflow" and error["class"] == "ConstructionError":
                self.assertRegex(
                    error["cause"],
                    r"^parameter \$1 cannot be bound: "
                    r"value `\d+` is out of range for Postgres type `int8`$",
                )
            elif case == "unsigned-overflow":
                self.assertEqual(
                    error, {"class": "DatabaseError", "cause": "sqlstate:22003"}
                )
            else:
                self.assertEqual(error["cause"], causes[case])
        self.assertEqual(cases, set(RULES))
        for options in (
            {"mode": "unknown"},
            {"mode": "invalid", "family": "select"},
            {"family": "rejection"},
        ):
            with self.assertRaises(ValueError):
                generate(1, 0, **options)

    def test_sources_types_and_binder_ownership_are_checked(self):
        state = State(1, 0)
        source = state.source()
        self.assertTrue(state.column(source, "score").nullable)
        self.assertFalse(state.column(source, "id").nullable)
        with self.assertRaises(ValueError):
            state.binary(state.constant("i32", 1), state.constant("text", "1"), "eq")
        pipe = Pipeline(State(1, 0))
        with self.assertRaisesRegex(ValueError, "unavailable source"):
            pipe.expression(Column("missing", "i32"))
        result = next(
            generate(18, index, family="pipeline")
            for index in range(100)
            if generate(18, index, family="pipeline").program.data()["binders"]
        )
        data = copy.deepcopy(result.program.data())
        owner = data["binders"][0]["owner"]
        next(node for node in data["nodes"] if node["id"] == owner)["data"].pop(
            "binder"
        )
        with self.assertRaises(wire.FormatError):
            Program.from_dict(data)

    def test_temporal_deletes_only_remove_written_versions(self):
        # Every room version in the fixture is referenced by a stay, so
        # deleting one is refused when the deferred temporal key is checked.
        deletes = 0
        for index in range(400):
            data = generate(20260913, index, family="active").program.data()
            for step in data["steps"]:
                if step["op"] != "active.write" or step["data"]["method"] != "delete":
                    continue
                origin, entities = _written_from(data, step)
                if "campaign.Room" in entities:
                    deletes += 1
                    self.assertEqual(origin["data"].get("method"), "insert", index)
        self.assertGreater(deletes, 0)

    def test_reused_results_require_the_original_producer(self):
        result = generate(22, 4, family="sequence")
        data = result.program.data()
        results = [node for node in data["nodes"] if node["op"] == "result.value"]
        self.assertTrue(results)
        producer = results[0]["data"]["step"]
        consumer = next(step for step in data["steps"] if step["id"] == producer)
        consumer["op"], consumer["data"] = "execute", {}
        with self.assertRaises(wire.FormatError):
            Program.from_dict(data)
