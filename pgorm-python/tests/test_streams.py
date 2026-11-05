"""Streams and cancelled queries must leave no uncertain pooled connection."""

import asyncio
import gc
import os
import unittest

import pgorm as p


class StreamTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1, acquire_timeout=0.5)

    async def asyncTearDown(self):
        await asyncio.wait_for(self.pool.close(), 2)

    # [spec:pgorm:req:bindings.streams+1/test]
    async def test_stream_exhaustion_releases_pool_lease(self):
        stream = await self.pool.stream(p.RawSQL("SELECT generate_series(1, 20) AS n"))
        with self.assertRaises(p.TimeoutError):
            await self.pool.acquire()
        self.assertEqual([row["n"] async for row in stream], list(range(1, 21)))
        self.assertTrue(stream.closed)
        self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:bindings.streams+1/test]
    async def test_connection_stream_reserves_and_releases_owner(self):
        async with self.pool.connection() as connection:
            before = await connection.one(p.RawSQL("SELECT pg_backend_pid() AS pid"))
            stream = await connection.stream(p.RawSQL("SELECT generate_series(1, 3) AS n"))
            with self.assertRaises(p.LifecycleError):
                await connection.ping()
            self.assertEqual(len([row async for row in stream]), 3)
            after = await connection.one(p.RawSQL("SELECT pg_backend_pid() AS pid"))
            self.assertEqual(before["pid"], after["pid"])

    # [spec:pgorm:req:bindings.streams+1/test]
    async def test_early_close_discards_incomplete_query(self):
        async with self.pool.connection() as connection:
            before = (await connection.one(p.RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]
            async with await connection.stream(p.RawSQL("SELECT generate_series(1, 1000000) AS n")) as rows:
                self.assertEqual((await anext(rows))["n"], 1)
            self.assertTrue(connection.closed)
        after = (await self.pool.one(p.RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]
        self.assertNotEqual(before, after)

    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_idle_stream_does_not_block_owner_shutdown(self):
        for close_pool in (False, True):
            pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
            try:
                connection = await pool.acquire()
                rows = await connection.stream(p.RawSQL("SELECT generate_series(1, 1000000) AS n"))
                await asyncio.wait_for(pool.close() if close_pool else connection.close(), 1)
                with self.assertRaises(p.LifecycleError):
                    await anext(rows)
                await rows.aclose()
            finally:
                await pool.close()

    # [spec:pgorm:req:bindings.streams+1/test]
    async def test_stream_abandonment_releases_without_async_cleanup(self):
        rows = await self.pool.stream(p.RawSQL("SELECT generate_series(1, 1000000) AS n"))
        self.assertEqual((await anext(rows))["n"], 1)
        del rows
        gc.collect()
        self.assertTrue(await asyncio.wait_for(self.pool.ping(), 1))

    # [spec:pgorm:req:bindings.streams+1/test]
    async def test_decode_failure_is_not_stream_exhaustion(self):
        rows = await self.pool.stream(p.RawSQL("SELECT timetz '12:00+02' AS n"))
        with self.assertRaises(p.DecodeError):
            await anext(rows)
        self.assertTrue(rows.closed)
        self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:bindings.streams+1/test]
    async def test_database_failure_keeps_sqlstate(self):
        with self.assertRaises(p.DatabaseError) as caught:
            rows = await self.pool.stream(p.RawSQL("SELECT 1/0 AS n"))
            await anext(rows)
        self.assertEqual(caught.exception.sqlstate, "22012")
        self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_cancelled_query_discards_native_connection(self):
        connection = await self.pool.acquire()
        before = (await connection.one(p.RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]
        query = asyncio.create_task(connection.query(p.RawSQL("SELECT pg_sleep(30)")))
        await asyncio.sleep(0.05)
        self.assertFalse(query.done())
        query.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(query, 1)
        await asyncio.wait_for(connection.close(), 1)
        after = (await self.pool.one(p.RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]
        self.assertNotEqual(before, after)

    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_pool_shutdown_interrupts_active_query(self):
        query = asyncio.create_task(self.pool.query(p.RawSQL("SELECT pg_sleep(30)")))
        await asyncio.sleep(0.05)
        await asyncio.wait_for(self.pool.close(), 1)
        with self.assertRaises(p.LifecycleError):
            await asyncio.wait_for(query, 1)

    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_cancellation_interrupts_a_pending_stream_pull(self):
        # Large rows flush the initial response before a later row sleeps.
        # Consume in one task because PostgreSQL may flush a partial final row.
        query = p.RawSQL(
            "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM seq WHERE n<4) "
            "SELECT n, repeat('x', 16384) AS padding FROM seq "
            "WHERE CASE WHEN n<3 THEN true ELSE pg_sleep(30) IS NULL END"
        )
        rows = await asyncio.wait_for(self.pool.stream(query), 2)
        seen = []

        async def consume():
            async for row in rows:
                seen.append(row["n"])

        pending = asyncio.create_task(consume())
        await asyncio.sleep(0.05)
        self.assertFalse(pending.done())
        self.assertTrue(seen)
        self.assertEqual(seen[0], 1)
        pending.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(pending, 1)
        self.assertTrue(rows.closed)
        self.assertTrue(await asyncio.wait_for(self.pool.ping(), 1))

    # [spec:pgorm:req:bindings.streams+1/test]
    async def test_late_database_error_preserves_prior_rows(self):
        query = p.RawSQL("SELECT CASE WHEN n=3 THEN 1/(n-3) ELSE n END AS n FROM generate_series(1, 3) AS n")
        rows = await self.pool.stream(query)
        self.assertEqual((await anext(rows))["n"], 1)
        self.assertEqual((await anext(rows))["n"], 2)
        with self.assertRaises(p.DatabaseError) as caught:
            await anext(rows)
        self.assertEqual(caught.exception.sqlstate, "22012")
        self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:python.runtime+1/test]
    async def test_foreign_loop_does_not_consume_stream(self):
        rows = await self.pool.stream(p.RawSQL("SELECT generate_series(1, 3) AS n"))
        for action in (lambda: anext(rows), rows.aclose):
            def foreign():
                return asyncio.run(action())
            with self.assertRaises(p.LifecycleError):
                await asyncio.wait_for(asyncio.to_thread(foreign), 1)
        self.assertFalse(rows.closed)
        self.assertEqual([row["n"] async for row in rows], [1, 2, 3])

    # [spec:pgorm:req:python.truth-testing+4/test]    a stream's truth would not say whether rows remain
    async def test_result_stream_refuses_truth_testing(self):
        async with await self.pool.stream(p.RawSQL("SELECT 1 AS n")) as rows:
            with self.assertRaises(p.ConstructionError):
                bool(rows)
            self.assertEqual([row["n"] async for row in rows], [1])
        with self.assertRaises(p.ConstructionError):
            bool(rows)


if __name__ == "__main__":
    unittest.main()
