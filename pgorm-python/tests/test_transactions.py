"""Native borrowed transactions, savepoints and bounded cancellation."""
import asyncio
import gc
import os
import time
import unittest

import pgorm as p
from pgorm import schema as s


class RolledBack(Exception):
    """Raised to leave a block, so that its transaction rolls back."""


class Transactions(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=3)
        self.table = p.Table("python_transaction_items")
        await self.pool.execute(s.create_table(self.table).column(s.ColumnDef("id", "integer")).column(s.ColumnDef("name", "text")).primary_key("id"))

    async def asyncTearDown(self):
        if self.pool.closed:
            self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=3)
        await self.pool.execute(s.drop_table(self.table, if_exists=True))
        await self.pool.close()

    def insert(self, identity):
        return p.insert(self.table).columns("id", "name").values(identity, f"item {identity}")

    def rows(self):
        return p.select(p.col("id")).from_(self.table).order_by(p.col("id").asc())

    # [spec:pgorm:req:bindings.transactions+1/test]
    async def test_block_commits_and_reserves_its_connection(self):
        async with self.pool.connection() as connection:
            async with connection.transaction() as tx:
                self.assertFalse(tx.closed)
                await tx.execute(self.insert(1))
                self.assertEqual([r["id"] for r in await tx.query(self.rows())], [1])
                self.assertEqual(await self.pool.query(self.rows()), [])
                with self.assertRaises(p.LifecycleError):
                    await connection.execute(self.insert(2))
            self.assertTrue(tx.closed)
            self.assertTrue(await connection.ping())
            with self.assertRaises(p.LifecycleError):
                await tx.execute(self.insert(2))
            with self.assertRaises(RolledBack):
                async with connection.transaction() as tx:
                    await tx.execute(self.insert(2))
                    raise RolledBack
            self.assertEqual([r["id"] for r in await connection.query(self.rows())], [1])

    # [spec:pgorm:req:python.transactions+3/test]
    async def test_transactions_open_only_for_a_block(self):
        async with self.pool.connection() as connection:
            self.assertFalse(hasattr(connection, "begin"))
            async with connection.transaction() as tx:
                for name in ("begin", "commit", "rollback", "close", "aclose", "__aenter__", "__aexit__"):
                    self.assertFalse(hasattr(tx, name), name)
                with self.assertRaises(TypeError):
                    async with tx:
                        pass
                await tx.execute(self.insert(1))
        self.assertEqual([r["id"] for r in await self.pool.query(self.rows())], [1])

    # [spec:pgorm:req:python.transactions+3/test]
    async def test_contexts_commit_and_preserve_original_exception(self):
        async with self.pool.transaction() as tx:
            await tx.execute(self.insert(1))
        self.assertTrue(tx.closed)
        marker = ValueError("application error")
        with self.assertRaises(ValueError) as caught:
            async with self.pool.transaction() as tx:
                await tx.execute(self.insert(2))
                raise marker
        self.assertIs(caught.exception, marker)
        self.assertTrue(tx.closed)
        self.assertEqual([r["id"] for r in await self.pool.query(self.rows())], [1])

    # [spec:pgorm:req:bindings.transactions+1/test]
    async def test_savepoints_reserve_parents_and_recover_database_errors(self):
        async with self.pool.connection() as connection:
            async with connection.transaction() as tx:
                await tx.execute(self.insert(1))
                with self.assertRaises(p.DatabaseError) as caught:
                    async with tx.transaction() as child:
                        with self.assertRaises(p.LifecycleError):
                            await tx.query(self.rows())
                        with self.assertRaises(p.LifecycleError):
                            async with tx.transaction():
                                pass
                        await child.execute(self.insert(2))
                        await child.execute(self.insert(1))
                self.assertEqual(caught.exception.sqlstate, "23505")
                self.assertTrue(child.closed)
                self.assertFalse(tx.closed)
                async with tx.transaction() as child:
                    await child.execute(self.insert(3))
                    async with child.transaction() as grandchild:
                        await grandchild.execute(self.insert(4))
                self.assertEqual([r["id"] for r in await tx.query(self.rows())], [1, 3, 4])
            self.assertTrue(await connection.ping())
        self.assertEqual([r["id"] for r in await self.pool.query(self.rows())], [1, 3, 4])

    # [spec:pgorm:req:bindings.transactions+1/test]
    async def test_a_caught_failure_rolls_the_block_back(self):
        aborted = "the transaction was rolled back, not committed: a statement that failed inside it had aborted it"
        async with self.pool.connection() as connection:
            with self.assertRaises(p.DatabaseError) as caught:
                async with connection.transaction() as tx:
                    await tx.execute(self.insert(1))
                    with self.assertRaises(p.UniqueViolationError):
                        await tx.execute(self.insert(1))
            self.assertEqual(caught.exception.sqlstate, "25P02")
            self.assertEqual(caught.exception.message, aborted)
            self.assertEqual(str(caught.exception), aborted)
            self.assertTrue(tx.closed)
            self.assertEqual(await connection.query(self.rows()), [])
            async with connection.transaction() as tx:
                await tx.execute(self.insert(2))
                with self.assertRaises(p.DatabaseError) as caught:
                    async with tx.transaction() as savepoint:
                        await savepoint.execute(self.insert(3))
                        with self.assertRaises(p.UniqueViolationError):
                            await savepoint.execute(self.insert(2))
                self.assertEqual(caught.exception.sqlstate, "25P02")
                self.assertTrue(savepoint.closed)
                await tx.execute(self.insert(4))
            self.assertTrue(await connection.ping())
        self.assertEqual([r["id"] for r in await self.pool.query(self.rows())], [2, 4])

    # [spec:pgorm:req:bindings.transactions+1/test]
    async def test_options_and_foreign_parent_rejection(self):
        async with self.pool.connection() as connection, self.pool.connection() as other:
            for mode, isolation in (("read_only", "repeatable_read"), ("read_write", "serializable"), ("deferrable", None)):
                async with connection.transaction(mode=mode, isolation=isolation) as tx:
                    settings = await tx.one(p.RawSQL("SELECT current_setting('transaction_isolation') AS isolation, current_setting('transaction_read_only') AS read_only, current_setting('transaction_deferrable') AS deferrable"))
                    self.assertEqual(settings["isolation"], (isolation or "serializable").replace("_", " "))
                    self.assertEqual(settings["read_only"], "off" if mode == "read_write" else "on")
                    self.assertEqual(settings["deferrable"], "on" if mode == "deferrable" else "off")
                    with self.assertRaises(TypeError):
                        p.Transaction(tx._native, other)
            for kwargs in ({"isolation": "serializable"}, {"mode": "deferrable", "isolation": "serializable"}):
                with self.assertRaises(p.ConstructionError):
                    async with connection.transaction(**kwargs):
                        self.fail("a refused mode opened a transaction")
            for kwargs in ({"mode": "raw SQL"}, {"mode": "read_only", "isolation": "made up"}, {"mode": 1}):
                with self.assertRaises(TypeError):
                    async with connection.transaction(**kwargs):
                        self.fail("a refused mode opened a transaction")
            with self.assertRaises(p.DatabaseError) as caught:
                async with connection.transaction(mode="read_only") as tx:
                    await tx.execute(self.insert(7))
            self.assertEqual(caught.exception.sqlstate, "25006")
            self.assertTrue(await connection.ping())

    # [spec:pgorm:req:python.transactions+3/test]
    async def test_failed_rollback_raises_from_exit_and_discards(self):
        marker = ValueError("the block failed")
        async with self.pool.connection() as connection:
            pid = (await connection.one(p.RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]
            with self.assertRaises((p.ConnectionError, p.DatabaseError)) as caught:
                async with connection.transaction() as tx:
                    await tx.execute(self.insert(1))
                    await self.pool.execute(p.RawSQL("SELECT pg_terminate_backend($1, 5000)", [pid]))
                    raise marker
            self.assertIs(caught.exception.__context__, marker)
            self.assertTrue(tx.closed)
            self.assertTrue(connection.closed)
        self.assertEqual(await self.pool.query(self.rows()), [])

    # [spec:pgorm:req:python.transactions+3/test]
    async def test_failed_rollback_never_replaces_a_cancellation(self):
        entered = asyncio.Event()
        connection = await self.pool.acquire()
        pid = (await connection.one(p.RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]
        async def work():
            async with connection.transaction() as tx:
                await tx.execute(self.insert(1))
                await self.pool.execute(p.RawSQL("SELECT pg_terminate_backend($1, 5000)", [pid]))
                entered.set()
                await asyncio.Event().wait()
        task = asyncio.create_task(work())
        await entered.wait()
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(task, 2)
        self.assertTrue(connection.closed)
        await connection.close()
        self.assertEqual(await self.pool.query(self.rows()), [])

    # [spec:pgorm:req:python.cancellation+1/test]
    async def test_cancelled_request_discards_scope_within_two_seconds(self):
        marker = RolledBack("leave the discarded scope")
        async with self.pool.connection() as connection:
            with self.assertRaises(RolledBack) as caught:
                async with connection.transaction() as tx:
                    async with tx.transaction() as child:
                        pending = asyncio.create_task(child.one(p.RawSQL("SELECT 1 AS n FROM pg_sleep(2)")))
                        await asyncio.sleep(0.05)
                        with self.assertRaises(p.LifecycleError):
                            await child.one(p.select(p.literal(1).as_("n")))
                        started = time.monotonic()
                        pending.cancel()
                        with self.assertRaises(asyncio.CancelledError):
                            await pending
                        raise marker
            self.assertIs(caught.exception, marker)
            await asyncio.wait_for(connection.close(), 2)
            self.assertLess(time.monotonic() - started, 2)
            self.assertTrue(tx.closed)
            self.assertTrue(child.closed)
            self.assertTrue(connection.closed)
            self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_cancelled_idle_context_rolls_back_cleanly(self):
        entered = asyncio.Event()
        async def work():
            async with self.pool.transaction() as tx:
                await tx.execute(self.insert(1))
                entered.set()
                await asyncio.Event().wait()
        task = asyncio.create_task(work())
        await entered.wait()
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(task, 2)
        self.assertEqual(await self.pool.query(self.rows()), [])

    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_pool_shutdown_releases_idle_transaction(self):
        connection = await self.pool.acquire()
        entered, release = asyncio.Event(), asyncio.Event()
        async def work():
            async with connection.transaction() as tx:
                await tx.execute(self.insert(1))
                entered.set()
                await release.wait()
            return tx
        task = asyncio.create_task(work())
        await entered.wait()
        await asyncio.wait_for(self.pool.close(), 2)
        self.assertTrue(connection.closed)
        release.set()
        with self.assertRaises(p.LifecycleError):
            await asyncio.wait_for(task, 2)

    # [spec:pgorm:req:bindings.transactions+1/test]
    async def test_abandoned_savepoint_rolls_back_and_releases_parent(self):
        async with self.pool.transaction() as tx:
            block = tx.transaction()
            child = await block.__aenter__()
            await child.execute(self.insert(1))
            del block, child
            gc.collect()
            async def released():
                while True:
                    try:
                        return await tx.query(self.rows())
                    except p.LifecycleError:
                        await asyncio.sleep(0.005)
            self.assertEqual(await asyncio.wait_for(released(), 2), [])
            await tx.execute(self.insert(2))
        self.assertEqual([r["id"] for r in await self.pool.query(self.rows())], [2])

    # [spec:pgorm:req:python.runtime+2/test]
    async def test_foreign_event_loop_fails_before_query(self):
        async with self.pool.transaction() as tx:
            async def use_transaction():
                await tx.query(self.rows())
            with self.assertRaises(p.LifecycleError):
                await asyncio.to_thread(lambda: asyncio.run(use_transaction()))
            await tx.execute(self.insert(1))

    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_exception_with_background_query_discards_scope(self):
        pending = None
        marker = ValueError("block failed during concurrent query")
        with self.assertRaises(ValueError) as caught:
            async with self.pool.transaction() as tx:
                pending = asyncio.create_task(tx.one(p.RawSQL("SELECT 1 AS n FROM pg_sleep(2)")))
                await asyncio.sleep(0.05)
                raise marker
        self.assertIs(caught.exception, marker)
        with self.assertRaises(p.LifecycleError):
            await asyncio.wait_for(pending, 2)
        self.assertTrue(tx.closed)
        self.assertTrue(await self.pool.ping())


if __name__ == "__main__":
    unittest.main()
