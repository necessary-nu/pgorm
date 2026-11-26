import ast
import unittest
from itertools import pairwise

from pgorm_campaign import baseline, emit_python, wire
from pgorm_campaign.author import Author
from pgorm_campaign.grammar import FAMILIES, generate

HOSTILE = "O'Brien \\ \" 雪 -- $tag$ ; 💩"
STDLIB = {"asyncio", "json", "os", "struct", "sys", "datetime", "decimal", "uuid"}
PGORM = {"pgorm", "pgorm.models"}


def emitted(source):
    return ast.parse(source)


def modules(tree):
    names = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            names.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom):
            names.add(node.module)
    return names


def constant(tree, name):
    for node in tree.body:
        if isinstance(node, ast.Assign) and node.targets[0].id == name:
            return ast.literal_eval(node.value)
    raise AssertionError("emitted source has no " + name)


def texts(tree):
    return {
        node.value
        for node in ast.walk(tree)
        if isinstance(node, ast.Constant) and isinstance(node.value, str)
    }


def hostile_fixture():
    fixture = baseline.default()
    fixture["tables"].append(
        {
            "schema": "fixture",
            "name": HOSTILE,
            "columns": [
                baseline.column("id", "i32", primary=True),
                baseline.column(HOSTILE, "text"),
            ],
            "rows": [[1, HOSTILE]],
        }
    )
    return fixture


def projection(author, value, alias):
    expression = author.node("expr.value", {"value": value}, {"mode": "bound"})
    return author.node("expr.alias", {"value": expression}, {"name": alias})


def hostile_program():
    author = Author(seed=7, fixture=hostile_fixture())
    table = author.node("table", data={"schema": "fixture", "name": HOSTILE})
    column = author.node("expr.column", {"table": table}, {"name": HOSTILE})
    enum = {"kind": "enum", "schema": "fixture", "name": 'State" 雪'}
    columns = [
        column,
        projection(author, author.value("text", HOSTILE), HOSTILE),
        projection(author, author.value("f64", "fff8000000000001"), "nan"),
        projection(author, author.value(enum, "O'Brien 雪"), "state"),
    ]
    query = author.node("select", {"columns": columns})
    author.fetch(author.node("select.from", {"query": query, "table": table}))
    return author.finish()


def join_program():
    author = Author(seed=8)
    base = author.node("table", data={"schema": "fixture", "name": "accounts"})
    other = author.node("table", data={"schema": "fixture", "name": "notes"})
    column = author.node("expr.column", {"table": base}, {"name": "id"})
    query = author.node("select", {"columns": [column]})
    query = author.node("select.from", {"query": query, "table": base})
    query = author.node(
        "select.join", {"query": query, "table": other}, {"kind": "inner"}
    )
    author.fetch(query)
    return author.finish()


def nested_scopes_program():
    """Seven nested transaction scopes, the most a program may open, each
    closed in turn by a rollback or a commit."""
    author = Author(seed=10)
    table = author.node("table", data={"schema": "fixture", "name": "accounts"})
    column = author.node("expr.column", {"table": table}, {"name": "id"})
    query = author.node("select", {"columns": [column]})
    query = author.node("select.from", {"query": query, "table": table})
    scopes = ["root"] + ["s" + str(depth) for depth in range(1, 8)]
    for parent, child in pairwise(scopes):
        author.effect(
            "begin",
            data={"child": child, "mode": "default", "isolation": "default"},
            scope=parent,
        )
    author.effect(
        "fetch", {"query": query}, {"mode": "all", "ordered": False}, scope=scopes[-1]
    )
    for depth, scope in enumerate(reversed(scopes[1:])):
        author.effect("rollback" if depth % 2 == 0 else "commit", scope=scope)
    return author.finish()


def runtime_model_program():
    author = Author(seed=9)
    table = author.node("table", data={"schema": "fixture", "name": "absent"})
    model = author.node("model", {"table": table}, {})
    query = author.node("model.select", {"model": model, "order": []})
    author.fetch(query)
    return author.finish()


