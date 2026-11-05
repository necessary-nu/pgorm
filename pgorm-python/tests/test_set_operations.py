"""A SELECT combined with another by UNION, INTERSECT and EXCEPT, each with
its ALL form, against PostgreSQL through the installed wheel."""

import os
import unittest
from collections import Counter
from uuid import uuid4

import pgorm as p

LEFT = [1, 1, 2, 3, 3, 3]
RIGHT = [1, 3, 3, 4]
THIRD = [2, 3, 3, 5]
EXPECTED = {
    "union": [1, 2, 3, 4],
    "union_all": [1, 1, 1, 2, 3, 3, 3, 3, 3, 4],
    "intersect": [1, 3],
    "intersect_all": [1, 3, 3],
    "except_": [2],
    "except_all": [1, 2, 3],
}


def quoted(name):
    return '"' + name.replace('"', '""') + '"'


def combine(name, left, right):
    """`name` over two bags as PostgreSQL computes it: the plain forms yield
    each qualifying value once, the ALL forms by multiplicity."""
    a, b = Counter(left), Counter(right)
    times = {
        "union": lambda v: int(a[v] + b[v] > 0),
        "union_all": lambda v: a[v] + b[v],
        "intersect": lambda v: int(a[v] > 0 and b[v] > 0),
        "intersect_all": lambda v: min(a[v], b[v]),
        "except_": lambda v: int(a[v] > 0 and b[v] == 0),
        "except_all": lambda v: max(a[v] - b[v], 0),
    }[name]
    return [v for v in sorted(a.keys() | b.keys()) for _ in range(times(v))]


class SetOperationTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.context = self.pool.connection()
        self.connection = await self.context.__aenter__()
        suffix = uuid4().hex[:8]
        self.left = p.Table(f'Sets "left" {suffix}')
        self.right = p.Table(f"Sets right {suffix}")
        self.third = p.Table(f"Sets third {suffix}")
        for table, numbers in ((self.left, LEFT), (self.right, RIGHT), (self.third, THIRD)):
            await self.connection.execute(p.RawSQL(f"CREATE TEMP TABLE {quoted(table.name)} (n int NOT NULL)"))
            insert = p.Insert(table).columns("n")
            for number in numbers:
                insert = insert.values(p.Value(number, "i32"))
            await self.connection.execute(insert)

    async def asyncTearDown(self):
        await self.context.__aexit__(None, None, None)
        await self.pool.close()

    async def numbers(self, query):
        return [row["n"] for row in await self.connection.query(query)]

    def side(self, table):
        return p.Select(p.col("n")).from_(table)

    # [spec:pgorm:req:bindings.select+3/test]    each of the six operators, its ALL form keeping
    # duplicates and the other removing them
    async def test_each_operation_combines_the_rows(self):
        left, right = self.side(self.left), self.side(self.right)
        for name, expected in EXPECTED.items():
            with self.subTest(operation=name):
                p.require_capability("select." + name)
                combined = getattr(left, name)(right).order_by(p.col("n").asc())
                self.assertEqual(await self.numbers(combined), expected)
                with self.assertRaises(p.ConstructionError):
                    bool(combined)
        self.assertEqual(await self.numbers(left), LEFT)

    # [spec:pgorm:req:bindings.select+3/test]    the receiver's ORDER BY and limits bound the
    # combined rows whenever they were set, an arm's own stay inside its parentheses, and bound
    # values number across the arms
    async def test_order_and_limits_cover_combined_rows(self):
        left, right = self.side(self.left), self.side(self.right)
        ascending, descending = p.col("n").asc(), p.col("n").desc()
        self.assertEqual(await self.numbers(left.order_by(descending).limit(2).union(right)), [4, 3])
        arm = right.order_by(descending).limit(1)
        self.assertEqual(await self.numbers(left.union_all(arm).order_by(ascending)), [*LEFT, 4])
        self.assertEqual(await self.numbers(left.union(right).order_by(ascending).limit(2).offset(1)), [2, 3])
        tied = left.union_all(right).order_by(descending).limit_with_ties(2)
        self.assertEqual(await self.numbers(tied), [4, 3, 3, 3, 3, 3])

        filtered = left.where(p.col("n") > 1).union_all(right.where(p.col("n") < 4)).order_by(ascending)
        compiled = filtered.inspect()
        self.assertIn('WHERE "n" > $1 UNION ALL (SELECT "n" FROM', compiled.sql)
        self.assertIn('WHERE "n" < $2) ORDER BY "n" ASC', compiled.sql)
        self.assertEqual([value.value for value in compiled.params], [1, 4])
        self.assertEqual(await self.numbers(filtered), [1, 2, 3, 3, 3, 3, 3])

    # [spec:pgorm:req:bindings.select+3/test]    an ordering of the combined rows names a result
    # column: the server refuses a qualified column of the first arm and any other expression
    async def test_combined_order_names_result_columns(self):
        aliased = self.left.as_("l")
        qualified = p.Select(aliased.col("n")).from_(aliased).union(self.side(self.right))
        for query, sqlstate in (
            (qualified.order_by(aliased.col("n").asc()), "42P01"),
            (self.side(self.left).union(self.side(self.right)).order_by((p.col("n") + 1).asc()), "0A000"),
        ):
            with self.subTest(sqlstate=sqlstate), self.assertRaises(p.DatabaseError) as refused:
                await self.connection.query(query)
            self.assertEqual(refused.exception.sqlstate, sqlstate)
        self.assertEqual(await self.numbers(qualified.order_by(p.col("n").asc())), [1, 2, 3, 4])

    # [spec:pgorm:req:bindings.select+3/test]    the receiver's WITH clause precedes the whole
    # statement, so every arm reads it
    async def test_with_clause_reaches_every_arm(self):
        shared = p.Table("shared")
        above = p.Select(p.col("n")).from_(shared).where(p.col("n") > 2)
        below = p.Select(p.col("n")).from_(shared).where(p.col("n") < 2)
        combined = above.union(below).with_(p.With("shared", self.side(self.left))).order_by(p.col("n").asc())
        self.assertTrue(combined.inspect().sql.startswith('WITH "shared" AS (SELECT "n" FROM'))
        self.assertEqual(await self.numbers(combined), [1, 3])

    # [spec:pgorm:req:bindings.select+3/test]    a chain combines in call order: every pair of
    # operators over three tables reads (left op1 right) op2 third, the bags chosen so that where
    # PostgreSQL's precedence would read the flat chain otherwise, the two readings differ
    # [spec:pgorm:sem:sql.ast.select.union+2/test]
    async def test_chains_combine_in_call_order(self):
        left, right, third = self.side(self.left), self.side(self.right), self.side(self.third)
        intersects = ("intersect", "intersect_all")
        for first in EXPECTED:
            for second in EXPECTED:
                with self.subTest(first=first, second=second):
                    chained = getattr(getattr(left, first)(right), second)(third).order_by(p.col("n").asc())
                    in_call_order = combine(second, combine(first, LEFT, RIGHT), THIRD)
                    self.assertEqual(await self.numbers(chained), in_call_order)
                    if second in intersects and first not in intersects:
                        self.assertNotEqual(combine(first, LEFT, combine(second, RIGHT, THIRD)), in_call_order)

    # [spec:pgorm:req:bindings.select+3/test]    only an INTERSECT after a UNION or EXCEPT
    # parenthesises what came before it, and a combined arm keeps its own grouping
    async def test_intersect_after_union_groups_the_union(self):
        one, two, three = (p.Select(p.literal(number).as_("n")) for number in (1, 2, 3))
        chained = one.union(two).intersect(three)
        self.assertEqual(
            chained.inspect().sql, '(SELECT 1 AS "n" UNION (SELECT 2 AS "n")) INTERSECT (SELECT 3 AS "n")'
        )
        self.assertEqual(await self.numbers(chained), [])
        flat = one.intersect(two).union(three)
        self.assertEqual(flat.inspect().sql, 'SELECT 1 AS "n" INTERSECT (SELECT 2 AS "n") UNION (SELECT 3 AS "n")')
        self.assertEqual(await self.numbers(flat), [3])
        self.assertEqual(await self.numbers(three.intersect(one.union(three))), [3])

if __name__ == "__main__":
    unittest.main()
