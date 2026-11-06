"""Installed Python pipeline composition, scope ownership and PostgreSQL results."""

from decimal import Decimal
import os
import unittest

import pgorm as p
from pgorm import pipeline as pl
from pgorm import _native
from pipeline_programs import programs


class PipelineConstruction(unittest.TestCase):
    # [spec:pgorm:req:bindings.pipeline+1/test]
    def test_public_programs_compile_with_native_parameters(self):
        cases = programs(_native)
        self.assertEqual(len(cases), 22)
        for name, query in cases.items():
            with self.subTest(name=name):
                self.assertIsInstance(query.inspect(), p.Compiled)
        query = cases["repeated"].inspect()
        self.assertEqual(
            [(v.kind, v.value) for v in query.params], [("i32", 2), ("i64", 10)]
        )
        self.assertEqual(query.sql.count("$1"), 2)
        self.assertEqual(cases["append"].inspect().params[1].value, 8)
        self.assertEqual(
            p.capabilities()["pipeline_policy"]["source_arities"], list(range(1, 7))
        )
        self.assertEqual(p.capabilities()["registrations"]["sources"], [])
        with self.assertRaises(p.UnsupportedCapabilityError):
            pl.sources("unregistered")

    # [spec:pgorm:req:bindings.pipeline-binder/test]
    def test_scopes_close_on_success_and_exceptions(self):
        base = pl.from_(p.Table("items"))
        key = pl.col("items", "id")
        saved = []

        def callback(binder):
            value = binder.bind(2)
            saved.extend([binder, value])
            return key > value

        query = base.filter_with(callback)
        self.assertEqual(query.inspect().params[0].value, 2)
        binder, value = saved
        for operation in (
            lambda: binder.bind(3),
            lambda: value + 1,
            lambda: base.filter(value),
            lambda: base.filter_with(lambda b: value),
            lambda: pl.over().by(value),
        ):
            with self.assertRaises(p.LifecycleError):
                operation()
        saved.clear()

        def failing(binder):
            saved.append(binder)
            raise ValueError("callback failure")

        with self.assertRaisesRegex(ValueError, "callback failure"):
            base.filter_with(failing)
        with self.assertRaises(p.LifecycleError):
            saved[0].bind(1)
        self.assertEqual(base.inspect().params, [])

    # [spec:pgorm:req:bindings.pipeline-binder/test]
    def test_nested_callbacks_cannot_mix_brands(self):
        base = pl.from_(p.Table("items"))
        key = pl.col("items", "id")

        def outer(binder):
            value = binder.bind(1)
            with self.assertRaises(p.LifecycleError):
                base.filter_with(lambda inner: (key > value) & (key < inner.bind(3)))
            with self.assertRaises(p.LifecycleError):
                base.filter_with(lambda inner: value)
            with self.assertRaises(p.LifecycleError):
                pl.over().sort_by(value)
            independent = base.filter_with(lambda inner: key < inner.bind(9))
            self.assertEqual(independent.inspect().params[0].value, 9)
            return key > value

        self.assertEqual(base.filter_with(outer).inspect().params[0].value, 1)

    # [spec:pgorm:req:bindings.pipeline-binder/test]
    def test_bound_list_limits_and_input_rejections(self):
        base = pl.from_(p.Table("items"))
        self.assertIsInstance(base.derive_with(lambda b: []), pl.Pipeline)
        full = base.select_with(lambda b: [b.bind(i).as_(f"x{i}") for i in range(32)])
        self.assertEqual(len(full.inspect().params), 32)
        key = pl.col("items", "id")
        for operation in (
            lambda: base.select_with(lambda b: [b.bind(i) for i in range(33)]),
            lambda: base.select(*((key + i).as_(f"x{i}") for i in range(33))),
        ):
            with self.assertRaises(p.ConstructionError):
                operation()
        self.assertIsInstance(
            base.select(*(pl.literal(i).as_(f"x{i}") for i in range(33))).inspect(),
            p.Compiled,
        )
        for operation in (
            lambda: base.take(2**64),
            lambda: pl.alias(""),
            lambda: pl.col("items", "bad\0name"),
            lambda: pl.literal(pl.col("items", "id")),
            lambda: bool(base),
            lambda: bool(pl.col("items", "id")),
            lambda: bool(base.group(pl.col("items", "id"))),
        ):
            with self.assertRaises(p.ConstructionError):
                operation()
        for operation in (
            lambda: base.take(True),
            lambda: base.select_with(lambda b: "bad"),
            lambda: key.in_array({1, 2}),
            lambda: pl.case([(key > 1,)], "no"),
            lambda: pl.alias(key),
        ):
            with self.assertRaises(TypeError):
                operation()
        for value in (
            float("inf"), 2**63, b"bytes", p.Value(2, "i16"), p.Value.null("i32")
        ):
            with self.assertRaises(p.ConstructionError):
                pl.literal(value)
        enum = p.Value("calm", p.TypeName("Mood", schema="app"))
        for operation in (
            lambda: base.filter_with(lambda b: b.bind(enum)),
            lambda: base.filter(key == enum),
            lambda: base.filter(key == None),  # noqa: E711
            lambda: base.filter_with(lambda b: b.bind(None)),
        ):
            with self.assertRaises(p.ConstructionError):
                operation()
        with self.assertRaises(p.ConstructionError):
            base.select(pl.literal(1).as_("sum")).inspect()

    # [spec:pgorm:req:bindings.pipeline-binder/test]
    # [spec:pgorm:req:bindings.operands/test]
    # [spec:pgorm:req:python.pipeline+1/test]
    def test_plain_operands_bind_through_their_stage(self):
        base = pl.from_(p.Table("items"))
        key, amount = pl.col("items", "id"), pl.col("items", "amount")
        cases = {
            "filter": base.filter(key > 2),
            "derive": base.derive((amount + 2).as_("more")),
            "select": base.select(pl.sum(5).as_("five")),
            "sort": base.sort(key + 1),
            "group": base.group(amount.coalesce(0)).aggregate(pl.count_rows()),
            "aggregate": base.group(key).aggregate((pl.sum(amount) + 1).as_("t")),
            "window": base.window((pl.sum(amount) + 3).as_("t"), over=pl.over()),
            "join": base.join(
                pl.source(p.Table("items")).named("peer"),
                (key == pl.col("peer", "id")) & (pl.col("peer", "amount") > 1),
            ),
            "case": base.select(pl.case([(key > 1, "yes")], "no").as_("a")),
            "in_array": base.filter(key.in_array([1, 2])),
        }
        for name, query in cases.items():
            with self.subTest(name=name):
                compiled = query.inspect()
                self.assertIn("$1", compiled.sql)
                self.assertTrue(compiled.params)
        query = base.filter(key > 2).inspect()
        self.assertEqual([(v.kind, v.value) for v in query.params], [("i64", 2)])
        self.assertEqual(
            base.filter(key > pl.literal(2)).inspect().params, []
        )
        mixed = base.filter_with(lambda b: (key > b.bind(1)) & (key < 9)).inspect()
        self.assertEqual([v.value for v in mixed.params], [1, 9])
        typed = base.filter(key == p.Value(3, "i16")).inspect()
        self.assertEqual([v.kind for v in typed.params], ["i16"])
        self.assertEqual(pl.null().__class__, pl.PipelineExpr)
        self.assertEqual(base.filter(key.is_null() | (key == pl.null())).inspect().params, [])
        for keys in ((key + 1,), (pl.literal(1), amount * 2)):
            with self.subTest(keys=len(keys)), self.assertRaises(p.ConstructionError):
                pl.over().by(*keys)
            with self.subTest(keys=len(keys)), self.assertRaises(p.ConstructionError):
                pl.over().sort_by(*keys)
        self.assertIsInstance(pl.over().by(pl.literal(1), key), pl.Over)

    # [spec:pgorm:req:bindings.pipeline+1/test]
    # [spec:pgorm:req:pipeline.errors+4/test]
    def test_names_the_screen_refuses_fail_at_compile(self):
        # The binding delegates to the Rust screen at `into_sql`, so it refuses
        # exactly the names Rust refuses, at the same boundary: a name that
        # begins with `$` or is `*` constructs and is refused when compiled.
        base = pl.from_(p.Table("items"))
        for name in ("$1", "$$", "$tag$", "*", 'a"b'):
            cause = (
                f"identifier `{name}` cannot be written as a pipeline name: it "
                "contains a double quote or NUL byte, begins with `$`, or is `*`; "
                "rename it"
            )
            for query in (
                base.select(pl.col("items", name)),
                pl.from_(p.Table(name)),
                base.group(pl.col("items", name)).aggregate(pl.count_rows()),
                base.derive(pl.col("items", "id").as_(name)),
            ):
                with self.subTest(name=name):
                    with self.assertRaises(p.ConstructionError) as refused:
                        query.inspect()
                    self.assertEqual(str(refused.exception), cause)
        inner = base.select(pl.col("items", "a$1")).inspect()
        self.assertEqual(inner.sql, "SELECT a$1 FROM items")
        self.assertEqual(inner.params, [])


class PipelineDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.connection = await self.pool.acquire()
        await self.connection.execute(
            p.RawSQL(
                "CREATE TEMP TABLE items (id integer, category text, amount bigint)"
            )
        )
        await self.connection.execute(
            p.RawSQL("INSERT INTO items VALUES (1,'a',2),(2,'a',3),(3,'b',4),(4,'b',5)")
        )

    async def asyncTearDown(self):
        await self.connection.close()
        await self.pool.close()

    # [spec:pgorm:req:bindings.pipeline+1/test]
    # [spec:pgorm:req:bindings.pipeline-terminals/test]
    # [spec:pgorm:req:bindings.row-count/test]
    async def test_terminals_models_and_dynamic_streams(self):
        model = p.Model(
            "items",
            {
                "id": p.Column("i32"),
                "category": p.Column("text"),
                "amount": p.Column("i64"),
            },
        )
        query = pl.from_(model).sort(pl.col("items", "id"))
        self.assertEqual((await query.one(self.connection))["id"], 1)
        self.assertIn("LIMIT 1", query.inspect(terminal="one").sql)
        with self.assertRaises(p.DecodeError):
            await self.connection.one(query)
        empty = query.filter(pl.col("items", "id") < 0)
        self.assertIsNone(await empty.optional(self.connection))
        with self.assertRaises(p.DecodeError):
            await empty.one(self.connection)
        async with await self.connection.stream(query) as rows:
            found = [row["id"] async for row in rows]
        self.assertEqual(found, [1, 2, 3, 4])
        self.assertEqual([row["id"] for row in await query.all(self.connection)], [1, 2, 3, 4])
        self.assertNotIn("LIMIT", query.inspect().sql)
        self.assertEqual(query.inspect(terminal="optional").sql, query.inspect(terminal="one").sql)
        with self.assertRaises(TypeError):
            query.all(object())

    # [spec:pgorm:req:bindings.pipeline-terminals/test]
    async def test_terminals_run_on_a_pool(self):
        name = pl.col("pg_type", "typname")
        int4 = pl.from_(p.Table("pg_type", schema="pg_catalog")).filter(name == "int4").select(name)
        pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        try:
            self.assertEqual([row["typname"] for row in await int4.all(pool)], ["int4"])
            self.assertEqual((await int4.one(pool))["typname"], "int4")
            self.assertIsNone(await int4.filter(name == "no such type").optional(pool))
            async with pool.transaction() as transaction:
                self.assertEqual((await int4.optional(transaction))["typname"], "int4")
        finally:
            await pool.close()

    # [spec:pgorm:req:bindings.pipeline+1/test]
    async def test_pipeline_stages_execute_in_postgresql(self):
        cases = programs(_native)
        expected = {
            "literal": [4, 3],
            "repeated": [3, 4],
            "literal_string": [],
            "in_array": [1, 2],
            "sort_with": [2, 3, 4],
        }
        for name, identifiers in expected.items():
            with self.subTest(name=name):
                rows = await cases[name].all(self.connection)
                self.assertEqual([row["id"] for row in rows], identifiers)
        self.assertEqual(
            sorted(
                (r["category"], r["total"])
                for r in await cases["group"].all(self.connection)
            ),
            [("a", 5), ("b", 9)],
        )
        self.assertEqual(
            [r["answer"] for r in await cases["case"].all(self.connection)],
            ["no", "yes", "yes", "yes"],
        )
        for name in (
            "derive",
            "derive_with",
            "select_with",
            "group_with",
            "window",
            "window_with",
            "append",
            "intersect",
            "remove",
            "distinct",
            "cast_unary",
            "join",
        ):
            with self.subTest(name=name):
                await cases[name].all(self.connection)

    # [spec:pgorm:req:bindings.pipeline-expressions+3/test]
    # [spec:pgorm:req:bindings.operands/test]
    async def test_literal_and_bound_text_remain_values(self):
        payload = "O'Brien'); DROP TABLE items; -- 雪"
        await self.connection.execute(
            p.Insert(p.Table("items"))
            .columns("id", "category", "amount")
            .values(5, payload, 6)
        )
        base = pl.from_(p.Table("items"))
        key = pl.col("items", "category")
        literal = base.filter(key == pl.literal(payload))
        plain = base.filter(key == payload)
        bound = base.filter_with(lambda b: key == b.bind(payload))
        self.assertEqual(literal.inspect().params, [])
        for query in (plain, bound):
            self.assertNotIn(payload, query.inspect().sql)
            self.assertEqual([v.value for v in query.inspect().params], [payload])
        for query in (literal, plain, bound):
            row = await query.one(self.connection)
            self.assertEqual((row["id"], row["category"]), (5, payload))
        self.assertEqual(len(await base.all(self.connection)), 5)

    # [spec:pgorm:req:bindings.pipeline-expressions+3/test]
    async def test_aggregates_windows_and_typed_nulls(self):
        base = pl.from_(p.Table("items"))
        amount = pl.col("items", "amount")
        category = pl.col("items", "category")
        aggregates = {
            "sum": (pl.sum(amount), 14),
            "min": (pl.min(amount), 2),
            "max": (pl.max(amount), 5),
            "average": (pl.average(amount), 3.5),
            "count": (pl.count(amount), 4),
            "count_rows": (pl.count_rows(), 4),
            "count_distinct": (pl.count_distinct(category), 2),
        }
        for name, (expression, expected) in aggregates.items():
            with self.subTest(name=name):
                row = (
                    await base.group()
                    .aggregate(expression.as_("result"))
                    .one(self.connection)
                )
                self.assertEqual(row["result"], expected)
        for name, expression in {
            "rank": pl.rank(amount),
            "rank_dense": pl.rank_dense(amount),
            "first": pl.first(amount),
            "last": pl.last(amount),
            "lag": pl.lag(1, amount),
            "lead": pl.lead(1, amount),
            "stddev": pl.stddev(amount),
        }.items():
            with self.subTest(name=name):
                query = base.window(
                    expression.as_("result"),
                    over=pl.over().sort_by(amount).range(None, 0),
                )
                self.assertEqual(len(await query.all(self.connection)), 4)
        query = base.select_with(
            lambda b: [b.bind(p.Value.null("i32")).cast("integer").as_("result")]
        ).take(1)
        self.assertEqual(query.inspect().params[0].kind, "i32")
        self.assertIsNone((await query.one(self.connection))["result"])



