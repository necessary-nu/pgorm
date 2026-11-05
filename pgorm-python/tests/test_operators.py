"""Expr's binary operators beyond comparison and arithmetic, its subscripts,
subqueries and quantifiers against PostgreSQL, through the installed wheel:
each selects the rows the server's own operator, written as SQL text,
selects."""

import os
import unittest
from uuid import uuid4

import pgorm as p


def quoted(name):
    return '"' + name.replace('"', '""') + '"'


def int4range(lower, upper):
    return p.Value(p.Range(lower, upper), "int4range")


def int4multirange(*ranges):
    return p.Value(p.Multirange([p.Range(lower, upper) for lower, upper in ranges]), "int4multirange")


# [spec:pgorm:req:bindings.expressions+3/test]    every operator method selects what PostgreSQL's
# [spec:pgorm:req:bindings.ranges/test]
# own operator selects, on rows where it holds and rows where it does not
class OperatorTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.context = self.pool.connection()
        self.connection = await self.context.__aenter__()
        self.name = f"operands {uuid4().hex[:8]}"
        await self.connection.execute(p.RawSQL(
            f"CREATE TEMP TABLE {quoted(self.name)} (id int PRIMARY KEY, label text NOT NULL, "
            "note text, doc jsonb NOT NULL, span int4range NOT NULL, "
            "spans int4multirange NOT NULL, tags text[] NOT NULL)"))
        await self.connection.execute(p.RawSQL(
            f"INSERT INTO {quoted(self.name)} VALUES "
            "(1, 'pgorm', 'a', '{\"a\": 1}', '[1,5)', '{[1,3),[7,9)}', '{a,b}'), "
            "(2, 'PGORM', NULL, '{\"b\": 2}', '[5,8)', '{[5,8)}', '{c}'), "
            "(3, 'sql', 'b', '{\"a\": \"x\"}', '[10,12)', '{[20,30)}', '{}')"))
        self.table = p.Table(self.name)

    async def asyncTearDown(self):
        await self.context.__aexit__(None, None, None)
        await self.pool.close()

    def c(self, name):
        return self.table.col(name)

    async def built(self, predicate):
        query = (p.Select(self.c("id")).from_(self.table).where(predicate)
                 .order_by(self.c("id").asc()))
        return [row["id"] for row in await self.connection.query(query)]

    async def written(self, predicate):
        rows = await self.connection.query(p.RawSQL(
            f"SELECT id FROM {quoted(self.name)} WHERE {predicate} ORDER BY id"))
        return [row["id"] for row in rows]

    async def test_each_operator_answers_as_postgres_does(self):
        jsonpath = p.bind("$.a == 1").cast(p.TypeName("jsonpath"))
        cases = [
            (self.c("note").is_distinct_from("a"), "note IS DISTINCT FROM 'a'", [2, 3]),
            (self.c("note").is_not_distinct_from(p.Value.null("text")),
             "note IS NOT DISTINCT FROM NULL", [2]),
            (self.c("label").concat("!").eq("sql!"), "label || '!' = 'sql!'", [3]),
            (self.c("id").between(3, 2, symmetric=True), "id BETWEEN SYMMETRIC 3 AND 2", [2, 3]),
            (self.c("id").not_between(3, 2, symmetric=True), "id NOT BETWEEN SYMMETRIC 3 AND 2", [1]),
            (self.c("label").regex("^pg"), "label ~ '^pg'", [1]),
            (self.c("label").not_regex("^pg"), "label !~ '^pg'", [2, 3]),
            (self.c("label").iregex("^pg"), "label ~* '^pg'", [1, 2]),
            (self.c("label").not_iregex("^pg"), "label !~* '^pg'", [3]),
            (self.c("span").contains(p.literal(3)), "span @> 3", [1]),
            (self.c("span").contained_by(int4range(0, 9)), "span <@ '[0,9)'::int4range", [1, 2]),
            (self.c("spans").overlaps(int4multirange((2, 6))),
             "spans && '{[2,6)}'::int4multirange", [1, 2]),
            (self.c("tags").contains(p.Value.array("text", ["a"])), "tags @> '{a}'", [1]),
            (self.c("tags").contained_by(p.Value.array("text", ["a", "b", "c"])),
             "tags <@ '{a,b,c}'", [1, 2, 3]),
            (self.c("tags").overlaps(p.Value.array("text", ["a", "c"])), "tags && '{a,c}'", [1, 2]),
            (self.c("span").strictly_left_of(int4range(9, 20)), "span << '[9,20)'::int4range", [1, 2]),
            (self.c("span").strictly_right_of(int4range(1, 5)), "span >> '[1,5)'::int4range", [2, 3]),
            (self.c("span").not_extend_right_of(int4range(1, 6)), "span &< '[1,6)'::int4range", [1]),
            (self.c("span").not_extend_left_of(int4range(5, 6)), "span &> '[5,6)'::int4range", [2, 3]),
            (self.c("span").adjacent_to(int4range(8, 9)), "span -|- '[8,9)'::int4range", [2]),
            (self.c("spans").not_extend_right_of(int4multirange((1, 10))),
             "spans &< '{[1,10)}'::int4multirange", [1, 2]),
            (self.c("spans").adjacent_to(int4multirange((9, 20))),
             "spans -|- '{[9,20)}'::int4multirange", [1, 3]),
            (self.c("label").matches("pgorm"), "label @@ 'pgorm'", [1, 2]),
            (self.c("doc").matches(jsonpath), "doc @@ '$.a == 1'", [1]),
            (self.c("doc").path_exists("$.a"), "doc @? '$.a'", [1, 3]),
        ]
        for predicate, written, expected in cases:
            with self.subTest(written=written):
                ids = await self.built(predicate)
                self.assertEqual(ids, await self.written(written))
                self.assertEqual(ids, expected)

    # [spec:pgorm:req:bindings.expressions+3/test]    at and slice subscript an array as
    # PostgreSQL's own subscripts do, an index past the bounds NULL and a slice cut to them
    async def test_subscripts_answer_as_postgres_does(self):
        cases = [
            (self.c("tags").at(1) == "a", "tags[1] = 'a'", [1]),
            (self.c("tags").at(self.c("id")).is_null(), "tags[id] IS NULL", [2, 3]),
            (self.c("tags").slice(2) == p.Value.array("text", ["b"]), "tags[2:] = '{b}'", [1]),
            (self.c("tags").slice(upper=1) == p.Value.array("text", ["c"]), "tags[:1] = '{c}'", [2]),
            (self.c("tags").slice(1, 5) == self.c("tags").slice(), "tags[1:5] = tags[:]", [1, 2, 3]),
        ]
        for predicate, written, expected in cases:
            with self.subTest(written=written):
                ids = await self.built(predicate)
                self.assertEqual(ids, await self.written(written))
                self.assertEqual(ids, expected)

    # [spec:pgorm:req:bindings.quantifiers+3/test]    EXISTS, a scalar subquery, IN a Select, and
    # any_ and all_ after each comparison over an array or a subquery's rows answer as the SQL
    # written by hand, an empty array false under ANY and true under ALL
    async def test_subqueries_and_quantifiers_answer_as_postgres_does(self):
        inner = p.Table(self.name).as_("i")
        noted = p.Select(inner.col("id")).from_(inner).where(inner.col("note").is_not_null())
        later = p.Select(inner.col("id")).from_(inner).where(inner.col("id") > self.c("id"))
        name = quoted(self.name)
        noted_sql = f"SELECT i.id FROM {name} i WHERE i.note IS NOT NULL"
        ints = lambda *items: p.Value.array("i64", list(items))
        cases = [
            (p.exists(later), f"EXISTS (SELECT i.id FROM {name} i WHERE i.id > {name}.id)", [1, 2]),
            (self.c("id") == p.scalar(p.Select(p.call("max", inner.col("id"))).from_(inner)),
             f"id = (SELECT max(i.id) FROM {name} i)", [3]),
            (self.c("id").is_in(noted), f"id IN ({noted_sql})", [1, 3]),
            (self.c("id").is_not_in(noted), f"id NOT IN ({noted_sql})", [2]),
            (self.c("id") == p.any_(ints(1, 3)), "id = ANY('{1,3}'::int8[])", [1, 3]),
            (self.c("id") != p.all_(ints(1, 3)), "id <> ALL('{1,3}'::int8[])", [2]),
            (self.c("id") == p.any_(ints()), "id = ANY('{}'::int8[])", []),
            (self.c("id") != p.all_(ints()), "id <> ALL('{}'::int8[])", [1, 2, 3]),
            (p.bind("a") == p.any_(self.c("tags")), "'a' = ANY(tags)", [1]),
            (self.c("id") >= p.all_(noted), f"id >= ALL ({noted_sql})", [3]),
            (self.c("id").lt(p.any_(noted)), f"id < ANY ({noted_sql})", [1, 2]),
            (self.c("id").ne(p.all_(p.array(noted))), f"id <> ALL (ARRAY({noted_sql}))", [2]),
        ]
        for predicate, written, expected in cases:
            with self.subTest(written=written):
                ids = await self.built(predicate)
                self.assertEqual(ids, await self.written(written))
                self.assertEqual(ids, expected)
        quantified = p.any_(self.c("tags"))
        for refused in [lambda: self.c("id") + quantified, lambda: self.c("label").concat(quantified),
                        lambda: self.c("label").similar_to(quantified),
                        lambda: self.c("id").is_distinct_from(quantified),
                        lambda: self.c("id").between(quantified, 3),
                        lambda: self.c("id").is_in([quantified]), lambda: bool(quantified)]:
            with self.assertRaises(p.ConstructionError):
                refused()

    # [spec:pgorm:req:bindings.quantifiers+3/test]    any_ and all_ after LIKE's and ILIKE's
    # family, the regular-expression matches, containment, the range positions, @@ and @?, over a
    # bound array or a subquery's rows, answer as the SQL written by hand
    async def test_quantifiers_beyond_comparisons_answer_as_postgres_does(self):
        texts = lambda *items: p.Value.array("text", list(items))
        windows = p.Select(p.literal("[0,6)").cast(p.TypeName("int4range"))).union(
            p.Select(p.literal("[8,20)").cast(p.TypeName("int4range"))))
        windows_sql = "SELECT '[0,6)'::int4range UNION SELECT '[8,20)'::int4range"
        paths = p.bind(["$.a", "$.c"]).cast(p.TypeName("jsonpath"), array=True)
        label, span, doc = self.c("label"), self.c("span"), self.c("doc")
        cases = [
            (label.like(p.any_(texts("pg%", "s%"))), "label LIKE ANY('{pg%,s%}')", [1, 3]),
            (label.not_like(p.all_(texts("pg%", "s%"))), "label NOT LIKE ALL('{pg%,s%}')", [2]),
            (label.ilike(p.any_(texts("pg%"))), "label ILIKE ANY('{pg%}')", [1, 2]),
            (label.not_ilike(p.all_(texts("pg%", "o%"))), "label NOT ILIKE ALL('{pg%,o%}')", [3]),
            (label.like(p.any_(p.Select(p.literal("s%")))), "label LIKE ANY(SELECT 's%')", [3]),
            (label.regex(p.any_(texts("^p", "^s"))), "label ~ ANY('{^p,^s}')", [1, 3]),
            (label.not_regex(p.all_(texts("^p", "^s"))), "label !~ ALL('{^p,^s}')", [2]),
            (label.iregex(p.any_(texts("^PG", "^O"))), "label ~* ANY('{^PG,^O}')", [1, 2]),
            (label.not_iregex(p.all_(texts("^pg"))), "label !~* ALL('{^pg}')", [3]),
            (span.contains(p.any_(p.bind(p.Value.array("i32", [3, 11])).cast(p.TypeName("int4"), array=True))), "span @> ANY('{3,11}'::int4[])", [1, 3]),
            (span.contained_by(p.any_(windows)), f"span <@ ANY({windows_sql})", [1, 3]),
            (span.overlaps(p.all_(windows)), f"span && ALL({windows_sql})", []),
            (span.strictly_left_of(p.any_(windows)), f"span << ANY({windows_sql})", [1, 2]),
            (span.strictly_right_of(p.any_(windows)), f"span >> ANY({windows_sql})", [3]),
            (span.not_extend_right_of(p.all_(windows)), f"span &< ALL({windows_sql})", [1]),
            (span.not_extend_left_of(p.all_(windows)), f"span &> ALL({windows_sql})", [3]),
            (span.adjacent_to(p.any_(windows)), f"span -|- ANY({windows_sql})", [2]),
            (label.matches(p.any_(texts("pgorm", "sql"))), "label @@ ANY('{pgorm,sql}'::text[])", [1, 2, 3]),
            (doc.path_exists(p.any_(paths)), "doc @? ANY('{$.a,$.c}'::jsonpath[])", [1, 3]),
            (doc.matches(p.all_(p.bind(["$.a == 1"]).cast(p.TypeName("jsonpath"), array=True))),
             "doc @@ ALL('{\"$.a == 1\"}'::jsonpath[])", [1]),
        ]
        for predicate, written, expected in cases:
            with self.subTest(written=written):
                ids = await self.built(predicate)
                self.assertEqual(ids, await self.written(written))
                self.assertEqual(ids, expected)
        with self.assertRaises(p.ConstructionError):
            await self.built(doc.path_exists(p.any_(texts("$.a"))))

    async def test_bare_operators_group_as_built(self):
        predicate = (self.c("label").not_regex("^s")
                     & (self.c("span").adjacent_to(int4range(8, 9)) | self.c("doc").path_exists("$.a"))
                     & ~self.c("span").not_extend_left_of(int4range(9, 10)))
        sql = predicate.inspect().sql
        self.assertIn('!~ $1 AND (', sql)
        self.assertIn('-|- $2 OR ', sql)
        self.assertIn('@? CAST($3::text AS jsonpath))', sql)
        ids = await self.built(predicate)
        self.assertEqual(ids, await self.written(
            "(label !~ '^s') AND ((span -|- '[8,9)'::int4range) OR (doc @? '$.a')) "
            "AND NOT (span &> '[9,10)'::int4range)"))
        self.assertEqual(ids, [1, 2])

    def test_a_json_path_is_a_str(self):
        for path in [1, None, p.col("x"), b"$.a"]:
            with self.subTest(path=path):
                with self.assertRaises(TypeError):
                    self.c("doc").path_exists(path)
        self.assertEqual(self.c("doc").path_exists("$.a").inspect().params, [p.Value("$.a")])


if __name__ == "__main__":
    unittest.main()
