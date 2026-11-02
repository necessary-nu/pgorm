"""Range types a schema created, through the installed wheel and against PostgreSQL."""

import datetime as dt
from decimal import Decimal
import os
import unittest
from uuid import UUID, uuid4

import pgorm as p
from pgorm import pipeline

UTC = dt.timezone.utc
SUBTYPES = [
    ("int2", "i16", p.Range(-3, 7, "[]")),
    ("int4", "i32", p.Range(1, 5, "[]")),
    ("int8", "i64", p.Range(None, 2**40)),
    ("float4", "f32", p.Range(0.5, 1.25, "(]")),
    ("float8", "f64", p.Range(1.5, float("inf"))),
    ("numeric", "decimal", p.Range(Decimal("-0.50"), Decimal("2.500"))),
    ("text", "text", p.Range('say "hi", (x)', "z\\]$1")),
    ("date", "date", p.Range(dt.date(2024, 1, 1), dt.date(2024, 2, 1))),
    ("time", "time", p.Range(dt.time(9, 0), dt.time(17, 30, 0, 1))),
    ("timestamp", "datetime", p.Range(dt.datetime(2024, 1, 1, 12), None)),
    ("timestamptz", "datetime_utc", p.Range(None, dt.datetime(2024, 1, 1, tzinfo=UTC), "(]")),
    ("uuid", "uuid", p.Range(UUID(int=1), UUID(int=2))),
]


# [spec:pgorm:req:bindings.created-ranges+1/test]
class CreatedRangeValueTests(unittest.TestCase):
    def test_kind_names_the_type_and_its_subtype(self):
        kind = p.CreatedRange('Span "雪"', "f64", schema="measure")
        self.assertEqual((kind.name, kind.schema, kind.subtype), ('Span "雪"', "measure", "f64"))
        self.assertEqual(kind, p.CreatedRange('Span "雪"', "f64", schema="measure"))
        self.assertEqual(hash(kind), hash(p.CreatedRange('Span "雪"', "f64", schema="measure")))
        self.assertNotEqual(kind, p.CreatedRange('Span "雪"', "f32", schema="measure"))
        self.assertNotEqual(p.CreatedRange("r", "i32"), p.CreatedMultirange("r", "i32"))
        for subtype in ("bool", "i8", "u32", "json", "int4range", "bytes"):
            with self.subTest(subtype=subtype), self.assertRaises(p.ConstructionError):
                p.CreatedRange("r", subtype)
        with self.assertRaises(p.ConstructionError):
            p.CreatedRange("", "i32")

    def test_value_holds_the_text_form(self):
        kind = p.CreatedRange("floatrange", "f64", schema="measure")
        value = p.Value(p.Range(1.5, None), kind)
        self.assertEqual(value.kind, "created_range")
        self.assertEqual(value.created_type, kind)
        self.assertEqual(value.type_name, p.TypeName("floatrange", schema="measure"))
        self.assertEqual(value.value, p.Range(1.5, None))
        self.assertEqual(value.snapshot(), {
            "version": 1, "sql_null": False, "data": "[1.5,)",
            "type": {"kind": "created_range", "name": "floatrange", "schema": "measure", "subtype": "f64"},
        })
        self.assertEqual(p.Value(" [ 1.5 ,) ", kind), value)
        with self.assertRaises(p.ConstructionError):
            # Whitespace inside the brackets belongs to the bound, as in range_in.
            p.Value("[1.5, )", kind)
        self.assertEqual(p.Value("empty", kind).value, p.Range.empty())
        null = p.Value.null(kind)
        self.assertTrue(null.is_null)
        self.assertIsNone(null.value)
        self.assertEqual(null.created_type, kind)
        spans = p.CreatedMultirange("floatmultirange", "f64")
        multi = p.Value(p.Multirange([p.Range(5.0, 8.0), p.Range.empty()]), spans)
        self.assertEqual(multi.snapshot()["data"], "{[5,8),empty}")
        self.assertEqual(multi.value, p.Multirange([p.Range(5.0, 8.0), p.Range.empty()]))

    def test_bounds_convert_with_the_subtypes_limits(self):
        for subtype, data in [
            ("i16", p.Range(0, 2**15)),
            ("i32", p.Range(True, 2)),
            ("f64", p.Range(1, 2)),
            ("decimal", p.Range(1.5, None)),
            ("datetime_utc", p.Range(dt.datetime(2024, 1, 1), None)),
            ("i32", "[1,2"),
            ("i32", "[a,b)"),
            ("i32", p.Multirange([p.Range(1, 2)])),
        ]:
            with self.subTest(subtype=subtype, data=data), self.assertRaises(p.ConstructionError):
                p.Value(data, p.CreatedRange("r", subtype))
        with self.assertRaises(p.ConstructionError):
            p.Value(p.Range(1, 2), p.CreatedMultirange("r", "i32"))
        with self.assertRaises(p.ConstructionError):
            p.Value.array(p.CreatedRange("r", "i32"), [])

    def test_writes_cast_text_to_the_type(self):
        value = p.Value(p.Range("a'b", None), p.CreatedRange('Span "雪"', "text", schema="Me'asure"))
        bound = p.bind(value).inspect()
        self.assertEqual(bound.sql, 'SELECT CAST($1::text AS "Me\'asure"."Span ""雪""")')
        self.assertEqual([param.snapshot()["data"] for param in bound.params], ["[a'b,)"])
        self.assertEqual(p.literal(value).inspect().sql,
                         'SELECT CAST(E\'[a\\\'b,)\' AS "Me\'asure"."Span ""雪""")')
        self.assertEqual(p.literal(value).inspect().params, [])

    def test_untyped_paths_refuse_a_created_range(self):
        value = p.Value(p.Range(1, 2), p.CreatedRange("slot", "i32"))
        with self.assertRaises(p.ConstructionError):
            p.JsonDefault(value)
        for operation in (
            lambda: pipeline.literal(value),
            lambda: pipeline.from_(p.Table("t")).filter(pipeline.col("t", "slot") == value),
        ):
            with self.assertRaises(p.ConstructionError):
                operation()