class PipelineDomainEnums(unittest.IsolatedAsyncioTestCase):
    """A column whose domain is over an enum, which PostgreSQL compares only
    as the enum, read by a pipeline that has no registration to type it."""

    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.connection = await self.pool.acquire()
        await self.drop()
        for statement in (
            "CREATE SCHEMA pipeline_enums",
            "CREATE TYPE pipeline_enums.\"Mood\" AS ENUM ('sad', 'calm', 'glad')",
            "CREATE DOMAIN pipeline_enums.cheerful AS pipeline_enums.\"Mood\"",
            "CREATE TABLE pipeline_enums.lines (id integer PRIMARY KEY, mood pipeline_enums.cheerful NOT NULL)",
            "INSERT INTO pipeline_enums.lines VALUES (1, 'calm'), (2, 'glad'), (3, 'calm'), (4, 'glad')",
            "CREATE TABLE pipeline_enums.tills (id integer PRIMARY KEY, takings money NOT NULL)",
            "INSERT INTO pipeline_enums.tills VALUES (1, 12.35), (2, 3)",
            "CREATE TYPE pipeline_enums.slot AS RANGE (SUBTYPE = int4)",
            "CREATE TABLE pipeline_enums.spans (id integer PRIMARY KEY, span pipeline_enums.slot NOT NULL)",
            "INSERT INTO pipeline_enums.spans VALUES (1, '[1,5)'), (2, '[7,9)')",
        ):
            await self.connection.execute(p.RawSQL(statement))

    async def drop(self):
        await self.connection.execute(
            p.RawSQL("DROP SCHEMA IF EXISTS pipeline_enums CASCADE")
        )

    async def asyncTearDown(self):
        await self.drop()
        await self.connection.close()
        await self.pool.close()

    # [spec:pgorm:req:bindings.pipeline-expressions+3/test]    a column with no registration compares as the
    # schema-qualified enum compares_as names, bound, literal, listed and through an alias,
    # where each left bare is refused with 42883
    async def test_untyped_columns_compare_as_the_named_enum(self):
        base = pl.from_(p.Table("lines", schema="pipeline_enums"))
        mood = pl.col("lines", "mood")
        enum = p.TypeName("Mood", schema="pipeline_enums")
        typed = mood.compares_as(enum)
        operations = p.capabilities()["operations"]
        self.assertEqual(
            operations["pipeline.compares_as"]["rust_api"],
            "pgorm::pipeline::ExprOps::compares_as",
        )
        self.assertTrue(operations["pipeline.registered_column"]["registration_required"])
        named = pl.alias("named")
        self.assertEqual(
            base.filter(typed == pl.literal("glad")).inspect().sql,
            'SELECT * FROM pipeline_enums.lines WHERE CAST(mood AS pipeline_enums."Mood") = CAST(\'glad\' AS pipeline_enums."Mood")',
        )
        self.assertEqual(
            base.filter(typed == "glad").inspect().sql,
            'SELECT * FROM pipeline_enums.lines WHERE CAST(mood AS pipeline_enums."Mood") = CAST($1::text AS pipeline_enums."Mood")',
        )
        for query, expected in (
            (base.filter(typed == pl.literal("glad")), [2, 4]),
            (base.filter(typed == "glad"), [2, 4]),
            (base.filter_with(lambda binder: typed == binder.bind("glad")), [2, 4]),
            (base.filter(typed.in_array(["calm"])), [1, 3]),
            (base.derive(mood.as_(named)).filter(named.compares_as(enum) < "glad"), [1, 3]),
        ):
            with self.subTest(sql=query.inspect().sql):
                rows = await query.all(self.connection)
                self.assertEqual(sorted(row["id"] for row in rows), expected)
        highest = (
            await base.group()
            .aggregate(pl.max(typed).cast("text").as_("highest"))
            .one(self.connection)
        )
        self.assertEqual(highest["highest"], "glad")
        for query in (
            base.filter(mood == pl.literal("glad")),
            base.filter(mood == "glad"),
            base.filter_with(lambda binder: mood == binder.bind("glad")),
            base.derive(mood.as_(named)).filter(named == "glad"),
        ):
            with self.subTest(sql=query.inspect().sql):
                with self.assertRaises(p.DatabaseError) as refused:
                    await query.all(self.connection)
                self.assertEqual(refused.exception.sqlstate, "42883")

    # [spec:pgorm:req:bindings.pipeline-expressions+3/test]    a money column with no registration
    # compares with a Decimal, bound or listed, as the money compares_as names, where left bare the
    # Decimal cannot be bound to it
    async def test_untyped_money_compares_as_money(self):
        base = pl.from_(p.Table("tills", schema="pipeline_enums"))
        identifier = pl.col("tills", "id")
        takings = pl.col("tills", "takings").compares_as(p.TypeName("money"))

        async def ids(query):
            return sorted(row["id"] for row in await query.select(identifier).all(self.connection))

        self.assertEqual(
            base.filter(takings > Decimal("5")).inspect().sql,
            "SELECT * FROM pipeline_enums.tills WHERE CAST(takings AS money) > CAST($1::numeric AS money)",
        )
        self.assertEqual(await ids(base.filter(takings > Decimal("5"))), [1])
        self.assertEqual(await ids(base.filter_with(lambda binder: takings <= binder.bind(Decimal("3")))), [2])
        self.assertEqual(await ids(base.filter(takings.in_array([Decimal("3"), Decimal("12.35")]))), [1, 2])
        with self.assertRaises(p.ConstructionError):
            await ids(base.filter(pl.col("tills", "takings") > Decimal("5")))

    # [spec:pgorm:req:bindings.pipeline-expressions+3/test]    a built-in range bound in a pipeline
    # and compared with a created range's column, which compares_as names, is written through its text
    async def test_created_range_reads_bound_range_as_text(self):
        span = pl.col("spans", "span").compares_as(p.TypeName("slot", schema="pipeline_enums"))
        query = (pl.from_(p.Table("spans", schema="pipeline_enums"))
                 .filter(span == p.Value(p.Range(1, 5), "int4range"))
                 .select(pl.col("spans", "id")))
        self.assertEqual(
            query.inspect().sql,
            "SELECT id FROM pipeline_enums.spans WHERE CAST(span AS pipeline_enums.slot) "
            "= CAST($1::int4range::text AS pipeline_enums.slot)",
        )
        self.assertEqual([row["id"] for row in await query.all(self.connection)], [1])


