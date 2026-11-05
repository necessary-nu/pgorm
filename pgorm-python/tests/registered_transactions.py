"""Registered Rust models, hooks, graphs and pipelines inside native transactions."""
import asyncio
import os
import unittest

import pgorm as p
from pgorm import pipeline as pl, schema as s


class RolledBack(Exception):
    """Raised to leave a block, so that its savepoint rolls back."""


class RegisteredTransactions(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        self.account = p.entity("app.Account")
        self.note = p.entity("app.Note")
        await self.pool.execute(p.RawSQL("CREATE SCHEMA python_entities"))
        for entity in (self.account, self.note):
            generated = s.from_entity(entity)
            for enum in generated.enums:
                await self.pool.execute(enum)
            await self.pool.execute(generated.table)

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP SCHEMA python_entities CASCADE"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.transactions+1/test]
    async def test_models_graphs_cursors_and_sources_share_transaction(self):
        async with self.pool.transaction() as tx:
            active = self.account.active().set("id", 1).set("display name", "Native").set("note", None)
            model = await active.insert(tx)
            self.assertEqual(model["display name"], "Native|before")
            self.assertEqual((await self.account.find().one(tx))["id"], 1)
            note = self.note.active().set("id", 11).set("account_id", 1).set("body", "first")
            await note.insert(tx)
            query = p.graph("app.AccountNotes").find()
            joined = await query.all(tx)
            self.assertEqual([(a["id"], n["id"]) for a, n in joined], [(1, 11)])
            cursor = await query.cursor("id").first(1).all(tx)
            self.assertEqual(cursor[0][0]["id"], 1)
            pipeline = pl.from_(self.account)
            self.assertEqual((await pipeline.one(tx))["id"], 1)
            selected = pipeline.select_sources(pl.sources("app.SingleAccount"))
            self.assertEqual((await selected.one(tx))[0]["id"], 1)
            async with self.pool.connection() as outside:
                self.assertEqual(await self.account.find().all(outside), [])
            with self.assertRaises(RolledBack):
                async with tx.transaction() as child:
                    changed = await model.into_active().set("display name", "changed").update(child)
                    self.assertEqual(changed["version"], 2)
                    raise RolledBack
            self.assertEqual((await self.account.find().one(tx))["version"], 1)
            await model.into_active().delete(tx)
            self.assertEqual(await self.account.find().all(tx), [])

    # [spec:pgorm:req:bindings.transactions+1/test]
    # [spec:pgorm:req:bindings.entity-writes+1/test]    the hook's refusal, a ConstructionError,
    # rolls back the write it refused inside the transaction
    async def test_after_hook_exception_rolls_back_database_write(self):
        with self.assertRaisesRegex(p.ConstructionError, "after_save refused"):
            async with self.pool.transaction() as tx:
                active = self.account.active().set("id", 1).set("display name", "reject_after").set("note", None)
                await active.insert(tx)
        async with self.pool.connection() as connection:
            self.assertEqual(await self.account.find().all(connection), [])

    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_cancelled_hook_ends_native_transaction_owner(self):
        async with self.pool.connection() as connection:
            with self.assertRaises(asyncio.CancelledError):
                async with connection.transaction() as tx:
                    active = self.account.active().set("id", 1).set("display name", "wait_before").set("note", None)
                    task = asyncio.ensure_future(active.insert(tx))
                    await asyncio.sleep(0.05)
                    task.cancel()
                    await task
            await asyncio.wait_for(connection.close(), 2)
            self.assertTrue(tx.closed)
            self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:bindings.cancellation/test]
    # [spec:pgorm:req:bindings.errors+5/test]    a transaction whose connection was given up
    # refuses its next operation and its commit as closed, a LifecycleError, never the
    # ConstructionError an application's refusal is
    async def test_a_discarded_transaction_refuses_with_lifecycle_error(self):
        async with self.pool.connection() as connection:
            with self.assertRaises(p.LifecycleError):
                async with connection.transaction() as tx:
                    active = self.account.active().set("id", 1).set("display name", "wait_before").set("note", None)
                    task = asyncio.ensure_future(active.insert(tx))
                    await asyncio.sleep(0.05)
                    task.cancel()
                    with self.assertRaises(asyncio.CancelledError):
                        await task
                    with self.assertRaises(p.LifecycleError):
                        await self.account.find().all(tx)
            self.assertTrue(tx.closed)
        self.assertEqual(await self.account.find().all(self.pool), [])


if __name__ == "__main__":
    unittest.main()
