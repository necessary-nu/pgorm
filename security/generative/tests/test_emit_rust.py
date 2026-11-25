import re
import unittest
from pathlib import Path

from pgorm_campaign import baseline, emit_rust, emit_rust_expr
from pgorm_campaign.author import Author
from pgorm_campaign.grammar import FAMILIES, generate

HOSTILE = "O'Brien \\ \" 雪 -- $tag$ ; 💩"
# Families whose terminals decode into compiled models rather than result rows.
TYPED = ("entity", "active", "graph", "cursor", "model", "sources")
ESCAPES = {'"': '"', "\\": "\\"}
LITERAL = re.compile(r'"(?:[^"\\]|\\(?:["\\]|u\{[0-9a-f]+\}))*"')
PIECE = re.compile(r'\\u\{([0-9a-f]+)\}|\\(["\\])|([^\\]+)')


def unescape(literal):
    """Read an emitted Rust string literal back, so escaping can be checked."""
    if not (literal.startswith('"') and literal.endswith('"')):
        raise AssertionError("not a Rust string literal: " + literal)
    parts = []
    for point, escape, plain in PIECE.findall(literal[1:-1]):
        if point:
            parts.append(chr(int(point, 16)))
        elif escape:
            parts.append(ESCAPES[escape])
        else:
            parts.append(plain)
    return "".join(parts)


def literals(source):
    return {unescape(match.group(0)) for match in LITERAL.finditer(source)}


ROOT = Path(__file__).resolve().parents[3]


def supported(family, limit=8):
    """The first program of a family standalone Rust can express."""
    for index in range(limit):
        program = generate(20260911, index, family=family).program
        try:
            emit_rust.render(program)
        except emit_rust.UnsupportedInstruction:
            continue
        return program
    return None


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
        projection(author, author.value("f32", "7fc00001"), "nan32"),
        projection(author, author.value(enum, "O'Brien 雪"), "state"),
    ]
    query = author.node("select", {"columns": columns})
    author.fetch(author.node("select.from", {"query": query, "table": table}))
    return author.finish()


def graph_program(name="campaign.AccountOnly", aliases=()):
    author = Author(seed=11)
    registration = author.node("graph", data={"name": name})
    query = author.node(
        "graph.find", {"graph": registration}, {"aliases": list(aliases)}
    )
    author.fetch(query)
    return author.finish()


def shared_cursor_program():
    """One cursor feeding two pages, which the Rust type cannot be asked for."""
    author = Author(seed=13)
    registration = author.node("graph", data={"name": "campaign.OptionalNotes"})
    query = author.node("graph.find", {"graph": registration}, {"aliases": ["slot"]})
    cursor = author.node("graph.cursor", {"query": query}, {"column": "id"})
    for count in (1, 2):
        page = author.node(
            "cursor.page",
            {"cursor": cursor},
            {"side": "first", "count": count, "direction": "asc"},
        )
        author.fetch(page)
    return author.finish()


def streaming_program():
    """The first select-family program whose terminal is a stopped stream."""
    for index in range(24):
        program = generate(20260911, index, family="select").program
        if any(step["op"] == "stream" for step in program.data()["steps"]):
            return program
    raise AssertionError("no generated select program streams its rows")


