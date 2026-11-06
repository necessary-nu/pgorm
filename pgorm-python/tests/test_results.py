"""Installed-wheel result terminals and exact PostgreSQL decoding."""

import datetime as dt
from decimal import Decimal
import math
import os
import unittest
from uuid import UUID, uuid4

import pgorm as p


class ResultTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)

    async def asyncTearDown(self):
        await self.pool.close()

    # [spec:pgorm:req:bindings.results+2/test]
    # [spec:pgorm:req:bindings.row-count/test]
    async def test_cardinality_and_database_errors_are_distinct(self):
        empty = p.Select(p.literal(1).as_("n")).where(p.literal(False))
        self.assertEqual(await self.pool.query(empty), [])
        self.assertIsNone(await self.pool.optional(empty))
        with self.assertRaises(p.DecodeError) as missing:
            await self.pool.one(empty)
        self.assertNotIsInstance(missing.exception, p.DatabaseError)
        self.assertIn("expected exactly one row", str(missing.exception))
        two = p.RawSQL("SELECT generate_series(1, 2) AS n")
        for terminal in (self.pool.one, self.pool.optional):
            with self.assertRaises(p.DecodeError):
                await terminal(two)
        with self.assertRaises(p.DatabaseError) as caught:
            await self.pool.optional(p.RawSQL("SELECT 1 / 0 AS n"))
        self.assertEqual(caught.exception.sqlstate, "22012")
        self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:bindings.results+2/test]
    async def test_builder_writes_return_counts_and_records(self):
        name = "python_result_" + uuid4().hex
        table = p.Table(name)
        async with self.pool.connection() as connection:
            await connection.execute(p.RawSQL(
                f'CREATE TEMP TABLE "{name}" (id integer PRIMARY KEY, name text, visits bigint DEFAULT 0)'
            ))
            query = p.Insert(table).columns("id", "name").values(1, "O'Brien").values(2, "β")
            self.assertEqual(await connection.execute(query), 2)
            update = p.Update(table).set("visits", p.col("visits") + 1).where(p.col("id") == 1)
            row = await connection.one(update.returning(p.col("id"), p.col("visits")))
            self.assertEqual(dict(row), {"id": 1, "visits": 1})
            self.assertEqual(row.tagged("id").kind, "i32")
            self.assertEqual(row.tagged("visits").kind, "i64")
            self.assertEqual(await connection.execute(p.Delete(table).where(p.col("id") == 2)), 1)
            self.assertEqual(await connection.execute(p.Delete(table).where(p.col("id") == 2)), 0)
            selected = await connection.one(p.Select(table.star()).from_(table))
            self.assertEqual(selected["name"], "O'Brien")
            field = selected.fields[0]
            self.assertEqual((field.name, field.index, field.type_name.name, field.type_oid), ("id", 0, "int4", 23))
            self.assertIsNotNone(field.table_oid)
            self.assertEqual(field.column_id, 1)

    # [spec:pgorm:req:bindings.rows/test]
    # [spec:pgorm:req:python.records+1/test]
    async def test_records_are_detached_and_keep_unique_names(self):
        row = await self.pool.one(p.Select(p.bind(7).cast(p.TypeName("int8", schema="pg_catalog")).as_("num"), p.bind("text").as_("name")))
        await self.pool.close()
        self.assertEqual(tuple(row), ("num", "name"))
        self.assertEqual(row.keys(), ("num", "name"))
        self.assertEqual(row.values(), (7, "text"))
        self.assertEqual(row.items(), (("num", 7), ("name", "text")))
        self.assertIn("num", row)
        self.assertEqual(row.get("missing", 42), 42)
        with self.assertRaises(KeyError):
            row["missing"]
        with self.assertRaises(AttributeError):
            row.fields[0].name = "changed"

    # [spec:pgorm:req:bindings.rows/test]
    # [spec:pgorm:req:bindings.values/test]
    async def test_duplicate_names_and_unsupported_types_fail(self):
        for query in ("SELECT 1 AS n, 2 AS n", "SELECT timetz '12:00+02' AS n", "SELECT NULL::timetz AS n"):
            with self.subTest(query=query), self.assertRaises(p.DecodeError):
                await self.pool.optional(p.RawSQL(query))
        self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:bindings.value-tags+1/test]
    async def test_wire_scalar_tags_and_nulls(self):
        cases = [
            ("true::boolean", "bool", True), ("65::\"char\"", "i8", 65),
            ("'-32768'::smallint", "i16", -32768), ("2147483647::integer", "i32", 2147483647),
            ("'-9223372036854775808'::bigint", "i64", -(2**63)), ("4294967295::oid", "u32", 2**32 - 1),
            ("1.5::real", "f32", 1.5), ("1.5::double precision", "f64", 1.5),
            ("'β'::text", "text", "β"), ("'\\x00ff'::bytea", "bytes", b"\x00\xff"),
            ("'{\"a\": [null, true, 3]}'::jsonb", "json", {"a": [None, True, 3]}),
            ("'12345678-1234-5678-1234-567812345678'::uuid", "uuid", UUID("12345678-1234-5678-1234-567812345678")),
            ("'10.0.0.1/24'::inet", "ipnetwork", "10.0.0.1/24"),
            ("'00:11:22:33:44:55'::macaddr", "mac_address", bytes.fromhex("001122334455")),
            ("'1 mon -2 days 00:00:03.000004'::interval", "interval", p.Interval(1, -2, 3_000_004)),
        ]
        for sql, kind, expected in cases:
            with self.subTest(kind=kind):
                row = await self.pool.one(p.RawSQL(f"SELECT {sql} AS value"))
                self.assertEqual(row["value"], expected)
                self.assertEqual(row.tagged("value").kind, kind)
                null = await self.pool.one(p.RawSQL(f"SELECT CASE WHEN FALSE THEN {sql} ELSE NULL END AS value"))
                self.assertIsNone(null["value"])
                self.assertTrue(null.tagged("value").is_null)
                self.assertEqual(null.tagged("value").kind, kind)
        row = await self.pool.one(p.RawSQL("SELECT NULL::jsonb AS sql_null, 'null'::jsonb AS json_null"))
        self.assertIsNone(row["sql_null"])
        self.assertIsNone(row["json_null"])
        self.assertTrue(row.tagged("sql_null").is_null)
        self.assertFalse(row.tagged("json_null").is_null)

    # [spec:pgorm:req:bindings.values/test]
    async def test_intervals_round_trip_through_statements(self):
        cases = [
            p.Interval(1, -2, 3_000_004), p.Interval(-1, 2, -3_000_004), p.Interval(),
            p.Interval(2**31 - 1, 0, 0), p.Interval(-(2**31), 0, 0),
            p.Interval(0, 2**31 - 1, 0), p.Interval(0, -(2**31), 0),
            p.Interval(0, 0, 2**63 - 1), p.Interval(0, 0, -(2**63)),
            p.Interval(2**31 - 1, 2**31 - 1, 2**63 - 1), p.Interval(-(2**31), -(2**31), -(2**63)),
        ]
        name = "python_interval_" + uuid4().hex
        table = p.Table(name)
        async with self.pool.connection() as connection:
            await connection.execute(p.RawSQL(
                f'CREATE TEMP TABLE "{name}" (id integer PRIMARY KEY, span interval, spans interval[])'
            ))
            insert = p.Insert(table).columns("id", "span", "spans")
            for index, interval in enumerate(cases):
                insert = insert.values(index, interval, p.Value.array("interval", [interval, None]))
            self.assertEqual(await connection.execute(insert), len(cases))
            rows = await connection.query(
                p.Select(p.col("span"), p.col("spans")).from_(table).order_by(p.col("id").asc()))
            self.assertEqual([row["span"] for row in rows], cases)
            self.assertEqual([row["spans"] for row in rows], [[case, None] for case in cases])
            self.assertEqual(rows[0].tagged("span").kind, "interval")
            self.assertEqual(rows[0].tagged("spans").element_type, "interval")

            # The server reads the three fields it was sent, and the infinities as such.
            texts = await connection.one(p.Select(
                p.bind(cases[0]).cast(p.TypeName("text")).as_("mixed"),
                p.bind(cases[-2]).cast(p.TypeName("text")).as_("infinite"),
                p.bind(cases[-1]).cast(p.TypeName("text")).as_("negative"),
            ))
            self.assertEqual(texts.values(), ("1 mon -2 days +00:00:03.000004", "infinity", "-infinity"))

            # A predicate binds the interval it compares with; PostgreSQL's `=`
            # holds 1 mon equal to 30 days, and no other row spells this one.
            found = await connection.query(
                p.Select(p.col("id")).from_(table).where(p.col("span") == p.Interval(-1, 2, -3_000_004)))
            self.assertEqual([row["id"] for row in found], [1])
            null = await connection.one(p.Select(
                p.bind(p.Value.null("interval")).cast(p.TypeName("interval")).as_("n")))
            self.assertIsNone(null["n"])
            self.assertEqual(null.tagged("n").kind, "interval")

    # [spec:pgorm:req:python.values+7/test]
    async def test_floating_special_values_survive(self):
        row = await self.pool.one(p.RawSQL(
            "SELECT '-0'::float8 AS zero, 'NaN'::float8 AS nan, 'Infinity'::float4 AS inf"
        ))
        self.assertEqual(math.copysign(1, row["zero"]), -1)
        self.assertTrue(math.isnan(row["nan"]))
        self.assertEqual(row["inf"], float("inf"))

    # [spec:pgorm:req:bindings.values/test]
    async def test_json_numeric_loss_is_explicit(self):
        for kind in ("json", "jsonb"):
            for text in ("18446744073709551616", "[0.10000000000000000001]", '{"n": 1e-400}'):
                with self.subTest(kind=kind, text=text), self.assertRaises(p.DecodeError):
                    await self.pool.optional(p.RawSQL(f"SELECT '{text}'::{kind} AS value"))
            row = await self.pool.one(p.RawSQL(f"SELECT '[0.1,18446744073709551615]'::{kind} AS value"))
            self.assertEqual(row["value"], [0.1, 2**64 - 1])

    # [spec:pgorm:req:bindings.values/test]
    async def test_numeric_is_exact_or_rejected(self):
        for text in ("0.0000", "1.2300", "-42.001", "79228162514264337593543950335", "0.0000000000000000000000000001"):
            row = await self.pool.one(p.RawSQL(f"SELECT '{text}'::numeric AS value"))
            self.assertEqual(row["value"].as_tuple(), Decimal(text).as_tuple())
        for text in ("NaN", "Infinity", "1.00000000000000000000000000001", "79228162514264337593543950336", "7922816251426433759354395033.51"):
            with self.subTest(text=text), self.assertRaises(p.DecodeError):
                await self.pool.optional(p.RawSQL(f"SELECT '{text}'::numeric AS value"))

    # [spec:pgorm:req:python.values+7/test]
    async def test_temporal_precision_and_timezone_policy(self):
        row = await self.pool.one(p.RawSQL(
            "SELECT DATE '2024-02-29' AS d, TIME '12:34:56.123456' AS t, "
            "TIMESTAMP '2024-02-29 12:34:56.123456' AS naive, "
            "TIMESTAMPTZ '2024-02-29 12:34:56.123456+02' AS aware"
        ))
        self.assertEqual(row["d"], dt.date(2024, 2, 29))
        self.assertEqual(row["t"], dt.time(12, 34, 56, 123456))
        self.assertEqual(row["naive"], dt.datetime(2024, 2, 29, 12, 34, 56, 123456))
        self.assertEqual(row["aware"], dt.datetime(2024, 2, 29, 10, 34, 56, 123456, tzinfo=dt.timezone.utc))
        self.assertEqual(row.tagged("aware").kind, "datetime_utc")
        for sql in ("DATE '10000-01-01'", "TIMESTAMP 'infinity'", "TIME '24:00:00'"):
            with self.subTest(sql=sql), self.assertRaises(p.DecodeError):
                await self.pool.one(p.RawSQL(f"SELECT {sql} AS value"))

    # [spec:pgorm:req:bindings.values/test]
    async def test_array_shape_and_elements_are_preserved(self):
        row = await self.pool.one(p.RawSQL(
            "SELECT ARRAY[1,NULL,3]::int4[] AS nums, '{}'::text[] AS empty, NULL::int4[] AS nil, "
            "ARRAY[1.2300,NULL]::numeric[] AS decimals"
        ))
        self.assertEqual(row["nums"], [1, None, 3])
        self.assertEqual(row["empty"], [])
        self.assertIsNone(row["nil"])
        self.assertEqual(row.tagged("nums").element_type, "i32")
        self.assertEqual([v.kind for v in row.tagged("nums").items()], ["i32"] * 3)
        self.assertEqual(row["decimals"][0].as_tuple(), Decimal("1.2300").as_tuple())
        for sql in ("'[0:1]={1,2}'::integer[]", "'{{1,2},{3,4}}'::integer[]", "ARRAY[1.00000000000000000000000000001]::numeric[]"):
            with self.subTest(sql=sql), self.assertRaises(p.DecodeError):
                await self.pool.one(p.RawSQL(f"SELECT {sql} AS value"))

    # [spec:pgorm:req:bindings.value-tags+1/test]
    async def test_enum_identity_survives_scalars_and_arrays(self):
        schema = "python_enum_" + uuid4().hex
        async with self.pool.connection() as connection:
            await connection.execute(p.RawSQL(f'CREATE SCHEMA "{schema}"'))
            try:
                await connection.execute(p.RawSQL(f'CREATE TYPE "{schema}"."Mood" AS ENUM (\'ok\', \'sad\')'))
                kind = p.TypeName("Mood", schema=schema)
                query = p.Select(
                    p.bind(p.Value("ok", kind)).as_("mood"),
                    p.bind(p.Value.array(kind, ["sad", None])).as_("moods"),
                    p.bind(p.Value.null(kind)).as_("nil"),
                )
                row = await connection.one(query)
                self.assertEqual(row["mood"], "ok")
                self.assertEqual(row["moods"], ["sad", None])
                self.assertEqual(row.tagged("mood").type_name, kind)
                self.assertEqual(row.tagged("moods").element_type, kind)
                self.assertEqual(row.tagged("moods").items()[1].type_name, kind)
                self.assertEqual(row.tagged("nil").type_name, kind)
                self.assertTrue(row.tagged("nil").is_null)
            finally:
                await connection.execute(p.RawSQL(f'DROP SCHEMA "{schema}" CASCADE'))

    # [spec:pgorm:req:python.raw/test]
    async def test_execution_requires_an_explicit_query_object(self):
        with self.assertRaises(TypeError):
            await self.pool.query("SELECT 1")
        row = await self.pool.one(p.Select(p.literal("O'Brien").as_("value")).inspect())
        self.assertEqual(row["value"], "O'Brien")

    # [spec:pgorm:sem:error.model.parameter-count/test]    another number of values than the placeholders take is the caller's ConstructionError
    async def test_raw_parameter_count_is_refused(self):
        with self.assertRaises(p.ConstructionError) as caught:
            await self.pool.one(p.RawSQL("SELECT $1 AS x, $2 AS y", [1]))
        self.assertEqual(str(caught.exception), "the statement takes 2 parameters, and 1 was given")
        with self.assertRaises(p.ConstructionError) as caught:
            await self.pool.execute(p.RawSQL("SELECT 1", [1, 2]))
        self.assertEqual(str(caught.exception), "the statement takes 0 parameters, and 2 were given")
        self.assertEqual((await self.pool.one(p.RawSQL("SELECT 1 AS n")))["n"], 1)

    # [spec:pgorm:req:conn.copy.statement/test]    a COPY given as a statement is the caller's ConstructionError, nothing sent
    async def test_copy_as_statement_is_refused(self):
        for run in (
            lambda: self.pool.query(p.RawSQL("COPY (SELECT 1) TO STDOUT")),
            lambda: self.pool.execute(p.RawSQL("/* rows */ copy pg_class FROM STDIN")),
            lambda: self.pool.batch_execute("SELECT 1; COPY pg_class FROM STDIN"),
        ):
            with self.assertRaises(p.ConstructionError) as caught:
                await run()
            self.assertIn("copy_in or copy_out", str(caught.exception))
        self.assertEqual((await self.pool.one(p.RawSQL("SELECT 'COPY' AS copy")))["copy"], "COPY")

    # [spec:pgorm:req:bindings.value-tags+1/test]
    async def test_absent_join_keeps_typed_null_fields(self):
        row = await self.pool.optional(p.RawSQL(
            "SELECT parent.id AS parent_id, child.id AS child_id, child.name AS child_name "
            "FROM (VALUES (1)) AS parent(id) LEFT JOIN (VALUES (2, 'name'::text)) AS child(id, name) "
            "ON parent.id=child.id"
        ))
        self.assertIsNotNone(row)
        self.assertEqual(row["parent_id"], 1)
        self.assertIsNone(row["child_id"])
        self.assertEqual(row.tagged("child_id").kind, "i32")
        self.assertEqual(row.tagged("child_name").kind, "text")


if __name__ == "__main__":
    unittest.main()
