"""Raw SQL beyond its terminals: scripts, grammar and PRQL checks, a
transaction's stream, rows decoded into declared models, verification and
pages over a written SELECT."""
import os
import unittest
from decimal import Decimal

import pgorm as p
from pgorm import Column, Model, RawSQL

Item = Model("python_raw_items", {
    "id": Column("i32", primary_key=True),
    "label": Column("text", name="name"),
    "price": Column("decimal"),
})


class RawSQLTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=3)
        await self.pool.batch_execute(
            "DROP TABLE IF EXISTS python_raw_items;"
            "CREATE TABLE python_raw_items (id integer PRIMARY KEY, name text NOT NULL, price numeric NOT NULL);"
            "INSERT INTO python_raw_items VALUES (1, 'one', 1.50), (2, 'two', 2.50), (3, 'three', 3.50)"
        )

    async def asyncTearDown(self):
        await self.pool.execute(RawSQL("DROP TABLE IF EXISTS python_raw_items"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.batch-execute/test]
    async def test_a_script_runs_unprepared_on_each_executor(self):
        rows = await self.pool.one(RawSQL("SELECT count(*) AS n FROM python_raw_items"))
        self.assertEqual(rows["n"], 3)
        async with self.pool.connection() as connection:
            await connection.batch_execute("UPDATE python_raw_items SET price = price + 1; UPDATE python_raw_items SET price = price * 2")
            async with connection.transaction() as tx:
                await tx.batch_execute("DELETE FROM python_raw_items WHERE id = 3; DELETE FROM python_raw_items WHERE id = 2")
                self.assertEqual((await tx.one(RawSQL("SELECT sum(price) AS total FROM python_raw_items")))["total"], Decimal("5.00"))
        with self.assertRaises(p.DatabaseError) as caught:
            await self.pool.batch_execute("SELECT 1; SELEC 2")
        self.assertEqual(caught.exception.sqlstate, "42601")
        with self.assertRaises(TypeError):
            await self.pool.batch_execute(RawSQL("SELECT 1"))  # type: ignore[arg-type]

    # [spec:pgorm:req:bindings.sql-check/test]
    def test_sql_is_held_to_the_grammar(self):
        text = "SELECT name FROM python_raw_items WHERE id = $1"
        self.assertIs(p.sql(text), text)
        self.assertEqual(p.sql("CREATE TABLE a (id int); INSERT INTO a VALUES (1)"), "CREATE TABLE a (id int); INSERT INTO a VALUES (1)")
        with self.assertRaises(p.ConstructionError) as caught:
            p.sql("SELECT id FROM cake\nORDER name")
        self.assertEqual(
            str(caught.exception),
            'PostgreSQL rejected this SQL: syntax error at or near "name"\n\n  ORDER name\n        ^',
        )
        with self.assertRaises(TypeError):
            p.sql(b"SELECT 1")  # type: ignore[arg-type]

    # [spec:pgorm:req:bindings.prql-text/test]
    async def test_prql_compiles_and_binds_its_placeholders(self):
        query = p.prql("from python_raw_items | filter price < $1 | sort name | select {name, price}", [Decimal("3")])
        self.assertIsInstance(query, RawSQL)
        compiled = query.inspect()
        self.assertEqual(compiled.sql, "SELECT name, price FROM python_raw_items WHERE price < $1 ORDER BY name")
        self.assertEqual([row["name"] for row in await self.pool.query(query)], ["one", "two"])
        with self.assertRaises(p.ConstructionError) as missing:
            p.prql("from python_raw_items | filter price < $1")
        self.assertIn("nothing binds $1", str(missing.exception))
        with self.assertRaises(p.ConstructionError) as surplus:
            p.prql("from python_raw_items", [1])
        self.assertIn("no $1 in the query", str(surplus.exception))
        with self.assertRaises(p.ConstructionError) as gap:
            p.prql("from python_raw_items | filter id > $1 | filter id < $3", [1, 2])
        self.assertIn("uses $3 but never $2", str(gap.exception))
        with self.assertRaises(p.ConstructionError) as refused:
            p.prql("from python_raw_items | take $1", [1])
        self.assertIn("PRQL rejected this query", str(refused.exception))

    # [spec:pgorm:req:bindings.streams+1/test]
    async def test_transaction_stream_holds_it_to_last_row(self):
        async with self.pool.transaction() as tx:
            await tx.execute(RawSQL("INSERT INTO python_raw_items VALUES (4, 'four', 4.50)"))
            async with await tx.stream(RawSQL("SELECT id FROM python_raw_items ORDER BY id")) as rows:
                seen = []
                async for row in rows:
                    seen.append(row["id"])
                    if row["id"] == 1:
                        with self.assertRaises(p.LifecycleError):
                            await tx.one(RawSQL("SELECT 1 AS one"))
            self.assertEqual(seen, [1, 2, 3, 4])
            records = [record["label"] async for record in await Item.find().order_by(Item.col("id").asc()).stream(tx)]
            self.assertEqual(records, ["one", "two", "three", "four"])
            self.assertEqual((await tx.one(RawSQL("SELECT count(*) AS n FROM python_raw_items")))["n"], 4)
        self.assertEqual((await self.pool.one(RawSQL("SELECT count(*) AS n FROM python_raw_items")))["n"], 4)

        # Closed with rows left, the stream discards its transaction.
        with self.assertRaises(p.LifecycleError):
            async with self.pool.transaction() as tx:
                await tx.execute(RawSQL("INSERT INTO python_raw_items VALUES (5, 'five', 5.50)"))
                async with await tx.stream(RawSQL("SELECT generate_series(1, 100000) AS n")) as rows:
                    async for row in rows:
                        break
                await tx.one(RawSQL("SELECT 1 AS one"))
        self.assertEqual((await self.pool.one(RawSQL("SELECT count(*) AS n FROM python_raw_items")))["n"], 4)

    # [spec:pgorm:req:bindings.raw-models/test]
    async def test_raw_rows_decode_into_a_declared_model(self):
        query = Item.find_by_statement(RawSQL("SELECT *, 'extra' AS unread FROM python_raw_items WHERE price > $1 ORDER BY id", [Decimal("2")]))
        records = await query.all(self.pool)
        self.assertEqual([(record["id"], record["label"]) for record in records], [(2, "two"), (3, "three")])
        self.assertIsInstance(records[0], p.ModelRecord)
        one = await Item.find_by_statement(RawSQL("SELECT * FROM python_raw_items WHERE id = $1", [1])).one(self.pool)
        self.assertEqual(one["price"], Decimal("1.50"))
        self.assertIsNone(await Item.find_by_statement(RawSQL("SELECT * FROM python_raw_items WHERE id = 9")).optional(self.pool))
        async with self.pool.connection() as connection:
            streamed = [record["id"] async for record in await query.stream(connection)]
        self.assertEqual(streamed, [2, 3])
        with self.assertRaises(p.DecodeError):
            await Item.find_by_statement(RawSQL("SELECT id, name FROM python_raw_items")).all(self.pool)
        with self.assertRaises(p.DecodeError):
            await Item.find_by_statement(RawSQL("SELECT id, name, price::text AS price FROM python_raw_items")).all(self.pool)
        with self.assertRaises(TypeError):
            Item.find_by_statement("SELECT 1")  # type: ignore[arg-type]

    # [spec:pgorm:req:bindings.verify/test]
    async def test_verify_checks_a_model_without_running_it(self):
        statement = RawSQL("SELECT id, name, price FROM python_raw_items WHERE id = $1", [99])
        await self.pool.verify(statement, Item)
        with self.assertRaises(p.DecodeError) as missing:
            await self.pool.verify(RawSQL("SELECT id, name FROM python_raw_items"), Item)
        self.assertEqual(
            str(missing.exception),
            "`python_raw_items` reads column `price`, which the statement does not return; it returns: id, name",
        )
        with self.assertRaises(p.DecodeError) as typed:
            await self.pool.verify(RawSQL("SELECT id, name, price::float8 AS price FROM python_raw_items"), Item)
        self.assertEqual(
            str(typed.exception),
            "`python_raw_items` decodes column `price` as `decimal`, which cannot read PostgreSQL type `float8`",
        )
        async with self.pool.transaction() as tx:
            await tx.verify(statement, Item)
        with self.assertRaises(p.DatabaseError):
            await self.pool.verify(RawSQL("SELECT * FROM no_such_table"), Item)

    # [spec:pgorm:req:bindings.raw-pages/test]
    async def test_raw_statements_page_and_count(self):
        statement = p.from_statement(RawSQL("SELECT name, price FROM python_raw_items WHERE price > $1 ORDER BY price DESC", [Decimal("1")]))
        self.assertEqual(await statement.count(self.pool), 3)
        self.assertEqual(len(await statement.all(self.pool)), 3)
        self.assertIsInstance((await statement.all(self.pool))[0], p.Record)
        pages = statement.paginate(2)
        self.assertEqual(await pages.num_items_and_pages(self.pool), (3, 2))
        second = await pages.fetch_page(self.pool, 1)
        self.assertIsInstance(second[0], p.Record)
        self.assertEqual([row["name"] for row in second], ["one"])
        self.assertTrue(pages.inspect(0).sql.startswith('SELECT * FROM (SELECT name, price FROM python_raw_items'))
        records = Item.find_by_statement(RawSQL("SELECT * FROM python_raw_items ORDER BY id")).paginate(2)
        first = await records.fetch_page(self.pool, 0)
        self.assertEqual([record["label"] for record in first], ["one", "two"])
        self.assertEqual(await Item.find_by_statement(RawSQL("SELECT * FROM python_raw_items")).count(self.pool), 3)
        with self.assertRaises(p.ConstructionError) as refused:
            p.from_statement(RawSQL("DELETE FROM python_raw_items")).paginate(2)
        self.assertIn("pagination needs a SELECT", str(refused.exception))
        with self.assertRaises(p.ConstructionError):
            await p.from_statement(RawSQL("SELECT 1; SELECT 2")).count(self.pool)
        with self.assertRaises(TypeError):
            p.from_statement("SELECT 1")  # type: ignore[arg-type]


if __name__ == "__main__":
    unittest.main()