# [spec:pgorm:req:generative.replay/test]
class EmitRustTests(unittest.TestCase):
    def test_every_family_renders_a_whole_reproducer(self):
        for family in FAMILIES:
            with self.subTest(family=family):
                program = supported(family)
                self.assertIsNotNone(program)
                source = emit_rust.render(program)
                self.assertIn(program.digest, source)
                self.assertIn("#[tokio::main]", source)
                self.assertIn("Report::new(PROGRAM_SHA256)", source)
                self.assertIn("report.emit()", source)

    def test_every_generated_program_renders_in_every_family(self):
        for family in FAMILIES:
            for index in range(6):
                program = generate(20260911, index, family=family).program
                with self.subTest(family=family, index=index):
                    self.assertIn(program.digest, emit_rust.render(program))

    def test_compiled_model_rows_carry_their_entity(self):
        for family in ("entity", "active", "graph", "cursor"):
            source = emit_rust.render(supported(family))
            with self.subTest(family=family):
                self.assertIn("::observe::model::<", source)
                found = literals(source)
                registered = {
                    "campaign.Account",
                    "campaign.Note",
                    "campaign.Room",
                    "campaign.Stay",
                }
                self.assertTrue(registered & found)
        # A compiled column is named at runtime and resolved against the entity.
        for family in ("entity", "active", "cursor"):
            with self.subTest(family=family, resolved=True):
                self.assertIn(
                    "::entities::column::<", emit_rust.render(supported(family))
                )

    def test_graph_rows_observe_every_declared_slot(self):
        source = emit_rust.render(graph_program("campaign.Arity3", ("one", "two")))
        self.assertIn("::observe::tuple(vec![", source)
        self.assertEqual(source.count("::observe::maybe::<"), 2)
        self.assertIn("entities::graphs::arity3", source)
        # The root-only shape reports the model itself, not a one-slot tuple.
        bare = emit_rust.render(graph_program())
        self.assertNotIn("::observe::tuple(", bare)
        self.assertIn("entities::graphs::account_only", bare)

    def test_source_tuples_stay_tuples_at_every_arity(self):
        source = emit_rust.render(supported("sources"))
        self.assertIn("::pipeline::Pipeline::select_sources(", source)
        self.assertIn("::pipeline::named_runtime(", source)
        self.assertIn("::observe::tuple(vec![", source)

    def test_runtime_models_project_their_declared_fields(self):
        program = supported("model")
        source = emit_rust.render(program)
        self.assertIn("query.expr_as(", source)
        # A runtime model is not a registered entity, so its rows stay rows.
        self.assertNotIn("::observe::model::<", source)
        self.assertIn("::observe::rows(", source)

    def test_streamed_steps_drain_through_the_harness(self):
        source = emit_rust.render(streaming_program())
        self.assertIn("::stream::drain(opened,", source)
        self.assertIn("::observe::stream(", source)
        self.assertIn("Connection::query_raw(", source)

    def test_result_reads_follow_their_producing_step(self):
        source = emit_rust.render(supported("active"))
        body = source.splitlines()
        first = next(i for i, line in enumerate(body) if "let r_s" in line)
        reads = [i for i, line in enumerate(body) if "decoded(&r_s" in line]
        self.assertTrue(reads)
        self.assertLess(first, min(reads))

    def test_checked_kinds_read_through_replay_codecs(self):
        # A bare numeric/JSON/inet/macaddr read would take the driver's lossy
        # path; an enum label is only taken once its column type is the one
        # the program declared.
        reads = {
            "decimal": "codecs::ExactDecimal>(",
            "json": "codecs::ExactJson>(",
            "ipnetwork": "codecs::Inet>(",
            "mac_address": "codecs::Mac>(",
            "enum": "codecs::declared_enum(&r_",
        }
        seen = set()
        for index in range(200):
            program = generate(20260913, index, family="types").program
            kinds = {
                node["data"]["type"]["kind"]
                for node in program.data()["nodes"]
                if node["op"] == "result.value"
            }
            for kind in kinds & reads.keys():
                source = emit_rust.render(program)
                self.assertIn(reads[kind], source)
                if kind == "enum":
                    self.assertIn(emit_rust.literal('State" 雪'), source)
                seen.add(kind)
        self.assertEqual(seen, set(reads))

    def test_emitted_source_reaches_only_public_crates(self):
        for family in FAMILIES:
            program = supported(family)
            if program is None:
                continue
            source = emit_rust.render(program)
            with self.subTest(family=family):
                self.assertNotIn("pgorm_campaign", source)
                self.assertNotIn("sqlmap", source)
                self.assertNotIn("postgres://", source)
                self.assertNotIn("postgresql://", source)
                self.assertIn("PGORM_REPLAY_URL", source)
                for line in source.splitlines():
                    if line.startswith("use "):
                        self.assertEqual(
                            line,
                            "use pgorm_generative_replay::{Error, Harness, Report};",
                        )

    def test_captured_sql_is_never_substituted_for_builders(self):
        program = supported("select")
        source = emit_rust.render(program)
        recorded = {
            item["value"]["sql"]
            for item in program.data()["observations"]
            if item.get("value", {}).get("kind") == "compiled"
        }
        for sql in recorded:
            self.assertNotIn(emit_rust.literal(sql), source)
        self.assertIn("pgorm::pgorm_query::Query::select()", source)

    def test_hostile_names_and_payloads_round_trip(self):
        source = emit_rust.render(hostile_program())
        found = literals(source)
        self.assertIn(HOSTILE, found)
        self.assertIn('State" 雪', found)
        fixture = baseline.render(hostile_program().data()["fixture"])
        self.assertIn(fixture, found)
        self.assertTrue(any("$tag$" in text for text in found))
        self.assertEqual(source, source.encode("ascii", "strict").decode())

    def test_every_escaped_literal_reads_back_unchanged(self):
        for text in (HOSTILE, 'a"b', "a\\b", "\x00", "\x1f\x7f", "雪💩", "$tag$"):
            with self.subTest(text=text):
                self.assertEqual(unescape(emit_rust.literal(text)), text)
        with self.assertRaises(emit_rust.UnsupportedInstruction):
            emit_rust.literal(b"bytes")

    def test_floats_emit_as_exact_ieee_bit_patterns(self):
        source = emit_rust.render(hostile_program())
        self.assertIn("f64::from_bits(0xfff8000000000001u64)", source)
        self.assertIn("f32::from_bits(0x7fc00001u32)", source)
        self.assertNotIn("f64::NAN", source)
        for line in source.splitlines():
            if "Value::Double" in line or "Value::Float" in line:
                self.assertIn("from_bits", line)

    def test_temporal_programs_build_ranges_and_period_graphs(self):
        found = {}
        for family in ("active", "graph"):
            for index in range(60):
                program = generate(20260913, index, family=family).program
                names = {
                    node["data"].get("name")
                    for node in program.data()["nodes"]
                    if node["op"] in ("entity", "graph")
                }
                for name in (
                    "campaign.Room",
                    "campaign.StayRooms",
                    "campaign.StayGuests",
                ):
                    if name in names:
                        found.setdefault(name, emit_rust.render(program))
        room = found["campaign.Room"]
        self.assertIn(
            "pgorm::pgorm_query::Value::Range(pgorm::pgorm_query::RangeType::Date, Some(",
            room,
        )
        self.assertIn(
            "std::ops::Bound::Included(pgorm::pgorm_query::Value::Date(", room
        )
        self.assertIn(
            "std::ops::Bound::Excluded(pgorm::pgorm_query::Value::Date(", room
        )
        self.assertIn("::entities::room::Entity", room)
        self.assertIn("::entities::graphs::stay_rooms", found["campaign.StayRooms"])
        self.assertIn("::observe::model::<", found["campaign.StayRooms"])
        self.assertIn("campaign.Stay", literals(found["campaign.StayRooms"]))
        self.assertIn("::entities::graphs::stay_guests", found["campaign.StayGuests"])

    def test_version_writes_call_their_rust_terminals(self):
        found = {}
        for index in range(60):
            program = generate(20260913, index, family="active").program
            for step in program.data()["steps"]:
                if step["op"].startswith("entity."):
                    found.setdefault(step["op"], emit_rust.render(program))
        self.assertIn(".exec_returning_change(&", found["entity.change"])
        self.assertIn("::observe::tuple(vec![", found["entity.change"])
        changes = found["entity.changes"]
        self.assertIn("pgorm::Update::many(", changes)
        self.assertIn("pgorm::Column::save_as(", changes)
        self.assertIn(".exec_returning_changes(&", changes)
        upsert = found["entity.upsert"]
        self.assertIn(".on_conflict(", upsert)
        self.assertIn("pgorm::Upserted::Inserted(model)", upsert)
        self.assertIn("::observe::absent()", upsert)

    def test_intervals_emit_and_read_back_exactly(self):
        program = next(
            program
            for program in (
                generate(20260913, index, family="types").program
                for index in range(200)
            )
            if any(
                node["op"] == "result.value"
                and node["data"]["type"]["kind"] == "interval"
                for node in program.data()["nodes"]
            )
        )
        source = emit_rust.render(program)
        self.assertIn("pgorm::pgorm_query::Value::Interval(Some(Box::new(", source)
        self.assertIn("pgorm::pgorm_query::Interval::new(", source)
        self.assertIn("try_get::<_, pgorm::entity::prelude::Interval>(", source)
        self.assertIn("pgorm::pgorm_query::ArrayType::Interval", source)
        author = Author(seed=17)
        columns = [
            projection(author, author.value("interval", data), "i" + str(index))
            for index, data in enumerate(
                (
                    {"months": "1", "days": "-2", "microseconds": "3000004"},
                    {
                        "months": "-2147483648",
                        "days": "-2147483648",
                        "microseconds": "-9223372036854775808",
                    },
                )
            )
        ]
        author.fetch(author.node("select", {"columns": columns}))
        source = emit_rust.render(author.finish())
        self.assertIn("Interval::new(1i32, -2i32, 3000004i64)", source)
        self.assertIn(
            "Interval::new(-2147483648i32, -2147483648i32, -9223372036854775808i64)",
            source,
        )

    def test_enum_values_carry_their_qualified_cast(self):
        source = emit_rust.render(hostile_program())
        name = emit_rust.literal('State" 雪')
        self.assertIn(
            f"cast_as_type(pgorm::pgorm_query::TypeName::new("
            f"pgorm::pgorm_query::Name::runtime({name}))",
            source,
        )
        self.assertIn('.schema(pgorm::pgorm_query::Name::runtime("fixture"))', source)

    def test_unsupported_instructions_raise_naming_the_reason(self):
        # `Cursor<S, K>` derives Clone, so the bound lands on `GraphRow`, which
        # carries no data and does not implement it. One cursor, one chain.
        with self.assertRaises(emit_rust.UnsupportedInstruction) as caught:
            emit_rust.render(shared_cursor_program())
        self.assertIn("Clone", str(caught.exception))
        with self.assertRaises(emit_rust.UnsupportedInstruction):
            emit_rust.literal(b"bytes")

    def test_every_step_is_recorded_in_program_order(self):
        program = supported("sequence")
        source = emit_rust.render(program)
        steps = program.data()["steps"]
        recorded = re.findall(
            r'report\.(?:observed|failed)\("(s\d+)", "([a-z.]+)"', source
        )
        seen = []
        for identity, operation in recorded:
            if seen and seen[-1] == (identity, operation):
                continue
            seen.append((identity, operation))
        self.assertEqual(seen, [(step["id"], step["op"]) for step in steps])

    def test_schema_types_name_real_column_variants(self):
        # Every entry is emitted as `ColumnType::<name>`, so a name the enum
        # does not carry compiles to nothing. The suite generates schema
        # programs rarely enough that a wrong entry can sit latent for a long
        # time, which is exactly why this is checked against the enum itself.
        source = (ROOT / "pgorm-query/src/table/column.rs").read_text()
        body = source.split("pub enum ColumnType {", 1)[1].split("\n}", 1)[0]
        variants = set(re.findall(r"^ {4}([A-Z]\w*)", body, re.MULTILINE))
        self.assertIn("Timestamp", variants)
        for kind, named in emit_rust_expr.SCHEMA_TYPES.items():
            with self.subTest(kind=kind):
                self.assertIn(named.split("(")[0], variants)

    def test_render_accepts_program_dict_and_text(self):
        program = supported("types")
        source = emit_rust.render(program)
        self.assertEqual(emit_rust.render(program.data()), source)
        self.assertEqual(emit_rust.render(program.encoded), source)


if __name__ == "__main__":
    unittest.main()