if __name__ == "__main__":
    unittest.main()


class PipelineModels(unittest.IsolatedAsyncioTestCase):
    """Declared models in a pipeline: a model as a source, its fields as
    typed columns, select_sources decoding each model's records, and the
    staged reads into_model and into_tuple."""

    flavor = p.TypeName("flavor", schema="pipeline_models")
    bakery = p.Model(
        "bakery",
        {"id": p.Column("i32", primary_key=True), "name": p.Column("text"), "city": p.Column("text")},
        schema="pipeline_models",
    )
    cake = p.Model(
        "cake",
        {
            "id": p.Column("i32", primary_key=True),
            "name": p.Column("text"),
            "flavor": p.Column(flavor, values=["lemon", "carrot", "chocolate"]),
            "price": p.Column("decimal"),
            "baker": p.Column("i32", name="bakery_id", nullable=True),
        },
        schema="pipeline_models",
    )

    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.connection = await self.pool.acquire()
        await self.drop()
        for statement in (
            "CREATE SCHEMA pipeline_models",
            "CREATE TYPE pipeline_models.flavor AS ENUM ('lemon', 'carrot', 'chocolate')",
            "CREATE TABLE pipeline_models.bakery (id integer PRIMARY KEY, name text NOT NULL, city text NOT NULL)",
            "CREATE TABLE pipeline_models.cake (id integer PRIMARY KEY, name text NOT NULL, "
            "flavor pipeline_models.flavor NOT NULL, price numeric(10, 2) NOT NULL, "
            "bakery_id integer REFERENCES pipeline_models.bakery (id))",
            "INSERT INTO pipeline_models.bakery VALUES (1, 'Sunrise', 'Oslo'), (2, 'Moonlight', 'Bergen'), "
            "(3, 'Noon', 'Tromsø')",
            "INSERT INTO pipeline_models.cake VALUES (1, 'Fudge', 'chocolate', 4.50, 1), "
            "(2, 'Drizzle', 'lemon', 3.20, 1), (3, 'Carrot', 'carrot', 3.50, 2), (4, 'Orphan', 'lemon', 2.00, NULL)",
        ):
            await self.connection.execute(p.RawSQL(statement))

    async def drop(self):
        await self.connection.execute(p.RawSQL("DROP SCHEMA IF EXISTS pipeline_models CASCADE"))

    async def asyncTearDown(self):
        await self.drop()
        await self.connection.close()
        await self.pool.close()

    # [spec:pgorm:req:bindings.pipeline-models/test]
    async def test_model_sources_and_field_columns(self):
        cake = self.cake
        cheap = (
            pl.from_(cake)
            .filter(pl.col(cake, "price") < Decimal("4"))
            .sort(pl.col(cake, "id"))
            .select(pl.col(cake, "name"), pl.col(cake, "baker"))
        )
        self.assertEqual(
            cheap.inspect().sql,
            "WITH table_0 AS (SELECT name, bakery_id, id FROM pipeline_models.cake WHERE price < $1) "
            "SELECT name, bakery_id FROM table_0 ORDER BY id",
        )
        self.assertEqual(cheap.inspect().params[0].kind, "decimal")
        rows = await cheap.all(self.connection)
        self.assertEqual(
            [(row["name"], row["bakery_id"]) for row in rows],
            [("Drizzle", 1), ("Carrot", 2), ("Orphan", None)],
        )
        other = cake.as_("other")
        pairs = (
            pl.from_(cake)
            .join(other, pl.col(cake, "baker") == pl.col(other, "baker"))
            .filter(pl.col(cake, "id") < pl.col(other, "id"))
            .select(pl.col(cake, "name").as_("earlier"), pl.col(other, "name").as_("later"))
        )
        self.assertEqual(
            [(row["earlier"], row["later"]) for row in await pairs.all(self.connection)],
            [("Fudge", "Drizzle")],
        )
        self.assertEqual(
            [row["name"] for row in await pl.from_(cake).append(other).filter(pl.col(cake, "id") == 1).all(self.connection)],
            ["Fudge", "Fudge"],
        )
        with self.assertRaises(p.ConstructionError):
            pl.col(cake, "nope")
        with self.assertRaises(TypeError):
            pl.col(cake, p.col("name"))  # type: ignore[call-overload]

    # [spec:pgorm:req:bindings.pipeline-models/test]
    async def test_model_column_converts_compared_values(self):
        cake = self.cake
        lemons = pl.from_(cake).filter(pl.col(cake, "flavor") == "lemon").sort(pl.col(cake, "id")).select(pl.col(cake, "id"))
        self.assertIn("flavor = CAST($1::text AS pipeline_models.flavor)", lemons.inspect().sql)
        self.assertEqual([row["id"] for row in await lemons.all(self.connection)], [2, 4])
        listed = (
            pl.from_(cake)
            .filter(pl.col(cake, "flavor").in_array(["carrot", "chocolate"]))
            .sort(pl.col(cake, "id"))
            .select(pl.col(cake, "id"))
        )
        self.assertEqual([row["id"] for row in await listed.all(self.connection)], [1, 3])
        tagged = pl.from_(cake).filter(pl.col(cake, "flavor") != p.Value("lemon", self.flavor)).select(pl.col(cake, "id"))
        self.assertEqual(len(await tagged.all(self.connection)), 2)
        reflected = pl.from_(cake).filter(Decimal(4) <= pl.col(cake, "price")).select(pl.col(cake, "name"))
        self.assertEqual([row["name"] for row in await reflected.all(self.connection)], ["Fudge"])
        written = pl.from_(cake).filter(pl.col(cake, "flavor") == pl.literal("carrot"))
        self.assertIn("flavor = CAST('carrot' AS pipeline_models.flavor)", written.inspect().sql)
        bound = pl.from_(cake).filter_with(lambda binder: pl.col(cake, "flavor") == binder.bind("carrot"))
        self.assertEqual(len(await bound.all(self.connection)), 1)
        # What an operator makes of the column is a plain expression again.
        with self.assertRaises(p.ConstructionError):
            pl.from_(cake).filter((pl.col(cake, "price") + 1) == p.Value("lemon", self.flavor)).inspect()
        with self.assertRaisesRegex(p.ConstructionError, "not one of the column's values"):
            pl.col(cake, "flavor") == "banana"
        with self.assertRaisesRegex(p.ConstructionError, "never true"):
            pl.col(cake, "baker") == None  # noqa: E711
        with self.assertRaises(p.ConstructionError):
            pl.col(cake, "price") > "cheap"

    # [spec:pgorm:req:bindings.pipeline-models/test]
    async def test_select_sources_decodes_model_records(self):
        cake, bakery = self.cake, self.bakery
        both = (
            pl.from_(cake)
            .join(bakery, pl.col(cake, "baker") == pl.col(bakery, "id"), kind=p.Join.Full)
            .sort(pl.col(bakery, "id"), pl.col(cake, "id"))
            .select_sources(cake, bakery)
        )
        self.assertIn("SELECT cake.id AS s0_id, cake.name AS s0_name, cake.flavor AS s0_flavor", both.inspect().sql)
        rows = await both.all(self.connection)
        self.assertEqual(
            [(None if c is None else c["name"], None if b is None else b["name"]) for c, b in rows],
            [("Fudge", "Sunrise"), ("Drizzle", "Sunrise"), ("Carrot", "Moonlight"), (None, "Noon"), ("Orphan", None)],
        )
        first = rows[0][0]
        assert first is not None
        self.assertIsInstance(first, p.ModelRecord)
        self.assertEqual(dict(first), {"id": 1, "name": "Fudge", "flavor": "chocolate", "price": Decimal("4.50"), "baker": 1})
        only = await pl.from_(cake).sort(pl.col(cake, "id")).select_sources(cake).one(self.connection)
        self.assertEqual(len(only), 1)
        assert only[0] is not None
        self.assertEqual(only[0]["name"], "Fudge")
        self.assertIn("LIMIT 1", pl.from_(cake).select_sources(cake).inspect(terminal="one").sql)
        none = pl.from_(cake).filter(pl.col(cake, "id") < 0).select_sources(cake)
        self.assertIsNone(await none.optional(self.connection))
        with self.assertRaises(p.DecodeError):
            await none.one(self.connection)
        other = cake.as_("other")
        same = (
            pl.from_(cake)
            .join(other, pl.col(cake, "baker") == pl.col(other, "baker"))
            .filter(pl.col(cake, "id") < pl.col(other, "id"))
            .select_sources(cake, other)
        )
        self.assertEqual(
            [(a["name"] if a else None, b["name"] if b else None) for a, b in await same.all(self.connection)],
            [("Fudge", "Drizzle")],
        )
        reshaped = pl.from_(cake).select(pl.col(cake, "id")).select_sources(cake)
        with self.assertRaisesRegex(p.ConstructionError, "select_sources after `select`"):
            reshaped.inspect()
        with self.assertRaisesRegex(p.ConstructionError, "select_sources after `select`"):
            await reshaped.all(self.connection)
        with self.assertRaises(TypeError):
            pl.from_(cake).select_sources(cake, qualifiers=["cake"])  # type: ignore[call-overload]
        with self.assertRaises(TypeError):
            pl.from_(cake).select_sources(p.Table("cake"))  # type: ignore[call-overload]

    # [spec:pgorm:req:bindings.pipeline-models/test]
    async def test_into_model_and_into_tuple(self):
        cake, bakery = self.cake, self.bakery
        lemons = pl.from_(cake).filter(pl.col(cake, "flavor") == "lemon").sort(pl.col(cake, "id"))
        records = await lemons.into_model(cake).all(self.connection)
        self.assertEqual([(r["id"], r["baker"], r["flavor"]) for r in records], [(2, 1, "lemon"), (4, None, "lemon")])
        self.assertEqual((await lemons.into_model(cake).one(self.connection))["name"], "Drizzle")
        with self.assertRaises(p.DecodeError):
            await lemons.select(pl.col(cake, "name")).into_model(cake).all(self.connection)
        named = await lemons.select(pl.col(cake, "name")).into_model(lambda name: f"cake {name}").all(self.connection)
        self.assertEqual(named, ["cake Drizzle", "cake Orphan"])
        with self.assertRaises(TypeError):
            lemons.into_model(3)  # type: ignore[call-overload]
        pairs = (
            pl.from_(cake)
            .join(bakery, pl.col(cake, "baker") == pl.col(bakery, "id"))
            .sort(pl.col(cake, "id"))
            .select(pl.col(cake, "name"), pl.col(bakery, "name"))
            .into_tuple()
        )
        self.assertEqual(
            await pairs.all(self.connection),
            [("Fudge", "Sunrise"), ("Drizzle", "Sunrise"), ("Carrot", "Moonlight")],
        )
        self.assertEqual(await pairs.optional(self.connection), ("Fudge", "Sunrise"))
        self.assertIn("LIMIT 1", pairs.inspect(terminal="one").sql)
        with self.assertRaises(p.ConstructionError):
            bool(pairs)