# [spec:pgorm:req:bindings.created-ranges+1/test]
class CreatedRangeResultTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.context = self.pool.connection()
        self.connection = await self.context.__aenter__()
        self.schema = 'created "雪" ' + uuid4().hex[:8]
        await self.connection.execute(p.RawSQL(f"CREATE SCHEMA {self.quoted(self.schema)}"))

    async def asyncTearDown(self):
        await self.connection.execute(p.RawSQL(f"DROP SCHEMA {self.quoted(self.schema)} CASCADE"))
        await self.context.__aexit__(None, None, None)
        await self.pool.close()

    @staticmethod
    def quoted(name):
        return '"' + name.replace('"', '""') + '"'

    async def create(self, name, subtype):
        await self.connection.execute(p.RawSQL(
            f"CREATE TYPE {self.quoted(self.schema)}.{self.quoted(name)} AS RANGE (SUBTYPE = {subtype})"))
        return p.CreatedRange(name, dict((sql, kind) for sql, kind, _ in SUBTYPES)[subtype],
                              schema=self.schema)

    async def test_every_subtype_round_trips_bound_and_literal(self):
        for sql, kind, written in SUBTYPES:
            with self.subTest(subtype=sql):
                created = await self.create(f"Range {sql}", sql)
                value = p.Value(written, created)
                row = await self.connection.one(p.Select(
                    p.bind(value).as_("bound"), p.literal(value).as_("inline")))
                self.assertEqual(row["bound"], written)
                self.assertEqual(row["inline"], written)
                self.assertEqual(row.tagged("bound"), value)

    async def test_built_in_subtype_keeps_its_name(self):
        slot = await self.create("slot", "int4")
        row = await self.connection.one(p.RawSQL(
            f"SELECT '[1,5]'::{self.quoted(self.schema)}.slot AS n, NULL::{self.quoted(self.schema)}.slot AS absent"))
        # A created range is continuous, so the server keeps [1,5] as written.
        self.assertEqual(row["n"], p.Range(1, 5, "[]"))
        self.assertEqual(row.tagged("n").created_type, slot)
        self.assertIsNone(row["absent"])
        self.assertEqual(row.tagged("absent"), p.Value.null(slot))
        table = p.Table("spans", schema=self.schema)
        await self.connection.execute(p.RawSQL(
            f"CREATE TABLE {self.quoted(self.schema)}.spans (span {self.quoted(self.schema)}.slot)"))
        await self.connection.execute(p.Insert(table).columns("span").values(p.literal(row.tagged("n"))))
        # A built-in int4range is a constructor call inline, which the column refuses.
        with self.assertRaises(p.DatabaseError) as built_in:
            await self.connection.execute(
                p.Insert(table).columns("span").values(p.literal(p.Value(p.Range(1, 5), "int4range"))))
        self.assertEqual(built_in.exception.sqlstate, "42804")
        stored = await self.connection.one(p.Select(p.col("span")).from_(table))
        self.assertEqual(stored["span"], p.Range(1, 5, "[]"))

    async def test_server_refuses_what_type_cannot_hold(self):
        slot = await self.create("slot", "int4")
        with self.assertRaises(p.DatabaseError) as inverted:
            await self.connection.one(p.Select(p.bind(p.Value(p.Range(5, 1), slot)).as_("n")))
        self.assertEqual(inverted.exception.sqlstate, "22000")
        with self.assertRaises(p.DatabaseError) as missing:
            await self.connection.one(
                p.Select(p.bind(p.Value(p.Range(1, 2), p.CreatedRange("absent", "i32"))).as_("n")))
        self.assertEqual(missing.exception.sqlstate, "42704")

    async def test_created_multirange_travels_as_text(self):
        await self.create("floatrange", "float8")
        spans = p.CreatedMultirange("floatmultirange", "f64", schema=self.schema)
        value = p.Value(p.Multirange([p.Range(5.0, 8.0), p.Range(1.0, 3.0), p.Range(2.0, 4.0)]), spans)
        text = p.bind(value).cast(p.TypeName("text", schema="pg_catalog")).as_("text")
        row = await self.connection.one(p.Select(text))
        self.assertEqual(p.Value(row["text"], spans).value, p.Multirange([p.Range(1.0, 4.0), p.Range(5.0, 8.0)]))
        with self.assertRaises(p.DecodeError):
            await self.connection.one(p.Select(p.bind(value).as_("spans")))

    async def test_domain_subtype_reads_as_its_base(self):
        schema = self.quoted(self.schema)
        for statement in (
            f"CREATE DOMAIN {schema}.height AS float8",
            f"CREATE DOMAIN {schema}.tall AS {schema}.height CHECK (VALUE > 0)",
            f"CREATE TYPE {schema}.heightrange AS RANGE (SUBTYPE = {schema}.tall)",
            f"CREATE TABLE {schema}.heights (id int PRIMARY KEY, span {schema}.heightrange, "
            f"spans {schema}.heightmultirange)",
        ):
            await self.connection.execute(p.RawSQL(statement))
        span = p.Value(p.Range(1.5, 2.0), p.CreatedRange("heightrange", "f64", schema=self.schema))
        spans = p.Value(p.Multirange([p.Range(1.5, 2.0), p.Range(3.0, None)]),
                        p.CreatedMultirange("heightmultirange", "f64", schema=self.schema))
        row = await self.connection.one(p.Select(p.bind(span).as_("span")))
        self.assertEqual(row.tagged("span"), span)
        table = p.Table("heights", schema=self.schema)
        self.assertEqual(await self.connection.copy_in_rows(p.CopyFrom(table, "binary"), [(1, span, spans)]), 1)
        stream = await self.connection.copy_out_rows(p.CopyTo(table, "binary"))
        async with stream:
            rows = [row async for row in stream]
        self.assertEqual(rows, [(1, span.value, spans.value)])

    async def test_arrays_of_a_created_range_are_refused(self):
        await self.create("slot", "int4")
        with self.assertRaises(p.DecodeError):
            await self.connection.one(p.RawSQL(
                f"SELECT ARRAY['[1,2)'::{self.quoted(self.schema)}.slot] AS spans"))
