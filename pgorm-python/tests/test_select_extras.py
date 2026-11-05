"""A SELECT's tie limit, grouping elements, SIMILAR TO, table functions and
table samples against PostgreSQL, through the installed wheel."""

import os
import unittest
from uuid import uuid4

import pgorm as p
from pgorm import schema as s

G = p.GroupingElement
T = p.TableFunction
SCORES = [(1, 10, "Ann", ["red"]), (2, 9, "Bob", ["red", "blue"]), (3, 9, "Cy", []), (4, 8, "10%", ["x"])]


def quoted(name):
    return '"' + name.replace('"', '""') + '"'


def int4(value):
    return p.bind(value).cast(p.TypeName("int4"))


def text_array(values):
    return p.bind(p.Value.array("text", values)).cast(p.TypeName("text"), array=True)


# [spec:pgorm:req:bindings.select+4/test]
# [spec:pgorm:req:bindings.expressions+3/test]
class SelectExtrasTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.context = self.pool.connection()
        self.connection = await self.context.__aenter__()
        suffix = uuid4().hex[:8]
        self.name = f'Extras "scores" {suffix}'
        await self.connection.execute(p.RawSQL(
            f"CREATE TEMP TABLE {quoted(self.name)} "
            "(id int PRIMARY KEY, score int NOT NULL, name text NOT NULL, tags text[] NOT NULL)"))
        insert = p.Insert(p.Table(self.name)).columns("id", "score", "name", "tags")
        for key, score, name, tags in SCORES:
            insert = insert.values(key, score, name, p.Value.array("text", tags))
        await self.connection.execute(insert)
        self.scores = p.Table(self.name).as_("s")
        self.pairs = f"Extras pairs {suffix}"
        await self.connection.execute(p.RawSQL(
            f"CREATE TEMP TABLE {quoted(self.pairs)} (a int NOT NULL, b int NOT NULL)"))
        await self.connection.execute(
            p.Insert(p.Table(self.pairs)).columns("a", "b").values(1, 2).values(1, 3))
        self.many = f"Extras many {suffix}"
        await self.connection.execute(p.RawSQL(
            f"CREATE TEMP TABLE {quoted(self.many)} AS SELECT g AS id FROM generate_series(1, 200) AS g"))

    async def asyncTearDown(self):
        await self.context.__aexit__(None, None, None)
        await self.pool.close()

    async def rows(self, query, target=None):
        target = target or self.connection
        return [tuple(row.values()) for row in await target.query(query)]

    def s(self, name):
        return self.scores.col(name)

    def ranked(self):
        return p.Select(self.s("id")).from_(self.scores).order_by(self.s("score").desc())

    # [spec:pgorm:req:sql.ast.select.tie-limit+2/test]    the tied rows come back beyond the count
    async def test_tie_limit_keeps_rows_tied_with_last(self):
        ids = lambda rows: sorted(row[0] for row in rows)
        self.assertEqual(ids(await self.rows(self.ranked().limit_with_ties(2))), [1, 2, 3])
        self.assertEqual(len(await self.rows(self.ranked().limit(2))), 2)
        self.assertEqual(ids(await self.rows(self.ranked().limit_with_ties(1))), [1])
        self.assertEqual(ids(await self.rows(self.ranked().limit_with_ties(2).offset(1))), [2, 3])
        self.assertEqual(len(await self.rows(self.ranked().limit_with_ties(2).limit(None))), 4)
        later = p.Select(self.s("id")).from_(self.scores).limit_with_ties(2).order_by(self.s("score").desc())
        self.assertEqual(ids(await self.rows(later)), [1, 2, 3])

    # [spec:pgorm:req:sql.ast.select.tie-limit+2/test]    refused before any SQL is sent: the
    # transaction the refused statement was offered to is still usable
    async def test_unordered_tie_limit_refused_before_sending(self):
        unordered = p.Select(self.s("id")).from_(self.scores).limit_with_ties(2)
        with self.assertRaises(p.ConstructionError):
            unordered.inspect()
        with self.assertRaisesRegex(p.ConstructionError, "ORDER BY"):
            await self.connection.stream(unordered)
        async with self.connection.transaction() as tx:
            for terminal in (tx.query, tx.one, tx.optional, tx.execute):
                with self.subTest(terminal=terminal.__name__):
                    with self.assertRaisesRegex(p.ConstructionError, "ORDER BY"):
                        await terminal(unordered)
            self.assertEqual(len(await self.rows(self.ranked().limit_with_ties(2), tx)), 3)

    # [spec:pgorm:req:sql.ast.select.tie-limit+2/test]    a CTE body or subquery cannot be ordered
    # once read, so a tie limit is checked as it is read
    async def test_embedded_tie_limit_is_checked_when_read(self):
        unordered = p.Select(self.s("id")).from_(self.scores).limit_with_ties(2)
        for embed in (lambda q: p.With("w", q), lambda q: p.With("v", self.ranked()).cte("w", q),
                      p.json_array_query):
            with self.assertRaisesRegex(p.ConstructionError, "ORDER BY"):
                embed(unordered)
        tied = self.ranked().limit_with_ties(2)
        cte = p.Select().from_(p.Table("w")).with_(p.With("w", tied))
        self.assertEqual(sorted(row[0] for row in await self.rows(cte)), [1, 2, 3])
        array = p.Select(p.json_array_query(tied).as_("ids"))
        self.assertEqual(sorted((await self.rows(array))[0][0]), [1, 2, 3])

    # [spec:pgorm:req:sql.ast.select.group-distinct/test]    ROLLUP (a, b), ROLLUP (a) reaches
    # (a, b) twice and (a) three times; DISTINCT groups each set once
    async def test_group_by_distinct_removes_repeated_grouping_sets(self):
        pairs = p.Table(self.pairs)
        a, b = pairs.col("a"), pairs.col("b")
        query = (p.Select(a, b, p.call("count", a).as_("n")).from_(pairs)
                 .group_by(G.rollup(a, b), G.rollup(a))
                 .order_by(a.asc(nulls=p.Nulls.Last), b.asc(nulls=p.Nulls.Last)))
        self.assertEqual(await self.rows(query), [
            (1, 2, 1), (1, 2, 1), (1, 3, 1), (1, 3, 1), (1, None, 2), (1, None, 2), (1, None, 2),
            (None, None, 2),
        ])
        self.assertEqual(await self.rows(query.group_by_distinct()), [
            (1, 2, 1), (1, 3, 1), (1, None, 2), (None, None, 2),
        ])
        sets = (p.Select(b, p.call("count", a)).from_(pairs)
                .group_by(G.sets(b, G.empty())).order_by(b.asc(nulls=p.Nulls.Last)))
        self.assertEqual(await self.rows(sets), [(2, 1), (3, 1), (None, 2)])
        cube = p.Select(p.call("count", a)).from_(pairs).group_by(G.cube(a, b))
        self.assertEqual(len(await self.rows(cube)), 6)
        mixed = p.Select(a, b).from_(pairs).group_by(a, G.set(a, b)).order_by(b.asc())
        self.assertEqual(await self.rows(mixed), [(1, 2), (1, 3)])
        with self.assertRaises(TypeError):
            G()

    # [spec:pgorm:req:sql.ast.expr.similar-to+2/test]    a whole-string match, the default
    # backslash escape and a chosen one
    async def test_similar_to_matches_whole_strings(self):
        def names(predicate):
            return self.rows(p.Select(self.s("name")).from_(self.scores).where(predicate)
                             .order_by(self.s("id").asc()))

        name = self.s("name")
        self.assertEqual(await names(name.similar_to("(A|B)%")), [("Ann",), ("Bob",)])
        self.assertEqual(await names(name.not_similar_to(p.LikeExpr("(A|B)%"))), [("Cy",), ("10%",)])
        self.assertEqual(await names(name.similar_to("A")), [])
        self.assertEqual(await names(name.similar_to("[0-9]+\\%")), [("10%",)])
        self.assertEqual(await names(name.similar_to(p.LikeExpr("%!%").escape("!"))), [("10%",)])
        self.assertEqual(await names(name.not_similar_to(p.LikeExpr("%!%").escape("!"))),
                         [("Ann",), ("Bob",), ("Cy",)])
        self.assertEqual(await names(name.similar_to("' OR 1=1 --")), [])

    # [spec:pgorm:req:sql.ast.expr.similar-to+2/test]    no_escape() turns escaping off for
    # LIKE, ILIKE and SIMILAR TO, a backslash then matching itself
    async def test_no_escape_turns_escaping_off(self):
        def text(value):
            return p.bind(value).cast(p.TypeName("text"))

        off = p.LikeExpr("a\\b").no_escape()
        answers = await self.rows(p.Select(
            text("a\\b").like("a\\b").as_("plain"),
            text("a\\b").like(off).as_("off"),
            text("A\\B").ilike(off).as_("folded"),
            text("a\\b").similar_to(off).as_("similar"),
            text("50%").not_like(p.LikeExpr("50\\%").no_escape()).as_("percent")))
        self.assertEqual(answers, [(False, True, True, True, True)])

    # [spec:pgorm:req:bindings.collations/test]    "C" orders and compares bytewise in both
    # positions, and the server refuses a collation it lacks or a type without one
    async def test_collated_expressions_compare_and_sort(self):
        p.require_capability("expr.collate")
        words = T(p.call("unnest", text_array(["b", "a", "B", "A"]))).columns("word").alias("w")
        word = p.col("word")
        ordered = p.select(word).from_(words).order_by(word.collate("C").asc())
        self.assertIn('ORDER BY ("word" COLLATE "C") ASC', ordered.inspect().sql)
        self.assertEqual(await self.rows(ordered), [("A",), ("B",), ("a",), ("b",)])
        below = p.select(word).from_(words).where(word.collate("POSIX", schema="pg_catalog") < "a")
        self.assertEqual(sorted(await self.rows(below)), [("A",), ("B",)])
        for collated, sqlstate in ((word.collate("no such collation"), "42704"),
                                   (p.literal(1).collate("C"), "42804")):
            with self.subTest(sqlstate=sqlstate), self.assertRaises(p.DatabaseError) as raised:
                await self.rows(p.select(collated).from_(words))
            self.assertEqual(raised.exception.sqlstate, sqlstate)

    # [spec:pgorm:req:sql.ast.select.table-function+2/test]    UNNEST numbered WITH ORDINALITY,
    # its columns named, beside the row it reads
    async def test_unnest_with_ordinality_numbers_positions(self):
        tags = T(p.call("unnest", self.s("tags"))).with_ordinality().columns("tag", "n").alias("u")
        query = (p.Select(self.s("id"), tags.col("tag"), tags.col("n"))
                 .from_(self.scores).from_(tags).order_by(self.s("id").asc(), tags.col("n").asc()))
        self.assertEqual(await self.rows(query), [(1, "red", 1), (2, "red", 1), (2, "blue", 2), (4, "x", 1)])
        bound = T(p.call("unnest", text_array(["x", "y", "z"]))).with_ordinality().columns("v", "n").alias("u")
        self.assertEqual(await self.rows(p.Select(bound.col("v"), bound.col("n")).from_(bound)),
                         [("x", 1), ("y", 2), ("z", 3)])
        plain = T(p.call("unnest", text_array(["a", "b"]))).alias("u")
        self.assertEqual(await self.rows(p.Select(plain.star()).from_(plain)), [("a",), ("b",)])

    # [spec:pgorm:req:sql.ast.select.table-function+2/test]    ROWS FROM reads its calls side by
    # side, padding the shorter with NULL
    async def test_rows_from_pads_shorter_functions_with_null(self):
        zipped = (T.rows_from(p.call("generate_series", int4(1), int4(2)))
                  .and_(p.call("unnest", text_array(["a", "b", "c"])))
                  .with_ordinality().columns("x", "y", "n").alias("t"))
        query = p.Select().from_(zipped).order_by(zipped.col("n").asc())
        self.assertEqual(await self.rows(query), [(1, "a", 1), (2, "b", 2), (None, "c", 3)])
        stepped = T(p.call("generate_series", p.literal(10), p.literal(1), p.literal(-3))).alias("g")
        self.assertEqual(await self.rows(p.Select().from_(stepped)), [(10,), (7,), (4,), (1,)])
        joined = (p.Select(self.s("id"), zipped.col("y")).from_(self.scores)
                  .join(zipped, self.s("id") == zipped.col("n"))
                  .order_by(self.s("id").asc()))
        self.assertEqual(await self.rows(joined), [(1, "a"), (2, "b"), (3, "c")])

    # [spec:pgorm:req:bindings.select+4/test]    a call returning record reads through the
    # columns its definition list gives, a lone call's list in the alias, each call of a ROWS FROM
    # with its own, and the server refuses a missing list or one on another function
    # [spec:pgorm:req:sql.ast.select.column-definitions/test]
    async def test_record_calls_read_through_column_definitions(self):
        p.require_capability("table_function.column_definition")
        one = (T(p.call("json_to_record", p.Value.json({"a": 7, "tags": ["x"]})))
               .column_definition("a", "integer").column_definition("tags", s.DataType("text").array())
               .column_definition("missing", s.DataType("varchar", length=3)).alias("r"))
        query = p.Select(one.star()).from_(one)
        self.assertEqual(query.inspect().sql,
                         'SELECT "r".* FROM JSON_TO_RECORD($1) AS "r"("a" integer, "tags" text[], '
                         '"missing" varchar(3))')
        self.assertEqual(await self.rows(query), [(7, ["x"], None)])
        zipped = (T(p.call("jsonb_to_recordset", p.Value.json([{"a": 2}, {"a": 1}])))
                  .column_definition("a", "integer")
                  .and_(p.call("generate_series", int4(10), int4(12)))
                  .with_ordinality().columns("a", "s", "n").alias("z"))
        self.assertIn("ROWS FROM (JSONB_TO_RECORDSET($1) AS (\"a\" integer), GENERATE_SERIES(",
                      zipped_sql := p.Select(zipped.star()).from_(zipped).inspect().sql)
        self.assertTrue(zipped_sql.endswith('WITH ORDINALITY AS "z"("a", "s", "n")'), zipped_sql)
        self.assertEqual(await self.rows(p.Select(zipped.star()).from_(zipped)),
                         [(2, 10, 1), (1, 11, 2), (None, 12, 3)])
        for item, sqlstate in (
            (T(p.call("json_to_record", p.Value.json({"a": 1}))).alias("r"), "42601"),
            (T(p.call("generate_series", int4(1), int4(2))).column_definition("a", "integer").alias("g"), "42601"),
        ):
            with self.subTest(sqlstate=sqlstate), self.assertRaises(p.DatabaseError) as refused:
                await self.connection.query(p.Select(item.star()).from_(item))
            self.assertEqual(refused.exception.sqlstate, sqlstate)
        with self.assertRaises(p.ConstructionError):
            T(p.call("json_to_record", p.Value.json({}))).column_definition("", "integer")

    # [spec:pgorm:req:sql.ast.select.table-function+2/test]    a bound argument gives the server no
    # overload to choose, so the caller casts it; nothing is guessed for it
    async def test_set_returning_calls_need_typed_arguments(self):
        untyped = T(p.call("generate_series", 1, 2)).alias("g")
        with self.assertRaises(p.DatabaseError) as caught:
            await self.connection.query(p.Select().from_(untyped))
        self.assertEqual(caught.exception.sqlstate, "42725")

    # [spec:pgorm:req:sql.ast.select.table-sample/test]    the percentage decides the sample,
    # a seed repeats it, and an out-of-range percentage is the server's refusal
    async def test_tablesample_draws_by_percentage(self):
        many = p.Table(self.many).as_("m")
        bernoulli = p.SampleMethod.Bernoulli

        async def sample(method, percentage, **seed):
            item = many.tablesample(method, percentage, **seed)
            return [row[0] for row in await self.rows(
                p.Select(item.col("id")).from_(item).order_by(item.col("id").asc()))]

        self.assertEqual(len(await sample(bernoulli, 100)), 200)
        self.assertEqual(len(await sample(p.SampleMethod.System, 100.0)), 200)
        self.assertEqual(await sample(bernoulli, 0), [])
        first = await sample(bernoulli, 50, repeatable=7)
        self.assertEqual(await sample(bernoulli, 50, repeatable=7.0), first)
        self.assertLess(0, len(first))
        self.assertLess(len(first), 200)
        for percentage in (100.5, -1):
            with self.subTest(percentage=percentage):
                with self.assertRaises(p.DatabaseError) as caught:
                    await sample(bernoulli, percentage)
                self.assertEqual(caught.exception.sqlstate, "2202H")
        unaliased = p.Table(self.many).tablesample(bernoulli, 100)
        self.assertEqual(unaliased.alias, self.many)
        count = p.Select(p.call("count", unaliased.col("id"))).from_(unaliased)
        self.assertEqual(await self.rows(count), [(200,)])

    # [spec:pgorm:req:bindings.expressions+3/test]    array(select) collects a subquery's one
    # column into an array value, empty for no rows, cast as one value, and the server refuses a
    # subquery of two columns with 42601
    async def test_array_collects_a_subquery_column(self):
        s = self.scores
        pairs = p.Table(self.pairs).as_("x")
        partners = p.Select(pairs.col("b")).from_(pairs).where(pairs.col("a") == s.col("id"))
        collected = await self.rows(
            p.Select(s.col("id"), p.array(partners).as_("partners"))
            .from_(s)
            .order_by(s.col("id").asc())
        )
        self.assertEqual(
            [(key, sorted(values)) for key, values in collected],
            [(1, [2, 3]), (2, []), (3, []), (4, [])],
        )
        nines = p.Select(s.col("name")).from_(s).where(s.col("score") == 9)
        spelled = p.array(nines).cast(p.TypeName("text"), array=True)
        [(names,)] = await self.rows(p.Select(spelled))
        self.assertEqual(sorted(names), ["Bob", "Cy"])
        self.assertEqual(
            p.capabilities()["operations"]["array"]["rust_api"], "pgorm_query::Expr::array"
        )
        wide = p.Select(s.col("id"), s.col("name")).from_(s)
        with self.assertRaises(p.DatabaseError) as caught:
            await self.rows(p.Select(p.array(wide)))
        self.assertEqual(caught.exception.sqlstate, "42601")


if __name__ == "__main__":
    unittest.main()
