"""Range and multirange columns and parameters against PostgreSQL, through the installed wheel."""

import datetime as dt
from decimal import Decimal
import os
import unittest
from uuid import uuid4

import pgorm as p

UTC = dt.timezone.utc


def pinned(value, name):
    return p.bind(value).cast(p.TypeName(name, schema="pg_catalog"))


# [spec:pgorm:req:bindings.values/test]
# [spec:pgorm:req:bindings.value-tags+1/test]
class RangeResultTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)

    async def asyncTearDown(self):
        await self.pool.close()

    async def test_every_built_in_range_decodes_canonically(self):
        cases = [
            ("'[1,5]'::int4range", "int4range", p.Range(1, 6)),
            ("'(,3)'::int8range", "int8range", p.Range(None, 3)),
            ("'empty'::numrange", "numrange", p.Range.empty()),
            ("'(1.50,2.000]'::numrange", "numrange", p.Range(Decimal("1.50"), Decimal("2.000"), "(]")),
            ("'[2024-01-01,2024-01-31]'::daterange", "daterange",
             p.Range(dt.date(2024, 1, 1), dt.date(2024, 2, 1))),
            ("'[2024-01-01 12:00,)'::tsrange", "tsrange", p.Range(dt.datetime(2024, 1, 1, 12), None)),
            ("'(,2024-01-01 00:00:00.000001+00]'::tstzrange", "tstzrange",
             p.Range(None, dt.datetime(2024, 1, 1, 0, 0, 0, 1, tzinfo=UTC), "(]")),
            ("'{[5,8),[1,3),[2,4)}'::int4multirange", "int4multirange",
             p.Multirange([p.Range(1, 4), p.Range(5, 8)])),
            ("'{}'::datemultirange", "datemultirange", p.Multirange()),
        ]
        for sql, kind, expected in cases:
            with self.subTest(sql=sql):
                row = await self.pool.one(p.RawSQL(f"SELECT {sql} AS value"))
                self.assertEqual(row["value"], expected)
                self.assertEqual(row.tagged("value"), p.Value(expected, kind))
                null = await self.pool.one(
                    p.RawSQL(f"SELECT CASE WHEN FALSE THEN {sql} END AS value")
                )
                self.assertIsNone(null["value"])
                self.assertEqual(null.tagged("value"), p.Value.null(kind))
        row = await self.pool.one(p.RawSQL(
            "SELECT ARRAY['[1,3)'::int4range, NULL, 'empty'] AS spans,"
            " ARRAY['{[1,2)}'::int8multirange] AS sets"
        ))
        self.assertEqual(row["spans"], [p.Range(1, 3), None, p.Range.empty()])
        self.assertEqual(row.tagged("spans").element_type, "int4range")
        self.assertEqual(row["sets"], [p.Multirange([p.Range(1, 2)])])

    async def test_bound_ranges_read_back_canonical(self):
        written = p.Range(Decimal("-0.50"), Decimal("2.500"), "(]")
        row = await self.pool.one(p.Select(
            pinned(p.Value(written, "numrange"), "numrange").as_("amounts"),
            pinned(p.Value(p.Range(1, 5, "[]"), "int4range"), "int4range").as_("counts"),
            pinned(p.Value(p.Range.empty(), "tstzrange"), "tstzrange").as_("nothing"),
            pinned(p.Value(p.Multirange([p.Range(5, 8), p.Range(1, 6)]), "int8multirange"),
                   "int8multirange").as_("sets"),
            p.bind(p.Value.array("daterange", [p.Range(dt.date(2024, 1, 1), None), None]))
            .cast(p.TypeName("daterange", schema="pg_catalog"), array=True).as_("days"),
        ))
        self.assertEqual(row["amounts"], written)
        self.assertEqual(row["counts"], p.Range(1, 6))
        self.assertEqual(row["nothing"], p.Range.empty())
        self.assertEqual(row["sets"], p.Multirange([p.Range(1, 8)]))
        self.assertEqual(row["days"], [p.Range(dt.date(2024, 1, 1), None), None])
        with self.assertRaises(p.DatabaseError) as inverted:
            await self.pool.one(p.Select(
                pinned(p.Value(p.Range(5, 1), "int4range"), "int4range").as_("n")
            ))
        self.assertEqual(inverted.exception.sqlstate, "22000")

    async def test_raw_sql_compares_bound_ranges(self):
        span = p.Value(p.Range(1, 5), "int4range")
        row = await self.pool.one(p.RawSQL(
            "SELECT $1 && $2::int4range AS overlaps, $1 @> $3::int4range AS contains,"
            " $1::int4range @> 3 AS element",
            [span, p.Value(p.Range(4, 9), "int4range"), p.Value(p.Range(5, 9), "int4range")],
        ))
        self.assertEqual(dict(row), {"overlaps": True, "contains": False, "element": True})

    async def test_created_ranges_read_as_their_kind(self):
        schema = "python_range_" + uuid4().hex
        async with self.pool.connection() as connection:
            await connection.execute(p.RawSQL(f'CREATE SCHEMA "{schema}"'))
            try:
                for name, subtype in (("slot", "int4"), ("floatrange", "float8")):
                    await connection.execute(p.RawSQL(
                        f'CREATE TYPE "{schema}".{name} AS RANGE (SUBTYPE = {subtype})'
                    ))
                row = await connection.one(p.RawSQL(
                    f"""SELECT '[1,3)'::"{schema}".slot AS n, '[1.5,2)'::"{schema}".floatrange AS f"""
                ))
                self.assertEqual((row["n"], row["f"]), (p.Range(1, 3), p.Range(1.5, 2.0)))
                self.assertEqual(row.tagged("n").created_type, p.CreatedRange("slot", "i32", schema=schema))
                self.assertEqual(row.tagged("f").created_type,
                                 p.CreatedRange("floatrange", "f64", schema=schema))
                # The driver reports a created multirange as a simple type.
                with self.assertRaises(p.DecodeError):
                    await connection.one(p.RawSQL(
                        f"""SELECT '{{[1,3)}}'::"{schema}".slot_multirange AS n"""
                    ))
            finally:
                await connection.execute(p.RawSQL(f'DROP SCHEMA "{schema}" CASCADE'))

if __name__ == "__main__":
    unittest.main()
