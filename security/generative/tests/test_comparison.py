import unittest

from pgorm_campaign import comparison as c
from pgorm_campaign import wire


def row(value, name="value"):
    return {"kind": "record", "fields": [{"name": name, "value": value}]}


# [spec:pgorm:req:generative.comparison+1/test]
class ComparisonTests(unittest.TestCase):
    def test_unordered_rows_retain_duplicate_counts(self):
        a, b = [row(wire.scalar("i32", str(i))) for i in (1, 2)]
        self.assertTrue(c.rows([a, a, b], [b, a, a], ordered=False).equal)
        self.assertFalse(c.rows([a, a, b], [b, b, a], ordered=False).equal)
        self.assertFalse(c.rows([a, b], [b, a], ordered=True).equal)

    def test_types_nulls_and_optional_slots_stay_distinct(self):
        variants = [
            row(wire.scalar("json", None)),
            row(wire.scalar("json", None, sql_null=True)),
            row(wire.scalar("text", None, sql_null=True)),
            {"kind": "absent"},
            {"kind": "tuple", "items": [{"kind": "absent"}]},
            row(wire.scalar("i32", "1")),
            row(wire.scalar("i64", "1")),
        ]
        for i, a in enumerate(variants):
            for j, b in enumerate(variants):
                self.assertEqual(c.rows([a], [b], ordered=False).equal, i == j)

    def test_exact_numeric_payloads_are_not_rounded(self):
        for kind, first, second in (
            ("f32", "00000000", "80000000"),
            ("f64", "3ff0000000000000", "3ff0000000000001"),
            ("decimal", "123.4500", "123.45"),
        ):
            self.assertFalse(
                c.rows(
                    [row(wire.scalar(kind, first))],
                    [row(wire.scalar(kind, second))],
                    ordered=False,
                ).equal
            )

    def test_temporal_spelling_preserves_the_declared_kind(self):
        a = row(wire.scalar("datetime_utc", "2024-01-02T03:04:05Z"))
        b = row(wire.scalar("datetime_utc", "2024-01-02 03:04:05+00:00"))
        self.assertTrue(c.rows([a], [b], ordered=True).equal)
        # The tag is part of the key, so the same wall clock read as a naive
        # value and as an instant stays two different observations.
        naive = row(wire.scalar("datetime", "2024-01-02T03:04:05"))
        self.assertFalse(c.rows([naive], [b], ordered=True).equal)

    def test_arbitrary_errors_do_not_satisfy_rejections(self):
        actual = {
            "kind": "error",
            "class": "DatabaseError",
            "cause": "duplicate key",
            "sqlstate": "23505",
        }
        self.assertTrue(
            c.exact_error(
                actual, {"class": "DatabaseError", "cause": "sqlstate:23505"}
            ).equal
        )
        self.assertFalse(
            c.exact_error(
                actual, {"class": "DatabaseError", "cause": "sqlstate:42501"}
            ).equal
        )
        self.assertFalse(
            c.exact_error(
                actual, {"class": "ConstructionError", "cause": "duplicate key"}
            ).equal
        )
        self.assertFalse(
            c.exact_error(
                actual, {"class": "DatabaseError", "cause": "duplicate"}
            ).equal
        )

    # [spec:pgorm:req:bindings.errors+6/test]
    def test_constraint_subclasses_satisfy_their_database_rejection(self):
        for sqlstate, subclass in c.DATABASE_SUBCLASS_BY_SQLSTATE.items():
            actual = {
                "kind": "error",
                "class": subclass,
                "cause": "server diagnostic",
                "sqlstate": sqlstate,
            }
            expected = {
                "class": "DatabaseError",
                "cause": "sqlstate:" + sqlstate,
            }
            self.assertTrue(c.exact_error(actual, expected).equal)

        wrong_subclass = {
            "kind": "error",
            "class": "NotNullViolationError",
            "cause": "server diagnostic",
            "sqlstate": "23505",
        }
        self.assertFalse(
            c.exact_error(
                wrong_subclass,
                {"class": "DatabaseError", "cause": "sqlstate:23505"},
            ).equal
        )
        self.assertFalse(
            c.exact_error(
                {**wrong_subclass, "class": "DatabaseError"},
                {
                    "class": "NotNullViolationError",
                    "cause": "sqlstate:23505",
                },
            ).equal
        )

    def test_missing_comparison_is_never_success(self):
        with self.assertRaises(c.InvalidOracle):
            c.observation({"kind": "compiled"}, {"kind": "compiled"})
        with self.assertRaises(c.InvalidOracle):
            c.rows([{"kind": "record", "fields": []}], [], ordered=False)
        self.assertFalse(
            c.observation(
                {"kind": "count", "value": 0}, {"kind": "count", "value": 1}
            ).equal
        )

    def test_stream_invariant_retains_duplicates_and_closure(self):
        a, b = [row(wire.scalar("i32", str(value))) for value in (1, 2)]
        expected = {
            "kind": "rows",
            "rows": [a, b],
            "stream_check": {"take": 1, "cancel": True},
        }
        actual = {
            "kind": "rows",
            "rows": [b],
            "stream": {"complete": False, "cancelled": True, "closed": True},
        }
        self.assertTrue(c.observation(actual, expected, ordered=False).equal)
        self.assertFalse(c.observation(actual, expected, ordered=True).equal)
        actual["stream"]["closed"] = False
        self.assertFalse(c.observation(actual, expected).equal)
        actual["stream"]["closed"] = True
        actual["rows"] = [a, a]
        expected["stream_check"]["take"] = 2
        self.assertFalse(c.observation(actual, expected).equal)
