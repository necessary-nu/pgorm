"""LISTEN notifications on a dedicated connection, and pg_notify from any executor."""

import asyncio
import os
import unittest

import pgorm as p

# How long a notification that must arrive is waited for. One that must not
# arrive is never waited for: a later notification fences it, since a session
# receives notifications in commit order.
ARRIVAL = 10


async def received(listener):
    notification = await asyncio.wait_for(anext(listener), ARRIVAL)
    return (notification.channel, notification.payload)


async def backend_pid(executor):
    return (await executor.one(p.RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]


async def listening_pid(executor, channel):
    query = p.RawSQL(
        "SELECT pid FROM pg_stat_activity WHERE datname = current_database() AND query = $1",
        [f'LISTEN "{channel}"'],
    )
    return (await executor.one(query))["pid"]


class ListenerSurface(unittest.TestCase):
    # [spec:pgorm:req:bindings.listen+2/test]    listeners come from a pool
    # [spec:pgorm:req:python.truth-testing+4/test]    neither Listener nor Notification is a boolean
    def test_listener_objects_refuse_truth_testing(self):
        with self.assertRaises(TypeError):
            p.Listener(object())
        with self.assertRaises(TypeError):
            p.Notification()
        self.assertIn("__bool__", vars(p.Listener))
        self.assertIn("__bool__", vars(p.Notification))


class ListenerExecution(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=3)

    async def asyncTearDown(self):
        await asyncio.wait_for(self.pool.close(), 2)

    # [spec:pgorm:req:bindings.listen+2/test]    a committed transaction's notification arrives with its sender
    async def test_committed_notification_arrives_with_sender(self):
        async with await self.pool.listener() as listener:
            await listener.listen("py_jobs")
            async with self.pool.connection() as connection:
                sender = await backend_pid(connection)
                async with connection.transaction() as tx:
                    await tx.notify("py_jobs", "42")
            notification = await asyncio.wait_for(anext(listener), ARRIVAL)
            self.assertEqual(
                (notification.channel, notification.payload, notification.process_id),
                ("py_jobs", "42", sender),
            )
            self.assertIn("py_jobs", repr(notification))
            with self.assertRaises(p.ConstructionError):
                bool(notification)
            with self.assertRaises(p.ConstructionError):
                bool(listener)
            self.assertEqual(self.pool.status().size, 1, "the listener took no pooled connection")
        self.assertTrue(listener.closed)

    # [spec:pgorm:req:bindings.listen+2/test]    a rollback discards its notification, which a later one fences
    async def test_rolled_back_notification_never_arrives(self):
        async with await self.pool.listener() as listener:
            await listener.listen("py_rollback")
            async with self.pool.connection() as connection:
                marker = ValueError("roll the notification back")
                with self.assertRaises(ValueError) as caught:
                    async with connection.transaction() as tx:
                        await tx.notify("py_rollback", "discarded")
                        raise marker
                self.assertIs(caught.exception, marker)
                await connection.notify("py_rollback", "fence")
            self.assertEqual(await received(listener), ("py_rollback", "fence"))

    # [spec:pgorm:req:bindings.listen+2/test]    commit order, sending order, duplicates folded
    async def test_notifications_arrive_in_commit_order(self):
        async with await self.pool.listener() as listener:
            await listener.listen("py_order")
            async with self.pool.connection() as first, self.pool.connection() as second:
                async with first.transaction() as early:
                    async with second.transaction() as late:
                        for payload in ("1", "2", "1", "3"):
                            await early.notify("py_order", payload)
                        await late.notify("py_order", "first committed")
            self.assertEqual(
                [(await received(listener))[1] for _ in range(4)],
                ["first committed", "1", "2", "3"],
            )

    # [spec:pgorm:req:bindings.listen+2/test]    unlisten and unlisten_all stop delivery
    async def test_unlisten_stops_delivery(self):
        async with await self.pool.listener() as listener:
            await listener.listen("py_unlisten")
            await listener.listen("py_fence")
            await self.pool.notify("py_unlisten", "heard")
            self.assertEqual(await received(listener), ("py_unlisten", "heard"))
            await listener.unlisten("py_unlisten")
            await self.pool.notify("py_unlisten", "unheard")
            await self.pool.notify("py_fence", "1")
            self.assertEqual(await received(listener), ("py_fence", "1"))
            await listener.unlisten_all()
            await listener.listen("py_after")
            await self.pool.notify("py_fence", "unheard")
            await self.pool.notify("py_after", "2")
            self.assertEqual(await received(listener), ("py_after", "2"))

    # [spec:pgorm:req:bindings.listen+2/test]    a channel is exact; a long or non-text name is refused before sending
    async def test_quoted_channel_names_are_exact(self):
        odd = 'Py Mixed "quoted"; DROP'
        async with await self.pool.listener() as listener:
            await listener.listen(odd)
            await listener.listen("py_fence")
            await self.pool.notify(odd.lower(), "other channel")
            await self.pool.notify(odd, "exact")
            await self.pool.notify("py_fence", "1")
            self.assertEqual(await received(listener), (odd, "exact"))
            self.assertEqual(await received(listener), ("py_fence", "1"))
            with self.assertRaises(p.ConstructionError):
                await listener.listen("c" * 64)
            for name in (b"py_bytes", 7):
                with self.subTest(name=name), self.assertRaises(TypeError):
                    await listener.listen(name)
            with self.assertRaises(p.DatabaseError) as caught:
                await listener.listen("")
            self.assertEqual(caught.exception.sqlstate, "42601")
            await listener.listen("c" * 63)
            self.assertFalse(listener.closed)
            for channel, payload in (("py_fence", b"bytes"), (None, "x")):
                with self.assertRaises(TypeError):
                    await self.pool.notify(channel, payload)

    # [spec:pgorm:req:bindings.listen+2/test]    the payload bound is the server's 22023
    async def test_payload_limit_is_the_servers_error(self):
        async with await self.pool.listener() as listener:
            await listener.listen("py_payload")
            await self.pool.notify("py_payload", "p" * 7999)
            self.assertEqual(await received(listener), ("py_payload", "p" * 7999))
            with self.assertRaises(p.DatabaseError) as caught:
                await self.pool.notify("py_payload", "p" * 8000)
            self.assertEqual(caught.exception.sqlstate, "22023")

    # [spec:pgorm:req:bindings.listen+2/test]    a terminated connection raises its error after what had arrived
    async def test_terminated_connection_ends_iteration(self):
        listener = await self.pool.listener()
        await listener.listen("py_doomed")
        pid = await listening_pid(self.pool, "py_doomed")
        await self.pool.notify("py_doomed", "1")
        await self.pool.notify("py_doomed", "2")
        # The session sends what it has pending before it answers a request,
        # so once this returns both notifications are waiting.
        await listener.listen("py_doomed")
        await self.pool.execute(p.RawSQL("SELECT pg_terminate_backend($1)", [pid]))
        self.assertEqual([(await received(listener))[1] for _ in range(2)], ["1", "2"])
        with self.assertRaises(p.DatabaseError) as caught:
            await asyncio.wait_for(anext(listener), ARRIVAL)
        self.assertEqual(caught.exception.sqlstate, "57P01")
        self.assertTrue(listener.closed)
        with self.assertRaises(StopAsyncIteration):
            await anext(listener)
        with self.assertRaises(p.LifecycleError):
            await listener.listen("py_doomed")

    # [spec:pgorm:req:bindings.listen+2/test]    past its capacity the listener closes with ConnectionError
    async def test_overflow_closes_the_listener(self):
        listener = await self.pool.listener(capacity=2)
        await listener.listen("py_burst")
        async with self.pool.transaction() as tx:
            for payload in ("1", "2", "3"):
                await tx.notify("py_burst", payload)

        # Nothing pulls until the listener's connection has closed, so the
        # overflow is decided by the capacity alone.
        async def probe():
            while True:
                try:
                    await listener.listen("py_probe")
                except p.ConnectionError:
                    return

        await asyncio.wait_for(probe(), ARRIVAL)
        self.assertEqual([(await received(listener))[1] for _ in range(2)], ["1", "2"])
        with self.assertRaises(p.ConnectionError) as caught:
            await anext(listener)
        self.assertIn("2 were waiting unread", str(caught.exception))
        self.assertTrue(listener.closed)
        with self.assertRaises(StopAsyncIteration):
            await anext(listener)

    # [spec:pgorm:req:bindings.listen+2/test]    a capacity is a positive whole number
    # [spec:pgorm:req:bindings.argument-errors+1/test]
    async def test_capacity_is_a_positive_int(self):
        for capacity in (0, -1, 2**64):
            with self.subTest(capacity=capacity), self.assertRaises(ValueError):
                await self.pool.listener(capacity=capacity)
        for capacity in (1.5, True, "8", None):
            with self.subTest(capacity=capacity), self.assertRaises(TypeError):
                await self.pool.listener(capacity=capacity)
        async with await self.pool.listener(capacity=1) as listener:
            self.assertFalse(listener.closed)

    # [spec:pgorm:req:bindings.listen+2/test]    one pull at a time; listen runs beside it; a cancelled pull loses nothing
    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_cancelled_pull_loses_nothing(self):
        async with await self.pool.listener() as listener:
            pending = asyncio.create_task(anext(listener))
            await listener.listen("py_cancel")
            with self.assertRaises(p.LifecycleError):
                await asyncio.wait_for(anext(listener), ARRIVAL)
            pending.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await pending
            self.assertFalse(listener.closed)
            reader = asyncio.create_task(received(listener))
            await listener.listen("py_late")
            await self.pool.notify("py_late", "heard")
            self.assertEqual(await asyncio.wait_for(reader, ARRIVAL), ("py_late", "heard"))
            await self.pool.notify("py_cancel", "kept")
            self.assertEqual(await received(listener), ("py_cancel", "kept"))

    # [spec:pgorm:req:bindings.listen+2/test]    closing ends iteration; closing the pool closes its listeners
    # [spec:pgorm:req:python.lifecycle+3/test]
    async def test_close_and_pool_close_end_listeners(self):
        listener = await self.pool.listener()
        await listener.listen("py_close")
        pending = asyncio.create_task(anext(listener))
        await listener.listen("py_close")
        await asyncio.wait_for(listener.close(), 2)
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(pending, 2)
        with self.assertRaises(StopAsyncIteration):
            await anext(listener)
        with self.assertRaises(p.LifecycleError):
            await listener.listen("py_close")
        listener = await self.pool.listener()
        await listener.listen("py_close")
        pending = asyncio.create_task(anext(listener))
        await listener.listen("py_close")
        await asyncio.wait_for(listener.aclose(), 2)
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(pending, 2)
        self.assertEqual([n async for n in listener], [])

        pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        try:
            listener = await pool.listener()
            await listener.listen("py_pool_close")
            pending = asyncio.create_task(anext(listener))
            await listener.listen("py_pool_close")
            await asyncio.wait_for(pool.close(), 2)
            with self.assertRaises(p.LifecycleError):
                await asyncio.wait_for(pending, 2)
            self.assertTrue(listener.closed)
            with self.assertRaises(p.LifecycleError):
                await pool.listener()
        finally:
            await pool.close()

        async def ended():
            query = p.RawSQL(
                "SELECT count(*) AS n FROM pg_stat_activity WHERE datname = current_database() "
                "AND query IN ('LISTEN \"py_close\"', 'LISTEN \"py_pool_close\"')"
            )
            while (await self.pool.one(query))["n"]:
                pass

        await asyncio.wait_for(ended(), ARRIVAL)


if __name__ == "__main__":
    unittest.main()
