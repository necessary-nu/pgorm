"""Lossless conversion through the installed native module, without a database."""

import datetime as dt
from decimal import Decimal
import json
import math
import struct
import unittest
from uuid import UUID
from zoneinfo import ZoneInfo

from pgorm import ConstructionError, Interval, TypeName, Value, capabilities


def bits(value):
    return struct.pack(">d", value).hex()


# [spec:pgorm:req:bindings.values/test]
# [spec:pgorm:req:python.values+7/test]
# [spec:pgorm:req:bindings.value-tags+1/test]
class ValueTests(unittest.TestCase):
    def test_scalar_variants_and_typed_nulls(self):
        instant = dt.datetime(2024, 2, 29, 13, 14, 15, 123456)
        cases = {
            "bool": True, "i8": -128, "i16": -32768, "i32": -(2**31),
            "i64": -(2**63), "u32": 2**32 - 1, "u64": 2**64 - 1,
            "f32": 1.5, "f64": 0.1, "text": "'雪%\\\"\0", "char": "雪",
            "bytes": b"\0\xff'\\", "json": {"null": None, "bool": True},
            "decimal": Decimal("123456789.00000000000000000001"),
            "uuid": UUID("f71dc341-54bc-4e7b-ae7b-1a60aa3b37b4"),
            "date": instant.date(), "time": instant.time(), "datetime": instant,
            "datetime_utc": instant.replace(tzinfo=dt.timezone.utc),
            "ipnetwork": "192.0.2.7/24", "mac_address": b"\x00\x01\x02\x03\xfe\xff",
            "vector": [1.5, -0.0, float("inf")],
            "interval": Interval(1, -2, 3_000_004),
        }
        ranges = {f"{name}{kind}" for name in ("int4", "int8", "num", "date", "ts", "tstz")
                  for kind in ("range", "multirange")}
        self.assertEqual(set(cases) | ranges | {"array", "enum", "created_range", "created_multirange"},
                         set(capabilities()["value_types"]))
        for kind, original in cases.items():
            with self.subTest(kind=kind):
                value = Value(original, kind)
                self.assertEqual(value.kind, kind)
                self.assertEqual(value.value, original)
                self.assertEqual(Value(value.value, kind), value)
                self.assertFalse(value.is_null)
                null = Value.null(kind)
                self.assertTrue(null.is_null)
                self.assertIsNone(null.value)
                self.assertEqual(null, Value(None, kind))
                self.assertNotEqual(null, value)

    def test_integer_bounds_and_boolean_distinction(self):
        for kind, low, high in [
            ("i8", -128, 127), ("i16", -(2**15), 2**15 - 1),
            ("i32", -(2**31), 2**31 - 1), ("i64", -(2**63), 2**63 - 1),
            ("u32", 0, 2**32 - 1), ("u64", 0, 2**64 - 1),
        ]:
            with self.subTest(kind=kind):
                for valid in [low, high]:
                    self.assertEqual(Value(valid, kind).value, valid)
                for invalid in [low - 1, high + 1, True, False, 1.0, "1"]:
                    with self.assertRaises(ConstructionError):
                        Value(invalid, kind)
        self.assertEqual(Value(True).kind, "bool")
        self.assertEqual(Value(1).kind, "i64")
        self.assertNotEqual(Value(1, "i32"), Value(1, "i64"))

    # [spec:pgorm:req:python.value-tags+1/test]
    def test_float_bits_survive_snapshots(self):
        numbers = [0.0, -0.0, math.inf, -math.inf, math.nan,
                   struct.unpack(">d", bytes.fromhex("7ff8000000000123"))[0],
                   struct.unpack(">d", bytes.fromhex("7ff0000000000123"))[0]]
        for number in numbers:
            with self.subTest(bits=bits(number)):
                value = Value(number)
                self.assertEqual(bits(value.value), bits(number))
                self.assertEqual(value.snapshot()["data"], bits(number))
                self.assertEqual(Value(value.value), value)
                json.dumps(value.snapshot(), allow_nan=False)
        self.assertNotEqual(Value(0.0), Value(-0.0))
        for number in [1.5, -0.0, math.inf, -math.inf, math.nan]:
            value = Value(number, "f32")
            self.assertEqual(bits(value.value), bits(number))
            self.assertEqual(value.snapshot()["data"], struct.pack(">f", number).hex())
        for invalid in [0.1, 1e100, 1, 2**24 + 1.0]:
            with self.assertRaises(ConstructionError):
                Value(invalid, "f32")

    def test_decimal_preserves_exact_coefficient_and_scale(self):
        for number in ["79228162514264337593543950335", "-0.000", "1E-28",
                       "1.2345678901234567890123456789", "1E+20"]:
            original = Decimal(number)
            value = Value(original)
            self.assertEqual(value.value, original)
            if original.as_tuple().exponent <= 0:
                self.assertEqual(value.value.as_tuple(), original.as_tuple())
            self.assertEqual(Decimal(value.snapshot()["data"]).as_tuple(), value.value.as_tuple())
        for invalid in [Decimal("79228162514264337593543950336"), Decimal("1E-29"),
                        Decimal("1.23456789012345678901234567891"), Decimal("NaN"),
                        Decimal("Infinity"), Decimal("1E+999999"), 0.1, "0.1", True]:
            with self.assertRaises(ConstructionError):
                Value(invalid, "decimal")

    def test_json_null_and_sql_null_remain_distinct(self):
        json_null = Value.json(None)
        sql_null = Value.null("json")
        self.assertEqual(json_null.value, sql_null.value)
        self.assertNotEqual(json_null, sql_null)
        self.assertFalse(json_null.is_null)
        self.assertTrue(sql_null.is_null)
        self.assertNotEqual(json_null.snapshot(), sql_null.snapshot())
        original = {"big": 2**64 - 1, "nested": [None, True, -0.0, {"雪": "\\%\"'"}]}
        value = Value.json(original)
        self.assertEqual(value.value, original)
        self.assertEqual(bits(value.value["nested"][2]), bits(-0.0))
        original["nested"].clear()
        self.assertEqual(len(value.value["nested"]), 4)
        cyclic = []
        cyclic.append(cyclic)
        for invalid in [2**64, -(2**63) - 1, math.inf, math.nan, {1: "key"},
                        {"decimal": Decimal("1")}, cyclic, b"bytes"]:
            with self.assertRaises(ConstructionError):
                Value.json(invalid)

    def test_array_identity_and_owned_elements(self):
        original = [1, None, Value(3, "i32")]
        value = Value.array("i32", original)
        original.clear()
        self.assertEqual(value.value, [1, None, 3])
        self.assertEqual(value.items(), [Value(1, "i32"), Value.null("i32"), Value(3, "i32")])
        self.assertEqual(value.element_type, "i32")
        value.value.append(4)
        self.assertEqual(len(value.items()), 3)
        self.assertNotEqual(Value.array("i32", []), Value.array("text", []))
        null = Value.array("i32", None)
        self.assertTrue(null.is_null)
        self.assertIsNone(null.items())
        self.assertNotEqual(null, Value.array("i32", []))
        for invalid in [[True], [2**31], [Value(1, "i64")], [[1]], "1", {1}]:
            with self.assertRaises(ConstructionError):
                Value.array("i32", invalid)
        with self.assertRaises(TypeError):
            Value.array(1, [1])
        jsons = Value.array("json", [Value.json(None), None])
        self.assertEqual([item.is_null for item in jsons.items()], [False, True])

    def test_enum_type_identity_is_qualified(self):
        kind = TypeName("Mood\"雪", schema="Tenant'A")
        value = Value("calm'%;\\", kind)
        self.assertEqual(value.kind, "enum")
        self.assertEqual(value.type_name, kind)
        self.assertEqual(value.value, "calm'%;\\")
        self.assertNotEqual(value, Value(value.value, TypeName(kind.name, schema="tenant_b")))
        self.assertTrue(Value.null(kind).is_null)
        array = Value.array(kind, [value, None])
        self.assertEqual(array.element_type, kind)
        self.assertEqual(array.type_name, kind)
        self.assertEqual(Value.array(kind, None).type_name, kind)
        self.assertIsNone(Value.array("i32", [1]).type_name)
        self.assertEqual(array.items()[0].type_name, kind)
        self.assertEqual(array.snapshot()["type"]["element"]["schema"], kind.schema)
        with self.assertRaises(ConstructionError):
            Value.array(kind, [Value(value.value, TypeName("different"))])
        for name in ["", "a" * 64, "雪" * 22, "nul\0"]:
            with self.assertRaises(ConstructionError):
                TypeName(name)
        for name in [None, 1, b"bytes"]:
            with self.assertRaises(TypeError):
                TypeName(name)
        with self.assertRaises(TypeError):
            TypeName("Mood", schema=1)

    def test_temporal_policies_are_explicit(self):
        naive = dt.datetime(2024, 11, 3, 1, 30, 0, 999999)
        utc = naive.replace(tzinfo=dt.timezone.utc)
        aware = Value(utc, "datetime_utc")
        self.assertEqual(aware.value, utc)
        self.assertEqual(aware.value.utcoffset(), dt.timedelta(0))
        self.assertEqual(Value(aware.value, "datetime_utc"), aware)
        # An offset is validated, never stored, so a caller converts in Python.
        # This ambiguous local time resolves to two instants; both survive.
        instants = set()
        for fold in [0, 1]:
            eastern = naive.replace(tzinfo=ZoneInfo("America/New_York"), fold=fold)
            converted = Value(eastern.astimezone(dt.timezone.utc), "datetime_utc")
            self.assertEqual(converted.value.timestamp(), eastern.timestamp())
            instants.add(converted.value)
        self.assertEqual(len(instants), 2)
        for data, kind in [(utc, "datetime"), (naive, "datetime_utc"),
                           (naive.replace(fold=1), "datetime"),
                           (eastern, "datetime_utc"), (utc, "date"),
                           (dt.time(tzinfo=dt.timezone.utc), "time"),
                           (utc.replace(tzinfo=dt.timezone(dt.timedelta(microseconds=1))), "datetime_utc")]:
            with self.subTest(kind=kind, value=data):
                with self.assertRaises(ConstructionError):
                    Value(data, kind)
        for retired in ("datetime_fixed", "datetime_local"):
            with self.subTest(kind=retired), self.assertRaises(ConstructionError):
                Value(utc, retired)
        with self.assertRaises(ConstructionError):
            Value(utc)

    def test_unsupported_values_never_stringify(self):
        class Custom:
            def __str__(self):
                raise AssertionError("must not stringify")

        for invalid in [None, Custom(), complex(1), 2**63, (1, 2), [Custom()]]:
            with self.assertRaises(ConstructionError):
                Value(invalid)
        for data, kind in [(Custom(), "text"), (1, "text"), (True, "boolz"),
                           ("xx", "char"), ("bad", "ipnetwork"), (b"short", "mac_address"),
                           ([0.1], "vector"), (b"bytes", "text")]:
            with self.assertRaises(ConstructionError):
                Value(data, kind)
        value = Value(1)
        with self.assertRaises(AttributeError):
            value.kind = "text"
        for kind in [1, None.__class__, b"i32"]:
            with self.subTest(kind=kind), self.assertRaises(TypeError):
                Value(1, kind)
            with self.subTest(kind=kind), self.assertRaises(TypeError):
                Value.null(kind)

    # [spec:pgorm:req:python.values+7/test]
    def test_lists_infer_as_arrays_dicts_as_json(self):
        cases = [
            ([1, None, 3], "i64", [1, None, 3]),
            (["a", "b"], "text", ["a", "b"]),
            ([True], "bool", [True]),
            ([1.5, None], "f64", [1.5, None]),
            ([Decimal("1.5")], "decimal", [Decimal("1.5")]),
            ([Interval(1, 2, 3)], "interval", [Interval(1, 2, 3)]),
            ([{"a": 1}, None, {"b": [2]}], "json", [{"a": 1}, None, {"b": [2]}]),
        ]
        for data, element, expected in cases:
            with self.subTest(data=data):
                value = Value(data)
                self.assertEqual(value.kind, "array")
                self.assertEqual(value.element_type, element)
                self.assertEqual(value.value, expected)
                self.assertEqual(value, Value.array(element, data))
        document = {"a": [1, None], "b": {"c": "雪"}}
        self.assertEqual(Value(document), Value.json(document))
        self.assertEqual(Value(document).kind, "json")
        self.assertEqual(Value({}).value, {})
        for invalid in [
            [], [None], [None, None], [1, "a"], [1, 1.0], [True, 1], [[1]], [(1,)],
            [1, [2]], (1,), (), [dt.datetime.now(dt.timezone.utc)],
            [Value(1, "i32")],
        ]:
            with self.subTest(data=invalid), self.assertRaises(ConstructionError):
                Value(invalid)

    # [spec:pgorm:req:bindings.values/test]
    # [spec:pgorm:req:python.values+7/test]
    def test_interval_keeps_three_signed_fields(self):
        interval = Interval(1, -2, 3_000_004)
        self.assertEqual((interval.months, interval.days, interval.microseconds), (1, -2, 3_000_004))
        self.assertEqual(Interval(), Interval(0, 0, 0))
        self.assertEqual(Interval(days=1), Interval(0, 1, 0))
        self.assertEqual(hash(Interval(days=1)), hash(Interval(0, 1, 0)))
        # No field is normalised into another, as PostgreSQL's own `=` would.
        self.assertNotEqual(Interval(months=1), Interval(days=30))
        self.assertNotEqual(Interval(days=1), Interval(microseconds=86_400_000_000))
        self.assertEqual(str(interval), "+1 months -2 days +3000004 microseconds")
        self.assertEqual(repr(interval), "Interval(months=1, days=-2, microseconds=3000004)")
        with self.assertRaises(AttributeError):
            interval.months = 2

        value = Value(interval)
        self.assertEqual(value.kind, "interval")
        self.assertEqual(value.value, interval)
        self.assertEqual(value.snapshot()["data"],
                         {"months": "1", "days": "-2", "microseconds": "3000004"})
        self.assertEqual(Value.array("interval", [interval, None]).value, [interval, None])
        for extreme in (Interval(2**31 - 1, 2**31 - 1, 2**63 - 1),
                        Interval(-(2**31), -(2**31), -(2**63))):
            self.assertEqual(Value(extreme, "interval").value, extreme)

        for invalid in (dict(months=2**31), dict(days=-(2**31) - 1), dict(microseconds=2**63),
                        dict(months=True), dict(days=1.0), dict(microseconds="1")):
            with self.subTest(fields=invalid), self.assertRaises(ConstructionError):
                Interval(**invalid)
        # A timedelta has no months and only one signed field, so it is no interval.
        for data in (dt.timedelta(hours=-1), "1 day", 1):
            with self.subTest(data=data), self.assertRaises(ConstructionError):
                Value(data, "interval")
        with self.assertRaises(ConstructionError):
            Value(dt.timedelta(days=1))

    # [spec:pgorm:req:python.value-tags+1/test]
    def test_snapshot_is_json_compatible_and_detached(self):
        value = Value.array("f64", [math.nan, -0.0, None])
        snapshot = value.snapshot()
        self.assertEqual(snapshot["version"], 1)
        self.assertEqual(json.loads(json.dumps(snapshot, allow_nan=False)), snapshot)
        snapshot["type"]["element"]["kind"] = "text"
        self.assertEqual(value.snapshot()["type"]["element"]["kind"], "f64")


if __name__ == "__main__":
    unittest.main()
