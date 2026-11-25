"""Installed optional-native and feature-disabled constructor contracts."""

import asyncio
import os
from pathlib import Path
import tempfile
import unittest

import pgorm as p

ENABLED = "turso" in p.capabilities()["features"]


class AvailabilityTests(unittest.IsolatedAsyncioTestCase):
    # [spec:pgorm:req:turso.python+1/test]
    async def test_explicit_paths_keep_file_intent(self):
        with tempfile.TemporaryDirectory() as directory:
            original = Path.cwd()
            pool = None
            try:
                os.chdir(directory)
                if ENABLED:
                    pool = p.Pool(Path("host=foo"))
                else:
                    with self.assertRaises(p.UnsupportedCapabilityError):
                        p.Pool(Path("host=foo"))
            finally:
                os.chdir(original)
            path = Path(directory) / "host=foo"
            self.assertFalse(path.exists())
            if pool is not None:
                try:
                    self.assertTrue(await pool.ping())
                    self.assertTrue(path.exists())
                finally:
                    await pool.close()

    # [spec:pgorm:req:turso.python+1/test]
    async def test_feature_metadata_and_unavailable_open(self):
        expected = os.environ.get("PGORM_EXPECT_TURSO")
        if expected is not None:
            self.assertEqual(ENABLED, expected == "1")
        self.assertEqual("pool.turso" in p.capabilities()["operations"], ENABLED)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "availability.db"
            if ENABLED:
                pool = p.Pool(path)
                self.assertFalse(path.exists())
                self.assertTrue(await pool.ping())
                await pool.close()
                self.assertTrue(path.exists())
            else:
                with self.assertRaises(p.UnsupportedCapabilityError):
                    await p.connect(path)
                self.assertFalse(path.exists())

    # [spec:pgorm:req:turso.python+1/test]
    async def test_options_refuse_before_open(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "invalid.db"
            for options in ({"max_size": 0}, {"tag": ""}, {"recycle": "clean"},
                            {"statement_cache_channel": "changes"}, {"acquire_timeout": 0}):
                with self.subTest(options=options):
                    with self.assertRaises(p.ConstructionError) as error:
                        await p.connect(path, **options)
                    self.assertNotIsInstance(error.exception, p.UnsupportedCapabilityError)
            with self.assertRaises(p.ConstructionError):
                await p.connect(path, tls="disable")
            with self.assertRaises(p.ConstructionError):
                await p.connect(path, connect_timeout=10)
            self.assertFalse(path.exists())


@unittest.skipUnless(ENABLED, "package built without experimental turso")
class NativeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = await p.connect(":memory:", max_size=2, acquire_timeout=0.2)
        await self.pool.batch_execute("CREATE TABLE items(id bigint PRIMARY KEY, note text)")

    async def asyncTearDown(self):
        await self.pool.close()

    # [spec:pgorm:req:turso.python+1/test]
    async def test_values_pooling_streams_and_transactions(self):
        first = await self.pool.acquire()
        second = await self.pool.acquire()
        try:
            self.assertEqual(await first.execute(p.RawSQL("INSERT INTO items VALUES($1,$2)", [1, "one"])), 1)
            row = await second.one(p.RawSQL("SELECT id,note FROM items WHERE id=$1", [1]))
            self.assertEqual((row["id"], row["note"]), (1, "one"))
        finally:
            await first.close()
            await second.close()
        async with self.pool.transaction() as tx:
            await tx.execute(p.RawSQL("INSERT INTO items VALUES($1,$2)", [2, p.Value.null("text")]))
            stream = await tx.stream(p.RawSQL("SELECT id FROM items WHERE id>$1 ORDER BY id", [0]))
            self.assertEqual([row["id"] async for row in stream], [1, 2])
        with self.assertRaisesRegex(RuntimeError, "rollback"):
            async with self.pool.transaction() as tx:
                await tx.execute(p.RawSQL("INSERT INTO items VALUES($1,$2)", [3, "three"]))
                raise RuntimeError("rollback")
        stream = await self.pool.stream(p.RawSQL("SELECT id FROM items WHERE id>$1 ORDER BY id", [0]))
        self.assertEqual([row["id"] async for row in stream], [1, 2])
        null = await self.pool.one(p.RawSQL("SELECT note FROM items WHERE id=$1", [2]))
        self.assertIsNone(null["note"])

    # [spec:pgorm:req:turso.python+1/test]
    async def test_file_reopen_and_hooks(self):
        opened = []
        async def created(connection):
            row = await connection.one(p.RawSQL("SELECT $1 AS value", [9]))
            opened.append(row["value"])
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "persist.db"
            pool = await p.connect(path, post_create=created, tag="native-file")
            self.assertEqual(opened, [9])
            self.assertEqual(pool.tag, "native-file")
            await pool.batch_execute("CREATE TABLE persisted(id bigint); INSERT INTO persisted VALUES(42)")
            await pool.close()
            pool = await p.connect(path)
            try:
                self.assertEqual((await pool.one(p.RawSQL("SELECT id FROM persisted")))["id"], 42)
            finally:
                await pool.close()

    # [spec:pgorm:req:turso.python+1/test]
    async def test_refusals_and_loop_ownership(self):
        with self.assertRaises(p.ConstructionError):
            await self.pool.execute(p.RawSQL("COPY items TO STDOUT"))
        with self.assertRaises(p.PgormError):
            await self.pool.listener()
        def foreign():
            return asyncio.run(self.pool.ping())
        with self.assertRaises(p.LifecycleError):
            await asyncio.to_thread(foreign)
        self.assertTrue(await self.pool.ping())
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(p.PgormError):
                await p.connect(Path(directory) / "missing" / "file.db")

    # [spec:pgorm:req:turso.python+1/test]
    async def test_models_and_cancelled_streams(self):
        model = p.Model("items", {"id": p.Column("i64", primary_key=True), "note": p.Column("text")})
        await model.insert({"id": 1, "note": "model"}).execute(self.pool)
        rows = await model.find().all(self.pool)
        self.assertEqual(rows[0]["note"], "model")
        slow = p.RawSQL("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT count(*) AS total FROM n")
        pending = asyncio.create_task(self.pool.one(slow))
        await asyncio.sleep(0.02)
        pending.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(pending, 3)
        self.assertTrue(await asyncio.wait_for(self.pool.ping(), 3))
        stream = await self.pool.stream(p.RawSQL("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT x::bigint AS x FROM n"))
        self.assertEqual((await anext(stream))["x"], 1)
        await stream.aclose()
        self.assertTrue(await asyncio.wait_for(self.pool.ping(), 3))


if __name__ == "__main__":
    unittest.main()
