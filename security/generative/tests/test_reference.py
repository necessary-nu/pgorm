import copy
import struct
import unittest
from decimal import Decimal
from unittest.mock import Mock

from execution_cases import select_case

from pgorm_campaign import comparison, wire
from pgorm_campaign.oracles import compare
from pgorm_campaign.reference import Resolution
from pgorm_campaign.reference_codec import Codec
from pgorm_campaign.reference_inspection import read_statement
from pgorm_campaign.reference_literal import literal
from pgorm_campaign.reference_sql import SQL, bound
from pgorm_campaign.reference_template import Raw, template
from pgorm_campaign.reference_values import Float32, qualified


class ReferenceTests(unittest.TestCase):
    def test_inspection_accepts_owned_select_templates(self):
        raw = Raw("SELECT $1::int4, '$2' /* $3 */", [wire.scalar("i32", "7")])
        text, values = read_statement(raw, "raw.template").command()
        self.assertEqual(values, ["7"])
        self.assertIn("'$2' /* $3 */", text)
        for text in ("DELETE FROM fixture.accounts", "SELECTED 1"):
            with self.assertRaises(comparison.InvalidOracle):
                read_statement(Raw(text, []), "raw.template")

    def test_decimal_decoding_preserves_fixed_scale(self):
        connection = Mock()
        codec = Codec(connection)
        codec.types = {1700: ("pg_catalog", "numeric", "b", 0, "N")}
        loader = connection.adapters.get_loader.return_value.return_value
        for value in ("0.0000000000000000000000000001", "-0.00", "100.0000"):
            loader.load.return_value = Decimal(value)
            self.assertEqual(codec.value(1700, b"decoded-by-driver")["data"], value)

    def test_reference_slots_respect_postgresql_quoted_contexts(self):
        source = "SELECT $2, $1, $2, '$3', E'\\'$4', \"$5\", $$ $6 $$, $tag$ $7 $tag$ /* outer $8 /* inner $9 */ */ -- $10\n"
        text, arguments = template(
            source, [wire.scalar("text", "O'Brien"), wire.scalar("i32", "2")]
        ).command()
        self.assertEqual(arguments, ["2", "O'Brien", "2"])
        self.assertIn("'$3'", text)
        self.assertIn("$tag$ $7 $tag$", text)
        self.assertIn("/* inner $9 */", text)
        self.assertTrue(text.endswith("-- $10\n"))
        with self.assertRaises(comparison.InvalidOracle):
            template("SELECT '$1'", [wire.scalar("text", "unused")])

    # [spec:pgorm:req:generative.oracles/test]
    def test_query_uses_independent_parameters_and_quotes_names(self):
        program = select_case().data()
        query = Resolution(program).get(program["steps"][0]["inputs"]["query"])
        sql, parameters = query.sql().command()
        self.assertIn('"accounts"."tenant" = %s::integer', sql)
        self.assertEqual(parameters, ["1"])
        hostile = 'n%"; SELECT false; -- 雪'
        command = SQL(
            ("SELECT ", qualified(hostile, "fixture"), " WHERE x = ")
        ) + bound(wire.scalar("text", hostile))
        text, values = command.command()
        self.assertIn('"n%%""; SELECT false; -- 雪"', text)
        self.assertEqual(values, [hostile])
        self.assertNotIn("pgorm", Resolution.__module__.split("."))

    # [spec:pgorm:req:generative.comparison+1/test]
    def test_binary_decode_preserves_bits_nulls_and_types(self):
        codec = Codec(None)
        codec.types = {
            23: ("pg_catalog", "int4", "b", 0, "N"),
            700: ("pg_catalog", "float4", "b", 0, "N"),
            3802: ("pg_catalog", "jsonb", "b", 0, "U"),
            9000: ("fixture", 'State" 雪', "e", 0, "E"),
            9001: ("fixture", '_State" 雪', "b", 9000, "A"),
        }
        self.assertEqual(
            codec.value(700, bytes.fromhex("80000000"))["data"], "80000000"
        )
        self.assertEqual(
            codec.value(700, bytes.fromhex("7fc00001"))["data"], "7fc00001"
        )
        self.assertEqual(
            bound(wire.scalar("f32", "7fc00001")).command()[1],
            [Float32(bytes.fromhex("7fc00001"))],
        )
        self.assertFalse(codec.value(3802, b"\x01null")["sql_null"])
        self.assertTrue(codec.value(3802, None)["sql_null"])
        label = "O'Brien 雪".encode()
        raw = (
            struct.pack("!iiIii", 1, 1, 9000, 2, 1)
            + struct.pack("!i", len(label))
            + label
            + struct.pack("!i", -1)
        )
        value = codec.value(9001, raw)
        self.assertEqual(
            value["type"]["element"],
            {"kind": "enum", "name": 'State" 雪', "schema": "fixture"},
        )
        self.assertEqual(value["data"][0]["data"], "O'Brien 雪")
        self.assertTrue(value["data"][1]["sql_null"])
        with self.assertRaises(comparison.InvalidOracle):
            codec.value(9001, raw + b"x")

    def test_empty_insert_refused_like_the_binding(self):
        from pgorm_campaign.grammar import generate
        from pgorm_campaign.reference_sql import Rejection

        for index in range(60):
            generated = generate(11, index, mode="invalid")
            if generated.recipe()["rejection_case"] == "empty-batch":
                break
        program = generated.program.data()
        step = program["steps"][0]
        with self.assertRaises(Rejection) as raised:
            Resolution(program).get(step["inputs"]["query"]).sql()
        declared = program["observations"][0]["error"]
        self.assertEqual(raised.exception.observation["class"], declared["class"])
        self.assertEqual(raised.exception.observation["cause"], declared["cause"])

    def test_unsigned_overflow_is_refused_by_position(self):
        from pgorm_campaign.reference_sql import Rejection

        past = wire.scalar("u64", "9223372036854775808")
        fits = wire.scalar("u64", "9223372036854775807")
        text, values = bound(fits).command()
        self.assertEqual((text, values), ("%s::bigint", ["9223372036854775807"]))
        array = {"kind": "array", "element": {"kind": "u64"}}
        null = wire.scalar("u64", None, sql_null=True)
        refusal = "value `9223372036854775808` is out of range for Postgres type `int8`"
        for statement, position in (
            (bound(past), 1),
            (bound(fits) + ", " + bound(wire.scalar(array, [null, past])), 2),
            (literal(fits) + ", " + bound(past), 1),
        ):
            with self.assertRaises(Rejection) as raised:
                statement.command()
            self.assertEqual(
                raised.exception.observation["cause"],
                f"parameter ${position} cannot be bound: {refusal}",
            )
        # Inlined digits are the server's to refuse, never the binding's.
        self.assertEqual(literal(past).command()[1], ["9223372036854775808"])

    # [spec:pgorm:req:generative.comparison+1/test]
    def test_intervals_decode_and_bind_field_by_field(self):
        from pgorm_campaign.reference_values import interval_text

        codec = Codec(None)
        codec.types = {1186: ("pg_catalog", "interval", "b", 0, "T")}
        raw = struct.pack("!qii", 3_000_004, -2, 1)
        self.assertEqual(
            codec.value(1186, raw),
            wire.scalar("interval", wire.interval(1, -2, 3_000_004)),
        )
        infinity = struct.pack("!qii", 2**63 - 1, 2**31 - 1, 2**31 - 1)
        self.assertEqual(codec.value(1186, infinity)["data"], wire.INTERVAL_INFINITY)
        with self.assertRaises(comparison.InvalidOracle):
            codec.value(1186, raw[:12])
        self.assertEqual(
            interval_text(wire.interval(1, -2, 3_000_004)),
            "+1 mon -2 day +3000004 us",
        )
        self.assertEqual(interval_text(wire.interval(0, 0, 0)), "+0 mon +0 day +0 us")
        self.assertEqual(interval_text(wire.INTERVAL_INFINITY), "infinity")
        self.assertEqual(interval_text(wire.INTERVAL_NEGATIVE_INFINITY), "-infinity")
        value = wire.scalar("interval", wire.interval(0, 30, 0))
        self.assertEqual(
            bound(value).command(), ("%s::interval", ["+0 mon +30 day +0 us"])
        )
        # The subject's literal carries its cast, so the reference's does too.
        self.assertEqual(literal(value).command(), bound(value).command())
        # Identity is field by field: a month is not thirty days here, though
        # PostgreSQL's `=` says it is.
        month = wire.scalar("interval", wire.interval(1, 0, 0))
        self.assertNotEqual(comparison.value_key(month), comparison.value_key(value))

    def test_vectors_spell_their_exact_elements(self):
        from pgorm_campaign.reference_values import vector_text

        value = wire.scalar("vector", ["00000000", "80000000", "3dcccccd"])
        self.assertEqual(vector_text(value), "[0,-0,0.1]")
        self.assertEqual(bound(value).command(), ("%s::vector", ["[0,-0,0.1]"]))
        with self.assertRaises(comparison.InvalidOracle):
            vector_text(wire.scalar("vector", ["7fc00000"]))

    def test_unsupported_semantics_cannot_become_a_pass(self):
        with self.assertRaises(comparison.InvalidOracle):
            literal(wire.scalar("f64", "7ff0000000000000"))
        program = select_case().data()
        report = {
            "status": "executed",
            "steps": [
                {
                    "id": "s0",
                    "status": "observed",
                    "native_paths": ["pgorm::ConnectionTrait::query_raw"],
                    "observation": {"kind": "rows", "rows": []},
                }
            ],
        }
        state = {"tables": []}
        self.assertTrue(
            all(
                item["equal"] for item in compare(program, report, report, state, state)
            )
        )
        with self.assertRaises(comparison.InvalidOracle):
            compare(program, report, report, None, state)
        missing = copy.deepcopy(report)
        missing["steps"] = []
        with self.assertRaises(comparison.InvalidOracle):
            compare(program, report, missing, state, state)
        inactive = copy.deepcopy(report)
        inactive["steps"][0]["native_paths"] = []
        with self.assertRaises(comparison.InvalidOracle):
            compare(program, inactive, report, state, state)
        changes = compare(
            program, report, report, {"tables": [{"name": "changed-sentinel"}]}, state
        )
        self.assertFalse(changes[-1]["equal"])
        program["observations"][0]["oracle"] = "native-parity"
        with self.assertRaises(comparison.InvalidOracle):
            compare(program, report, report, state, state)
