"""Actual Rust select_sources tuple decoding through an application wheel."""

import asyncio
import unittest
import pgorm as p
from pgorm import pipeline as pl
import registered_entities as fixture


class SourceTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        await fixture.RegisteredEntities.asyncSetUp(self)
        async with self.pool.connection() as connection:
            for identity in (1, 2, 3):
                await fixture.RegisteredEntities.active(self, identity).insert(
                    connection
                )
            await connection.execute(
                p.RawSQL(
                    "INSERT INTO python_entities.notes VALUES (11,1,'first'),(12,1,'second'),(21,2,'third'),(99,99,'orphan')"
                )
            )

    async def asyncTearDown(self):
        await fixture.RegisteredEntities.asyncTearDown(self)

    def joined(self, kind=p.Join.Left, alias="n"):
        return pl.from_(self.account).join(
            pl.source(self.note).named(alias),
            pl.col("accounts", "id") == pl.col(alias, "account_id"),
            kind=kind,
        )

    # [spec:pgorm:req:bindings.pipeline-sources+3/test]
    async def test_all_source_arities_preserve_optional_models(self):
        async with self.pool.connection() as connection:
            one = pl.from_(self.account).select_sources(pl.sources("app.SingleAccount"))
            self.assertIsInstance((await one.one(connection))[0], p.EntityModel)
            for size, word in (
                (2, "Two"),
                (3, "Three"),
                (4, "Four"),
                (5, "Five"),
                (6, "Six"),
            ):
                query = pl.from_(self.account).filter(pl.col("accounts", "id") == 3)
                aliases = ["accounts"]
                for index in range(1, size):
                    alias = f"n{index}"
                    aliases.append(alias)
                    query = query.join(
                        pl.source(self.note).named(alias),
                        pl.col("accounts", "id") == pl.col(alias, "account_id"),
                        kind=p.Join.Left,
                    )
                selected = query.select_sources(
                    pl.sources(f"app.{word}Sources"), qualifiers=aliases
                )
                row = await selected.one(connection)
                self.assertEqual(len(row), size)
                self.assertEqual(row[0]["id"], 3)
                self.assertEqual(row[1:], (None,) * (size - 1))

    # [spec:pgorm:req:bindings.pipeline-sources+3/test]
    async def test_right_full_and_absent_single_sources(self):
        selection = pl.sources("app.TwoSources")
        async with self.pool.connection() as connection:
            for kind, expected in (
                (p.Join.Inner, {(1, 11), (1, 12), (2, 21)}),
                (p.Join.Left, {(1, 11), (1, 12), (2, 21), (3, None)}),
                (p.Join.Right, {(1, 11), (1, 12), (2, 21), (None, 99)}),
                (p.Join.Full, {(1, 11), (1, 12), (2, 21), (3, None), (None, 99)}),
            ):
                rows = (
                    await self.joined(kind)
                    .select_sources(selection, qualifiers=["accounts", "n"])
                    .all(connection)
                )
                actual = {
                    (None if a is None else a["id"], None if n is None else n["id"])
                    for a, n in rows
                }
                self.assertEqual(actual, expected)
            absent = self.joined(p.Join.Right).filter(pl.col("n", "id") == 99)
            single = absent.select_sources(pl.sources("app.SingleAccount"))
            self.assertEqual(await single.optional(connection), (None,))
            empty = absent.filter(False).select_sources(pl.sources("app.SingleAccount"))
            self.assertIsNone(await empty.optional(connection))
            self.assertEqual(await empty.all(connection), [])
            with self.assertRaises(p.DecodeError):
                await empty.one(connection)

    # [spec:pgorm:req:bindings.pipeline-sources+3/test]
    async def test_aliases_parameters_and_decode_failures(self):
        alias = "notes '雪'; -- $1"
        refused = 'notes "雪"'
        with self.assertRaisesRegex(
            p.ConstructionError, "cannot be written as a pipeline name"
        ):
            self.joined(alias=refused).select_sources(
                pl.sources("app.TwoSources"), qualifiers=["accounts", refused]
            ).inspect()
        query = self.joined(alias=alias).filter_with(
            lambda b: pl.col(alias, "body") == b.bind("second")
        )
        selected = query.select_sources(
            pl.sources("app.TwoSources"), qualifiers=["accounts", alias]
        )
        self.assertEqual(selected.inspect().params[0].value, "second")
        async with self.pool.connection() as connection:
            row = await selected.one(connection)
            self.assertEqual((row[0]["id"], row[1]["id"]), (1, 12))
            self.assertEqual(row[1].entity_name, "app.Note")
            self.assertEqual(row[0]["mood"], "calm")
            self.assertEqual(
                row[0].into_active().get("id").state, p.ActiveState.Unchanged
            )
            await connection.execute(
                p.RawSQL(
                    "ALTER TABLE python_entities.notes ALTER COLUMN body DROP NOT NULL"
                )
            )
            await connection.execute(
                p.RawSQL("UPDATE python_entities.notes SET body=NULL WHERE id=11")
            )
            broken = (
                self.joined()
                .filter(pl.col("n", "id") == 11)
                .select_sources(
                    pl.sources("app.TwoSources"), qualifiers=["accounts", "n"]
                )
            )
            for terminal in (broken.all, broken.one, broken.optional):
                with self.assertRaises(p.DecodeError):
                    await terminal(connection)

    # [spec:pgorm:req:bindings.pipeline-sources+3/test]
    async def test_registration_and_shape_rejections(self):
        entries = {x["name"]: x for x in p.capabilities()["registrations"]["sources"]}
        self.assertEqual(len(entries), 6)
        self.assertEqual(pl.source_tuples(), sorted(entries))
        selection = pl.sources("app.TwoSources")
        self.assertEqual(selection.describe(), entries[selection.name])
        self.assertEqual(selection.describe()["entities"], ["app.Account", "app.Note"])
        base = self.joined()
        for qualifiers in (
            [],
            ["a"],
            ["a", "n", "x"],
            ["a", ""],
            ["a", "bad\0name"],
        ):
            with self.assertRaises(p.ConstructionError):
                base.select_sources(selection, qualifiers=qualifiers)
        for qualifiers in ("an", 1):
            with self.assertRaises(TypeError):
                base.select_sources(selection, qualifiers=qualifiers)
        for changed in (
            base.select(pl.col("accounts", "id")),
            base.group(pl.col("accounts", "id")).aggregate(pl.count_rows().as_("n")),
        ):
            with self.assertRaisesRegex(p.ConstructionError, "reshap"):
                changed.select_sources(selection).inspect()

    # [spec:pgorm:req:python.truth-testing+5/test]
    async def test_selected_sources_refuse_truth_testing(self):
        selected = self.joined().select_sources(
            pl.sources("app.TwoSources"), qualifiers=["accounts", "n"]
        )
        with self.assertRaises(p.ConstructionError):
            bool(selected)

    # [spec:pgorm:req:bindings.pipeline-sources+3/test]
    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_source_query_cancellation_releases_connection(self):
        await self.pool.execute(
            p.RawSQL(
                "CREATE VIEW python_entities.slow_accounts AS SELECT a.* FROM python_entities.accounts a CROSS JOIN pg_sleep(2)"
            )
        )
        query = pl.from_(
            pl.source(p.Table("slow_accounts", schema="python_entities")).named(
                "accounts"
            )
        ).select_sources(pl.sources("app.SingleAccount"))
        connection = await self.pool.acquire()
        pending = asyncio.ensure_future(query.all(connection))
        await asyncio.sleep(0.05)
        pending.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await pending
        await connection.close()
        self.assertTrue(await asyncio.wait_for(self.pool.ping(), 5))


if __name__ == "__main__":
    unittest.main()
