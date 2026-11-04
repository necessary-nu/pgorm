"""Installed Python pipeline composition, scope ownership and PostgreSQL results."""

from decimal import Decimal
import os
import unittest

import pgorm as p
from pgorm import pipeline as pl
from pgorm import _native
from pipeline_programs import programs


class PipelineConstruction(unittest.TestCase):
    # [spec:pgorm:req:bindings.pipeline/test]
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

    # [spec:pgorm:req:bindings.pipeline/test]
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

    # [spec:pgorm:req:bindings.pipeline/test]
    # [spec:pgorm:req:python.pipeline/test]
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

    # [spec:pgorm:req:bindings.pipeline/test]
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


if __name__ == "__main__":
    unittest.main()
