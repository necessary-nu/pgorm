"""An update's rows before and after it, and an upsert's per-row action, in the application wheel."""

import os
import unittest

import pgorm as p
from pgorm import schema as s


def rows(changes):
    return [(change.old["id"], change.old["note"], change.new["note"]) for change in changes]


# [spec:pgorm:req:bindings.entity-versions+3/test]
class RegisteredVersions(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.account, self.old = p.entity("app.Account"), p.entity("app.Old")
        await self.pool.execute(p.RawSQL("CREATE SCHEMA python_entities"))
        for entity in (self.account, self.old):
            generated = s.from_entity(entity)
            for enum in generated.enums:
                await self.pool.execute(enum)
            await self.pool.execute(generated.table)
        async with self.pool.connection() as connection:
            for identity, note in ((1, "one"), (2, "two"), (3, None)):
                await self.active(identity, f"name {identity}").set("note", note).insert(connection)

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP SCHEMA python_entities CASCADE"))
        await self.pool.close()

    def active(self, identity, name):
        return self.account.active().set("id", identity).set("display name", name).set("note", None)

    async def test_update_by_key_returns_both_versions(self):
        async with self.pool.connection() as connection:
            model = await self.account.find().where(self.account.col("id") == 2).one(connection)
            change = await self.account.update(model.into_active().set("note", "changed")).returning_change(connection)
            self.assertIsInstance(change, p.Change)
            self.assertEqual((change.old["note"], change.new["note"]), ("two", "changed"))
            # A statement terminal, as Rust's is: the ActiveModel's hooks do
            # not run, so neither the name's insert suffix nor the version moves.
            self.assertEqual((change.old["version"], change.new["version"]), (1, 1))
            self.assertEqual(change.new["display name"], "name 2|before")
            unchanged = await self.account.update(change.new.into_active()).returning_change(connection)
            self.assertEqual(dict(unchanged.old), dict(unchanged.new))
            with self.assertRaises(p.DecodeError):
                await self.account.update(
                    self.active(99, "nobody").set("note", "x")).returning_change(connection)

    async def test_update_many_returns_each_change(self):
        async with self.pool.connection() as connection:
            update = (self.account.update_many().set("note", "bulk").set("mood", "busy")
                      .set("version", self.account.col("version").expr() + 10)
                      .where(self.account.col("id") >= 2))
            changes = sorted(await update.returning_changes(connection), key=lambda change: change.old["id"])
            self.assertEqual(rows(changes), [(2, "two", "bulk"), (3, None, "bulk")])
            self.assertEqual([change.new["mood"] for change in changes], ["busy", "busy"])
            self.assertEqual([change.new["version"] for change in changes], [11, 11])
            self.assertEqual(await self.account.update_many().where(
                self.account.col("id") == 99).set("note", "x").returning_changes(connection), [])
            with self.assertRaises(p.ConstructionError):
                await self.account.update_many().where(self.account.col("id") == 1).returning_changes(connection)

    # [spec:pgorm:req:bindings.entity-versions+3/test]
    async def test_update_many_needs_where_or_all_rows(self):
        async with self.pool.connection() as connection:
            unguarded = self.account.update_many().set("note", "every")
            with self.assertRaisesRegex(p.ConstructionError, "where\\(..\\) or all_rows\\(\\)"):
                await unguarded.returning_changes(connection)
            notes = [row["note"] for row in await self.account.find().order_by(
                self.account.col("id").expr().asc()).all(connection)]
            self.assertEqual(notes, ["one", "two", None])
            changes = await unguarded.all_rows().returning_changes(connection)
            self.assertEqual(sorted(change.new["note"] for change in changes), ["every"] * 3)
            narrowed = (self.account.update_many().set("note", "first").all_rows()
                        .where(self.account.col("id") == 1))
            self.assertEqual(rows(await narrowed.returning_changes(connection)), [(1, "every", "first")])

    async def test_upsert_says_what_it_did(self):
        renamed = p.Conflict.on("id").update("display name")
        async with self.pool.connection() as connection:
            fresh = await self.account.insert(self.active(4, "four")).returning_upsert(connection)
            self.assertIsInstance(fresh, p.Upserted.Inserted)
            self.assertEqual(fresh.model["display name"], "four")
            match await self.account.insert(self.active(1, "renamed")).on_conflict(renamed).returning_upsert(connection):
                case p.Upserted.Updated(change):
                    self.assertEqual((change.old["display name"], change.new["display name"]),
                                     ("name 1|before", "renamed"))
                case other:
                    self.fail(f"expected an update, got {other!r}")
            skipped = p.Conflict.on("id").do_nothing()
            self.assertIsNone(
                await self.account.insert(self.active(2, "skipped")).on_conflict(skipped).returning_upsert(connection))
            guarded = renamed.where(p.literal(False))
            self.assertIsNone(
                await self.account.insert(self.active(2, "guarded")).on_conflict(guarded).returning_upsert(connection))
            batch = self.account.insert_many([self.active(3, "three again"), self.active(5, "five")])
            upserted = await batch.on_conflict(renamed).returning_upserts(connection)
            self.assertEqual([isinstance(row, p.Upserted.Updated) for row in upserted], [True, False])
            self.assertEqual([row.into_model()["display name"] for row in upserted], ["three again", "five"])
            self.assertEqual(upserted[0].change.old["display name"], "name 3|before")
            self.assertEqual(await self.account.insert_many([]).returning_upserts(connection), [])
            with self.assertRaises(p.LifecycleError):
                self.account.insert(self.old.active().set("id", 1).set("label", "x"))

    # [spec:pgorm:req:bindings.entity-versions+3/test]
    def test_insert_is_typed_by_row_count(self):
        one = self.account.insert(self.active(9, "nine"))
        many = self.account.insert_many([self.active(9, "nine")])
        self.assertIs(type(one), p.EntityInsert)
        self.assertIs(type(many), p.EntityInsertMany)
        self.assertIs(type(one.on_conflict(p.Conflict.do_nothing())), p.EntityInsert)
        self.assertIs(type(many.on_conflict(p.Conflict.do_nothing())), p.EntityInsertMany)
        self.assertFalse(hasattr(one, "returning_upserts"))
        self.assertFalse(hasattr(many, "returning_upsert"))

    async def test_table_called_old_keeps_version_order(self):
        async with self.pool.connection() as connection:
            row = await self.old.insert(self.old.active().set("id", 1).set("label", "before")).returning_upsert(connection)
            change = await self.old.update(row.model.into_active().set("label", "after")).returning_change(connection)
            self.assertEqual((change.old["label"], change.new["label"]), ("before", "after"))
            again = self.old.active().set("id", 1).set("label", "again")
            upsert = await self.old.insert(again).on_conflict(
                p.Conflict.on("id").update("label")).returning_upsert(connection)
            self.assertEqual((upsert.change.old["label"], upsert.change.new["label"]), ("after", "again"))


if __name__ == "__main__":
    unittest.main()