# [spec:pgorm:req:generative.replay/test]
class EmitPythonTests(unittest.TestCase):
    def test_every_family_emits_compilable_reproducer(self):
        for family in FAMILIES:
            for index in range(4):
                program = generate(20260911, index, family=family).program
                source = emit_python.render(program)
                with self.subTest(family=family, index=index):
                    compile(source, "python.py", "exec")
                    self.assertIn(program.digest, source)
                    self.assertNotIn("eval(", source)
                    self.assertNotIn("exec(", source)

    def test_emitted_source_imports_only_public_pgorm(self):
        for family in FAMILIES:
            program = generate(20260911, 2, family=family).program
            source = emit_python.render(program)
            with self.subTest(family=family):
                self.assertNotIn("pgorm_campaign", source)
                self.assertNotIn("sqlmap", source)
                self.assertLessEqual(modules(emitted(source)), STDLIB | PGORM)

    def test_hostile_names_and_values_round_trip(self):
        program = hostile_program()
        tree = emitted(emit_python.render(program))
        literals = texts(tree)
        self.assertIn(HOSTILE, literals)
        self.assertIn('State" 雪', literals)
        self.assertIn("_f64('fff8000000000001')", ast.unparse(tree))
        statements = constant(tree, "FIXTURE_STATEMENTS")
        fixture = program.data()["fixture"]
        self.assertEqual(statements, emit_python.statements(baseline.render(fixture)))
        self.assertTrue(any("$tag$" in statement for statement in statements))
        self.assertEqual(constant(tree, "PROGRAM_SHA256"), program.digest)

    def test_report_records_every_step_in_order(self):
        program = generate(20260911, 0, family="sequence").program
        source = emit_python.render(program)
        steps = [step["id"] for step in program.data()["steps"]]
        recorded = [
            node.values[0].value
            for node in ast.walk(emitted(source))
            if isinstance(node, ast.Dict)
            and [key.value for key in node.keys][:1] == ["id"]
        ]
        self.assertEqual(recorded, steps)
        self.assertEqual(source.count('report["steps"].append(_step)'), len(steps))
        self.assertIn("STEP_COUNT = " + str(len(steps)), source)

    def test_nested_scopes_emit_as_async_with_blocks(self):
        source = emit_python.render(nested_scopes_program())
        compile(source, "python.py", "exec")
        self.assertEqual(source.count("async with connection.transaction("), 1)
        self.assertEqual(source.count(".transaction() as tx_s"), 6)
        self.assertEqual(source.count("raise _RolledBack"), 4)
        for guard in (".begin(", ".commit(", ".rollback(", "open_transactions"):
            self.assertNotIn(guard, source)

    def test_unsupported_join_raises_named_instruction_error(self):
        with self.assertRaises(emit_python.UnsupportedInstruction) as caught:
            emit_python.render(join_program())
        self.assertIn("select.join", str(caught.exception))
        with self.assertRaises(emit_python.UnsupportedInstruction):
            emit_python.literal(object())

    def test_runtime_table_identity_uses_fixture_descriptor(self):
        source = emit_python.render(runtime_model_program())
        compile(source, "python.py", "exec")
        tree = emitted(source)
        self.assertIn("_descriptor(n_n0, None)", ast.unparse(tree))
        self.assertIn('(p.ModelQuery, "pgorm::Connection::query_raw")', source)
        self.assertIn("category in (p.ModelQuery, p.ModelRows)", source)
        tables = constant(tree, "FIXTURE_TABLES")
        self.assertIn("accounts", [table["name"] for table in tables])

    def test_ranges_rebuild_through_public_range_values(self):
        value = wire.scalar(
            "daterange", {"lower": "2026-01-01", "upper": None, "bounds": "[)"}
        )
        self.assertEqual(
            emit_python.value_source(value),
            "p.Value(p.Range(date.fromisoformat('2026-01-01'), None, '[)'), 'daterange')",
        )
        empty = wire.scalar("int4range", {"empty": True})
        self.assertEqual(
            emit_python.value_source(empty), "p.Value(p.Range.empty(), 'int4range')"
        )

    def test_intervals_rebuild_through_public_interval_values(self):
        value = wire.scalar("interval", wire.interval(1, -2, 3_000_004))
        self.assertEqual(
            emit_python.value_source(value),
            "p.Value(p.Interval(months=1, days=-2, microseconds=3000004), 'interval')",
        )
        self.assertEqual(
            emit_python.value_source(wire.scalar("interval", None, sql_null=True)),
            "p.Value.null('interval')",
        )

    def test_version_writes_reach_the_public_terminals(self):
        for index in range(60):
            program = generate(20260913, index, family="active").program
            if any(step["op"] == "entity.upsert" for step in program.data()["steps"]):
                source = emit_python.render(program)
                ast.parse(source)
                self.assertIn("await _versions('entity.upsert', ", source)
                self.assertIn(".returning_upserts(connection)", source)
                return
        self.fail("no generated active program upserts")

    def test_render_accepts_program_dict_and_text(self):
        program = generate(20260911, 3, family="select").program
        source = emit_python.render(program)
        self.assertEqual(emit_python.render(program.data()), source)
        self.assertEqual(emit_python.render(program.encoded), source)


if __name__ == "__main__":
    unittest.main()
