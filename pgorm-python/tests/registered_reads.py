"""A registered entity's and graph's reads beyond all, one and optional:
counts, pages, keyset cursors, streams, joins, projections and grouping, in
the downstream application wheel."""

import unittest

import pgorm as p
import registered_entities as fixture

ACCOUNTS = ((1, "Ann", "calm"), (2, "Bob", "busy"), (3, "Cy", "calm"), (4, "Di", "busy"), (5, "Ed", "calm"))


# The fixture's before_save hook appends "|before" to each name it inserts.
class RegisteredReads(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        await fixture.RegisteredEntities.asyncSetUp(self)
        async with self.pool.connection() as connection:
            for identity, name, mood in ACCOUNTS:
                await fixture.RegisteredEntities.active(self, identity, name).set("mood", mood).insert(connection)
            await connection.execute(p.RawSQL(
                "INSERT INTO python_entities.notes VALUES (11, 1, 'first'), (12, 1, 'second'), (21, 2, 'third')"
            ))
            await connection.execute(p.RawSQL(
                "CREATE TABLE python_entities.stays (id integer PRIMARY KEY, room_id integer NOT NULL, "
                "guest_id integer NOT NULL, during daterange NOT NULL)"
            ))
            await connection.execute(p.RawSQL(
                "INSERT INTO python_entities.stays VALUES (1, 1, 2, '[2026-01-01,2026-01-03)'), "
                "(2, 1, 3, '[2026-01-04,2026-01-05)'), (3, 1, 4, '[2026-01-06,2026-01-07)')"
            ))

    async def asyncTearDown(self):
        await fixture.RegisteredEntities.asyncTearDown(self)

    def calm(self):
        account = self.account
        return account.find().where(account.col("mood") == "calm").order_by(account.col("id").asc())

    # [spec:pgorm:req:bindings.entity-reads+4/test]
    async def test_count_and_pages(self):
        query = self.calm()
        self.assertEqual(await query.count(self.pool), 3)
        counted = query.inspect(terminal="count").sql
        self.assertTrue(counted.startswith('SELECT COUNT(*) AS num_items FROM (SELECT "accounts"."id"'), counted)
        self.assertTrue(counted.endswith(') AS "sub_query"'), counted)
        pages = query.paginate(2)
        self.assertEqual(await pages.num_items_and_pages(self.pool), (3, 2))
        self.assertEqual(await pages.num_items(self.pool), 3)
        self.assertEqual(await pages.num_pages(self.pool), 2)
        self.assertEqual([model["id"] for model in await pages.fetch_page(self.pool, 1)], [5])
        self.assertEqual(await pages.fetch_page(self.pool, 2), [])
        page = pages.inspect(1)
        self.assertTrue(page.sql.endswith('ORDER BY "accounts"."id" ASC LIMIT $2 OFFSET $3'), page.sql)
        self.assertEqual([value.value for value in page.params][1:], [2, 2])
        self.assertTrue(pages.inspect("count").sql.startswith('SELECT COUNT(*)'))
        self.assertEqual([[model["id"] for model in page] async for page in pages.pages(self.pool)], [[1, 3], [5]])
        with self.assertRaises(p.ConstructionError):
            query.paginate(0)
        with self.assertRaises(TypeError):
            pages.inspect("first")
        for value in (query.select("id"), pages, self.account.find().cursor("id")):
            with self.assertRaises(p.ConstructionError):
                bool(value)

    # [spec:pgorm:req:bindings.entity-reads+4/test]
    async def test_cursor_orders_by_columns_and_casts_boundaries(self):
        cursor = self.account.find().cursor("mood", "id")
        first = await cursor.first(2).all(self.pool)
        self.assertEqual([model["id"] for model in first], [1, 3])
        following = await cursor.after("calm", 3).first(3).all(self.pool)
        self.assertEqual([model["id"] for model in following], [5, 2, 4])
        compiled = cursor.after("calm", 3).first(3).inspect()
        self.assertTrue(compiled.sql.endswith('ORDER BY "accounts"."mood" ASC, "accounts"."id" ASC LIMIT $4'), compiled.sql)
        self.assertIn('("accounts"."mood" = CAST($1::text AS python_entities."Mood") AND "accounts"."id" > $2) OR', compiled.sql)
        last = await cursor.before("busy", 4).last(2).all(self.pool)
        self.assertEqual([model["id"] for model in last], [5, 2])
        descending = await cursor.desc().first(1).all(self.pool)
        self.assertEqual([model["id"] for model in descending], [4])
        with self.assertRaises(p.ConstructionError):
            cursor.after("calm")
        with self.assertRaises(p.ConstructionError):
            self.account.find().cursor()
        with self.assertRaises(p.ConstructionError):
            self.account.find().cursor("missing")

    # [spec:pgorm:req:bindings.entity-reads+4/test]
    async def test_stream_decodes_models_as_they_arrive(self):
        async with await self.calm().stream(self.pool) as models:
            read = [model async for model in models]
        self.assertEqual([model["id"] for model in read], [1, 3, 5])
        self.assertIsInstance(read[0], p.EntityModel)
        async with self.pool.connection() as connection:
            async with await self.calm().limit(1).stream(connection) as models:
                self.assertEqual([model["display name"] async for model in models], ["Ann|before"])

    # [spec:pgorm:req:bindings.entity-reads+4/test]
    async def test_join_reads_the_root_by_a_relation(self):
        stay = p.entity("app.Stay")
        guest = stay.relation("Guest")
        self.assertEqual((guest.entity_name, guest.name), ("app.Stay", "Guest"))
        busy = (
            stay.find()
            .join(guest)
            .where(self.account.col("mood") == "busy")
            .order_by(stay.col("id").asc())
        )
        self.assertIn(
            'INNER JOIN "python_entities"."accounts" ON "stays"."guest_id" = "accounts"."id"',
            busy.inspect().sql,
        )
        self.assertEqual([row["id"] for row in await busy.all(self.pool)], [1, 3])
        left = stay.find().join(guest, kind=p.Join.Left).inspect().sql
        self.assertIn("LEFT JOIN", left)
        with self.assertRaises(p.ConstructionError):
            stay.relation("Missing")

    # [spec:pgorm:req:bindings.entity-reads+4/test]
    async def test_select_projects_columns_as_records(self):
        names = self.calm().select("display name", "mood")
        self.assertEqual(names.columns, ["display name", "mood"])
        projected = names.inspect().sql
        self.assertTrue(
            projected.startswith('SELECT "accounts"."display name", CAST("accounts"."mood" AS text) FROM'),
            projected,
        )
        rows = await names.all(self.pool)
        self.assertEqual([dict(row) for row in rows][:2], [
            {"display name": "Ann|before", "mood": "calm"},
            {"display name": "Cy|before", "mood": "calm"},
        ])
        self.assertEqual((await names.one(self.pool))["display name"], "Ann|before")
        self.assertTrue(names.inspect(terminal="one").sql.endswith("LIMIT $2"))
        none = self.account.find().where(self.account.col("id") == 99).select("id")
        self.assertIsNone(await none.optional(self.pool))
        async with await names.stream(self.pool) as records:
            self.assertEqual(
                [record["display name"] async for record in records], ["Ann|before", "Cy|before", "Ed|before"]
            )
        with self.assertRaises(p.ConstructionError):
            self.calm().select()
        with self.assertRaises(p.ConstructionError):
            self.calm().select("missing")


class RegisteredGraphReads(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        await RegisteredReads.asyncSetUp(self)
        self.graph = p.graph("app.AccountNotes")

    async def asyncTearDown(self):
        await fixture.RegisteredEntities.asyncTearDown(self)

    def ordered(self):
        query = self.graph.find(aliases=["n"])
        return query.order_by(query.col(0, "id").asc(), query.col(1, "id").asc())

    # [spec:pgorm:req:bindings.entity-graphs+4/test]
    async def test_all_grouped_count_and_pages(self):
        grouped = await self.ordered().all_grouped(self.pool)
        self.assertEqual(
            [(root["id"], [note["id"] for note in notes]) for root, notes in grouped],
            [(1, [11, 12]), (2, [21]), (3, []), (4, []), (5, [])],
        )
        with self.assertRaises(p.ConstructionError):
            p.graph("app.RequiredNotes").find().all_grouped(self.pool)
        self.assertEqual(await self.ordered().count(self.pool), 6)
        pages = self.ordered().paginate(4)
        self.assertEqual(await pages.num_items_and_pages(self.pool), (6, 2))
        second = await pages.fetch_page(self.pool, 1)
        self.assertEqual([(root["id"], None if note is None else note["id"]) for root, note in second], [(4, None), (5, None)])

    # [spec:pgorm:req:bindings.entity-graphs+4/test]
    async def test_stream_decodes_each_row(self):
        async with await self.ordered().stream(self.pool) as rows:
            read = [(root["id"], None if note is None else note["id"]) async for root, note in rows]
        self.assertEqual(read, [(1, 11), (1, 12), (2, 21), (3, None), (4, None), (5, None)])

    # [spec:pgorm:req:bindings.entity-graphs+4/test]
    async def test_cursors_over_slot_columns_and_composite_orders(self):
        on_body = self.graph.find(aliases=["n"]).cursor_on(1, "body")
        compiled = on_body.first(2).inspect()
        self.assertTrue(
            compiled.sql.endswith('ORDER BY "n"."body" ASC, "n"."id" ASC, "accounts"."id" ASC LIMIT $1'),
            compiled.sql,
        )
        rows = await on_body.first(2).all(self.pool)
        self.assertEqual([note["body"] for _, note in rows], ["first", "second"])
        past = await on_body.after_with("first", 11, 1).first(1).all(self.pool)
        self.assertEqual([note["body"] for _, note in past], ["second"])
        composite = self.graph.find(aliases=["n"]).cursor("mood", "id")
        first = await composite.first(3).all(self.pool)
        self.assertEqual([(root["id"], None if note is None else note["id"]) for root, note in first], [(1, 11), (1, 12), (3, None)])
        resumed = await composite.after_with("calm", 1, 11).first(1).all(self.pool)
        self.assertEqual([note["id"] for _, note in resumed], [12])
        with self.assertRaises(p.ConstructionError):
            self.graph.find().cursor_on(2, "body")
        with self.assertRaises(p.ConstructionError):
            composite.after("calm")


if __name__ == "__main__":
    unittest.main()
