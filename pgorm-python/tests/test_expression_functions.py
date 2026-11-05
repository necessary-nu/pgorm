"""Functions by name, SQL templates, GROUPING, full-text search and the
jsonb operators."""
import os
import unittest

import pgorm as p
from pgorm import GroupingElement, RawSQL, Table, TableFunction, call, col, func, select, template

docs = Table("python_function_docs")


class ExpressionFunctions(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        await self.pool.batch_execute(
            "DROP TABLE IF EXISTS python_function_docs;"
            "CREATE TABLE python_function_docs (id integer PRIMARY KEY, kind text NOT NULL, body text NOT NULL, meta jsonb NOT NULL);"
            "INSERT INTO python_function_docs VALUES"
            " (1, 'cake', 'a rich chocolate cake', '{\"tags\": [\"sweet\"], \"size\": {\"w\": 3}}'),"
            " (2, 'cake', 'lemon drizzle', '{\"tags\": [], \"size\": {\"w\": 1}}'),"
            " (3, 'bread', 'sourdough loaf', '{\"size\": {\"w\": 2}}')"
        )

    async def asyncTearDown(self):
        await self.pool.execute(RawSQL("DROP TABLE IF EXISTS python_function_docs"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.named-functions/test]
    async def test_any_function_by_name(self):
        query = select(func("initcap", col("body")).as_("title")).from_(docs).where(col("id") == 2)
        self.assertEqual(query.inspect().sql, 'SELECT initcap("body") AS "title" FROM "python_function_docs" WHERE "id" = $1')
        self.assertEqual((await self.pool.one(query))["title"], "Lemon Drizzle")
        quoted = select(func("Initcap", "x")).inspect().sql
        self.assertEqual(quoted, 'SELECT "Initcap"($1)')
        with self.assertRaises(p.DatabaseError) as missing:
            await self.pool.one(select(func("Initcap", "x").as_("t")))
        self.assertEqual(missing.exception.sqlstate, "42883")
        with self.assertRaises(TypeError):
            func(1)  # type: ignore[arg-type]
        with self.assertRaises(p.ConstructionError):
            func("")
        series = TableFunction(func("jsonb_object_keys", col("meta"))).columns("key").alias("keys")
        keys = select(series.col("key")).from_(docs).join(series, p.literal(True)).where(col("id") == 1).order_by(series.col("key").asc())
        self.assertEqual([row["key"] for row in await self.pool.query(keys)], ["size", "tags"])

    # [spec:pgorm:req:bindings.template/test]
    async def test_a_template_binds_its_interpolations(self):
        limit = 15
        expression = template(t"char_length({col('body')}) < {limit}")
        query = select(col("id")).from_(docs).where(expression).order_by(col("id").asc())
        compiled = query.inspect()
        self.assertEqual(compiled.sql, 'SELECT "id" FROM "python_function_docs" WHERE char_length("body") < $1 ORDER BY "id" ASC')
        self.assertEqual([value.value for value in compiled.params], [15])
        self.assertEqual([row["id"] for row in await self.pool.query(query)], [2, 3])
        self.assertEqual(select(template(t"$$a$$ || {'b'}").as_("ab")).inspect().sql, 'SELECT $$a$$ || $1 AS "ab"')
        with self.assertRaises(TypeError):
            template("char_length(body) < 15")  # type: ignore[arg-type]
        with self.assertRaises(p.ConstructionError):
            template(t"{limit} < $1")
        with self.assertRaises(p.ConstructionError):
            template(t"'{limit}'")
        with self.assertRaises(p.ConstructionError):
            template(t"{limit!r}")

    # [spec:pgorm:req:bindings.expressions+3/test]    GROUPING, and the full-text search functions
    async def test_grouping_and_full_text_search(self):
        query = (
            select(col("kind"), call("grouping", col("kind")).as_("rolled"), call("count", col("id")).as_("n"))
            .from_(docs)
            .group_by(GroupingElement.rollup(col("kind")))
            .order_by(col("kind").asc())
        )
        self.assertIn('GROUPING("kind")', query.inspect().sql)
        rows = await self.pool.query(query)
        self.assertEqual([(row["kind"], row["rolled"], row["n"]) for row in rows], [("bread", 0, 1), ("cake", 0, 2), (None, 1, 3)])
        found = (
            select(col("id"), call("ts_rank", call("to_tsvector", col("body")), call("plainto_tsquery", "chocolate cake")).as_("rank"))
            .from_(docs)
            .where(call("to_tsvector", col("body")).matches(call("websearch_to_tsquery", "chocolate")))
        )
        self.assertEqual([row["id"] for row in await self.pool.query(found)], [1])
        for name in ("to_tsquery", "phraseto_tsquery"):
            self.assertIn(name.upper(), select(call(name, "a")).inspect().sql)
        self.assertIn("TS_RANK_CD", select(call("ts_rank_cd", col("a"), col("b"))).inspect().sql)
        with self.assertRaises(p.UnsupportedCapabilityError):
            call("grouping")
        functions = p.capabilities()["expression_functions"]
        self.assertEqual((functions["grouping"], functions["ts_rank"]), ({"min_args": 1}, [2]))

    # [spec:pgorm:req:bindings.json-operators/test]
    async def test_the_jsonb_operators(self):
        meta = col("meta")
        query = (
            select(
                col("id"),
                meta.get_json_field("size").as_("size"),
                meta.get_json_field("size").cast_json_field("w").as_("width"),
                meta.get_json_path(["size", "w"]).as_("w"),
                meta.cast_json_path(["size", "w"]).as_("w_text"),
            )
            .from_(docs)
            .where(meta.has_json_key("tags"))
            .order_by(col("id").asc())
        )
        self.assertEqual(
            query.inspect().sql,
            'SELECT "id", "meta" -> $1 AS "size", ("meta" -> $2) ->> $3 AS "width", "meta" #> $4 AS "w", '
            '"meta" #>> $5 AS "w_text" FROM "python_function_docs" WHERE "meta" ? $6 ORDER BY "id" ASC',
        )
        rows = await self.pool.query(query)
        self.assertEqual([(row["id"], row["width"], row["w_text"]) for row in rows], [(1, "3", "3"), (2, "1", "1")])
        self.assertEqual(rows[0]["size"], {"w": 3})
        either = select(col("id")).from_(docs).where(meta.has_any_json_keys(["tags", "none"])).order_by(col("id").asc())
        self.assertEqual([row["id"] for row in await self.pool.query(either)], [1, 2])
        both = select(col("id")).from_(docs).where(meta.has_all_json_keys(("tags", "size")))
        self.assertEqual(len(await self.pool.query(both)), 2)
        first = select(meta.get_json_field("tags").get_json_field(p.literal(0)).as_("tag")).from_(docs).where(col("id") == 1)
        self.assertEqual((await self.pool.one(first))["tag"], "sweet")
        with self.assertRaises(TypeError):
            meta.has_json_key(1)  # type: ignore[arg-type]
        with self.assertRaises(TypeError):
            meta.get_json_path("size")  # type: ignore[arg-type]


if __name__ == "__main__":
    unittest.main()
