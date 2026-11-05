"""Range and multirange values through the installed native module, without a database."""

import datetime as dt
from decimal import Decimal
import json
import unittest

from pgorm import ConstructionError, Multirange, Range, Value, capabilities

RANGES = {
    "int4range": ("i32", 1, 5),
    "int8range": ("i64", -(2**63), 2**63 - 1),
    "numrange": ("decimal", Decimal("-1.50"), Decimal("2.000")),
    "daterange": ("date", dt.date(2024, 1, 1), dt.date(2024, 2, 1)),
    "tsrange": ("datetime", dt.datetime(2024, 1, 1, 12), dt.datetime(2024, 1, 2)),
    "tstzrange": (
        "datetime_utc",
        dt.datetime(2024, 1, 1, tzinfo=dt.timezone.utc),
        dt.datetime(2024, 1, 2, 0, 0, 0, 1, tzinfo=dt.timezone.utc),
    ),
}


# [spec:pgorm:req:bindings.values/test]
# [spec:pgorm:req:python.values+7/test]
# [spec:pgorm:req:bindings.value-tags+1/test]
class RangeTests(unittest.TestCase):
    def test_range_shapes_and_equality(self):
        self.assertEqual(Range(None, 5, "[]"), Range(None, 5, "(]"))
        self.assertEqual(Range(None, None, "[]").bounds, "()")
        self.assertNotEqual(Range(), Range.empty())
        self.assertTrue(Range.empty().is_empty)
        self.assertFalse(Range().is_empty)
        self.assertTrue(Range().lower_inf and Range().upper_inf)
        self.assertFalse(Range.empty().lower_inf)
        self.assertEqual((Range(1, 5).lower_inc, Range(1, 5).upper_inc), (True, False))
        self.assertNotEqual(Range(1, 5), Range(1, 5, "[]"))
        self.assertEqual(hash(Range(1, 5)), hash(Range(1, 5)))
        self.assertEqual(Range(1, 5, None), Range(1, 5))
        for bounds in ["[[", "", "[)x", 1]:
            with self.assertRaises(ConstructionError):
                Range(1, 5, bounds)
        with self.assertRaises(AttributeError):
            Range(1, 5).lower = 2
        self.assertEqual(list(Multirange([Range(1, 2), Range.empty()])), [Range(1, 2), Range.empty()])
        self.assertEqual(len(Multirange([Range(1, 2)])), 1)
        self.assertEqual(Multirange([Range(1, 2)])[-1], Range(1, 2))
        with self.assertRaises(IndexError):
            Multirange()[0]
        with self.assertRaises(ConstructionError):
            Multirange([(1, 2)])

    def test_every_built_in_range_converts_both_ways(self):
        kinds = set(capabilities()["value_types"])
        for kind, (element, lower, upper) in RANGES.items():
            multi = kind.replace("range", "multirange")
            self.assertLessEqual({kind, multi}, kinds)
            for original in [Range(lower, upper), Range(lower, upper, "(]"), Range(None, upper),
                             Range(lower, None, "[]"), Range(), Range.empty()]:
                with self.subTest(kind=kind, range=original):
                    value = Value(original, kind)
                    self.assertEqual(value.kind, kind)
                    self.assertEqual(value.value, original)
                    self.assertEqual(Value(value.value, kind), value)
            sets = Multirange([Range(lower, upper), Range.empty()])
            value = Value(sets, multi)
            self.assertEqual(value.kind, multi)
            self.assertEqual(value.value, sets)
            self.assertEqual(Value(Multirange(), multi).value, Multirange())
            for null in [Value.null(kind), Value(None, kind), Value.null(multi)]:
                self.assertTrue(null.is_null)
                self.assertIsNone(null.value)
            # A bound of the wrong element type is refused as that scalar is.
            with self.assertRaises(ConstructionError):
                Value(Range("1", None), kind)
        self.assertNotEqual(Value.null("int4range"), Value.null("int8range"))
        self.assertNotEqual(Value.null("int4range"), Value.null("int4multirange"))

    def test_ranges_need_an_explicit_kind(self):
        for data in [Range(1, 5), Multirange([Range(1, 5)])]:
            with self.assertRaises(ConstructionError):
                Value(data)
        for data, kind in [((1, 5), "int4range"), (Range(1, 5), "int4multirange"),
                           (Multirange([Range(1, 5)]), "int4range"),
                           ([Range(1, 5)], "int4multirange"),
                           (Range(True, None), "int4range"), (Range(2**31, None), "int4range")]:
            with self.subTest(kind=kind, data=data), self.assertRaises(ConstructionError):
                Value(data, kind)

    def test_arrays_of_ranges_keep_their_element_identity(self):
        value = Value.array("int4range", [Range(1, 3), None, Range.empty()])
        self.assertEqual(value.element_type, "int4range")
        self.assertEqual(value.value, [Range(1, 3), None, Range.empty()])
        self.assertEqual([item.kind for item in value.items()], ["int4range"] * 3)
        self.assertNotEqual(Value.array("int4range", []), Value.array("int8range", []))

    # [spec:pgorm:req:python.value-tags+1/test]
    def test_range_snapshots_are_tagged_and_lossless(self):
        value = Value(Range(Decimal("-1.50"), None, "[)"), "numrange")
        snapshot = value.snapshot()
        self.assertEqual(snapshot["type"], {"kind": "numrange"})
        self.assertEqual(snapshot["data"], {"lower": "-1.50", "upper": None, "bounds": "[)"})
        self.assertEqual(Value(Range.empty(), "int4range").snapshot()["data"], {"empty": True})
        self.assertIsNone(Value.null("int4range").snapshot()["data"])
        multi = Value(Multirange([Range(1, 2)]), "int8multirange").snapshot()
        self.assertEqual(multi["data"], [{"lower": "1", "upper": "2", "bounds": "[)"}])
        json.dumps(snapshot, allow_nan=False)


if __name__ == "__main__":
    unittest.main()
