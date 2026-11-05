"""pgorm's metrics layer: an InstrumentedPool reports what runs through it to a Python collector."""

import asyncio
import gc
import logging
import os
import unittest

import pgorm as p
import pgorm.pipeline as pl
from pgorm.metrics import _hooks

WIDGET = p.Table("widget", schema="python_metrics")


class Recorder(p.NoOpMetrics):
    """Every report, in order, as a tuple naming its hook."""

    def __init__(self) -> None:
        self.reports: list[tuple[object, ...]] = []

    def record_query_success(self, query, duration, rows):
        self.reports.append(("ok", query.operation, query.sql, rows, query.fingerprint, duration))

    def record_query_error(self, query, duration, error):
        self.reports.append(("error", query.operation, query.sql, type(error), duration))

    def record_connection_acquired(self, duration):
        self.reports.append(("acquired", duration))

    def record_connection_error(self, duration, error):
        self.reports.append(("refused", type(error)))

    def record_transaction_begin(self, duration):
        self.reports.append(("begin",))

    def record_transaction_commit(self, duration):
        self.reports.append(("commit",))

    def record_transaction_rollback(self, duration):
        self.reports.append(("rollback",))

    def hooks(self) -> list[object]:
        return [report[0] for report in self.reports]

    def operations(self, hook: str = "ok") -> list[object]:
        return [report[1] for report in self.reports if report[0] == hook]


class Surface(unittest.TestCase):
    # [spec:pgorm:req:bindings.metrics/test]    a fingerprint is pgorm's, the same for statements differing in constants alone
    def test_fingerprints_are_pgorms(self):
        def print_(sql):
            fingerprint = p.QueryContext("query_all", sql).fingerprint
            self.assertIsNotNone(fingerprint)
            return fingerprint

        by_id = print_("SELECT id FROM widget WHERE id = 1")
        self.assertEqual(str(by_id), "394a2f90c244bffe")
        self.assertEqual(by_id.value, int("394a2f90c244bffe", 16))
        self.assertEqual(by_id, print_("SELECT id FROM widget WHERE id = 4242"))
        self.assertEqual(hash(by_id), hash(print_("SELECT id FROM public.widget WHERE id = 1")))
        self.assertNotEqual(by_id, print_("SELECT name FROM widget WHERE id = 1"))
        self.assertEqual(repr(by_id), "QueryFingerprint('394a2f90c244bffe')")
        self.assertIsNone(p.QueryContext("begin").fingerprint)
        self.assertIsNone(p.QueryContext("execute", "SELEKT nonsense").fingerprint)
        with self.assertRaises(TypeError):
            p.QueryFingerprint()  # type: ignore[call-arg]
        capabilities = p.capabilities()
        self.assertIn("metrics-fingerprint", capabilities["features"])
        self.assertEqual(
            capabilities["operations"]["metrics.fingerprint"]["rust_api"],
            "pgorm::metric::QueryContext::fingerprint",
        )
        self.assertEqual(
            capabilities["operations"]["pool.instrumented"]["rust_api"],
            "pgorm::metric::InstrumentedPool::new",
        )

    # [spec:pgorm:req:bindings.metrics/test]    a collector has all seven hooks, and one left as NoOpMetrics wrote it is never run
    def test_collectors_are_checked_and_noop_hooks_skipped(self):
        class QueriesOnly(p.NoOpMetrics):
            def record_query_success(self, query, duration, rows):
                return None

        hooks = _hooks(QueriesOnly())
        self.assertEqual(len(hooks), 7)
        self.assertIsNotNone(hooks[0])
        self.assertEqual(hooks[1:], [None] * 6)
        self.assertEqual(_hooks(p.NoOpMetrics()), [None] * 7)
        self.assertTrue(all(hook is not None for hook in _hooks(p.LoggingMetrics())))

        class Missing:
            def record_query_success(self, query, duration, rows):
                return None

        with self.assertRaisesRegex(TypeError, "record_query_error"):
            _hooks(Missing())


class Instrumented(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        await self.pool.execute(p.RawSQL("CREATE SCHEMA python_metrics"))
        await self.pool.execute(
            p.RawSQL("CREATE TABLE python_metrics.widget (id integer PRIMARY KEY, name text NOT NULL)")
        )
        await self.pool.execute(
            p.RawSQL("INSERT INTO python_metrics.widget VALUES (1, 'bolt'), (2, 'nut')")
        )
        self.recorder = Recorder()
        self.instrumented = p.InstrumentedPool(self.pool, self.recorder)

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP SCHEMA python_metrics CASCADE"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.metrics/test]    statements, their rows and their failures are reported, and the failure still reaches the caller
    async def test_statements_and_acquisitions_are_reported(self):
        rows = await self.instrumented.query(p.RawSQL("SELECT id FROM python_metrics.widget WHERE id > $1", [0]))
        self.assertEqual(len(rows), 2)
        with self.assertRaises(p.DatabaseError):
            await self.instrumented.query(p.RawSQL("SELECT * FROM python_metrics.missing"))
        changed = await self.instrumented.execute(p.RawSQL("UPDATE python_metrics.widget SET name = name"))
        self.assertEqual(changed, 2)

        self.assertEqual(self.recorder.hooks(), ["acquired", "ok", "acquired", "error", "acquired", "ok"])
        ok, error, update = (r for r in self.recorder.reports if r[0] != "acquired")
        self.assertEqual(ok[1:4], ("query_all", "SELECT id FROM python_metrics.widget WHERE id > $1", 2))
        self.assertEqual(ok[4], p.QueryContext("query_all", ok[2]).fingerprint)
        self.assertIsInstance(ok[5], float)
        self.assertEqual(error[1:4], ("query_all", "SELECT * FROM python_metrics.missing", p.DatabaseError))
        self.assertEqual(update[1], "execute")
        self.assertEqual(update[3], 2)

    # [spec:pgorm:req:bindings.metrics/test]    a model's query, a pipeline, a stream, a COPY's start and a notification are reported
    async def test_every_executor_path_is_reported(self):
        widget = p.Model("widget", {
            "id": p.Column("i32", primary_key=True),
            "name": p.Column("text"),
        }, schema="python_metrics")
        self.assertEqual(len(await widget.find().all(self.instrumented)), 2)
        name = pl.col("widget", "name")
        bolt = pl.from_(WIDGET).filter(name == "bolt").select(name)
        self.assertEqual((await bolt.one(self.instrumented))["name"], "bolt")
        async with self.instrumented.connection() as connection:
            stream = await connection.stream(p.RawSQL("SELECT id FROM python_metrics.widget"))
            self.assertEqual(len([row async for row in stream]), 2)
            copy = p.CopyFrom(WIDGET, "text")
            self.assertEqual(await connection.copy_in(copy, b"3\tscrew\n"), 1)
            await connection.notify("python_metrics", "done")

        self.assertEqual(
            self.recorder.operations(),
            ["query_all", "query_all", "query_raw", "copy_in", "execute"],
        )
        copy_sql = [r[2] for r in self.recorder.reports if r[0] == "ok"][3]
        self.assertTrue(copy_sql.startswith('COPY "python_metrics"."widget"'), copy_sql)
        self.assertEqual(self.recorder.hooks().count("acquired"), 3)

    # [spec:pgorm:req:bindings.metrics/test]    a transaction, its savepoint, its mode and its rollback are reported as Rust reports them
    async def test_transactions_are_reported(self):
        async with self.instrumented.transaction() as tx:
            await tx.query(p.RawSQL("SELECT 1"))
            async with tx.transaction() as savepoint:
                await savepoint.execute(p.RawSQL("UPDATE python_metrics.widget SET name = 'x' WHERE id = 1"))
        with self.assertRaises(p.DatabaseError):
            async with self.instrumented.transaction(mode="read_only") as tx:
                await tx.execute(p.RawSQL("DELETE FROM python_metrics.widget"))
        with self.assertRaises(RuntimeError):
            async with self.instrumented.transaction():
                raise RuntimeError("changed my mind")

        self.assertEqual(
            self.recorder.hooks(),
            [
                "acquired", "begin", "ok", "begin", "ok", "commit", "commit",
                "acquired", "begin", "error", "rollback",
                "acquired", "begin", "rollback",
            ],
        )

    # [spec:pgorm:req:bindings.metrics/test]    an async hook is awaited: once the statement returns, its collector has been told
    async def test_async_hooks_are_awaited(self):
        class Slow(p.NoOpMetrics):
            def __init__(self) -> None:
                self.told: list[str] = []

            async def record_query_success(self, query, duration, rows):
                await asyncio.sleep(0.05)
                self.told.append(query.operation)

        slow = Slow()
        instrumented = p.InstrumentedPool(self.pool, slow)
        await instrumented.one(p.RawSQL("SELECT 1 AS one"))
        self.assertEqual(slow.told, ["query_all"])

    # [spec:pgorm:req:bindings.metrics/test]    a hook that raises, or an async one that fails, breaks no statement: the loop's exception handler is told
    async def test_failing_hooks_reach_the_exception_handler(self):
        class Broken(p.NoOpMetrics):
            def record_query_success(self, query, duration, rows):
                raise ValueError("plain hook")

            async def record_transaction_commit(self, duration):
                raise KeyError("async hook")

        handled: list[dict[str, object]] = []
        loop = asyncio.get_running_loop()
        loop.set_exception_handler(lambda _, context: handled.append(context))
        try:
            instrumented = p.InstrumentedPool(self.pool, Broken())
            row = await instrumented.one(p.RawSQL("SELECT 7 AS seven"))
            self.assertEqual(row["seven"], 7)
            async with instrumented.transaction() as tx:
                await tx.execute(p.RawSQL("UPDATE python_metrics.widget SET name = 'y' WHERE id = 2"))
            self.assertEqual((await self.pool.one(p.RawSQL("SELECT name FROM python_metrics.widget WHERE id = 2")))["name"], "y")
        finally:
            loop.set_exception_handler(None)

        self.assertEqual(
            [(context["message"], type(context["exception"])) for context in handled],
            [
                ("pgorm's metrics hook record_query_success raised", ValueError),
                ("pgorm's metrics hook record_query_success raised", ValueError),
                ("pgorm's metrics hook record_transaction_commit raised", KeyError),
            ],
        )

    # [spec:pgorm:req:bindings.metrics/test]    the wrapped pool runs unreported, shares the instrumented pool's connections, and closes with it
    async def test_inner_pool_is_shared_and_unreported(self):
        self.assertIs(self.instrumented.inner, self.pool)
        self.assertIs(self.instrumented.metrics, self.recorder)
        self.assertIsInstance(self.instrumented, p.Pool)
        await self.pool.query(p.RawSQL("SELECT 1"))
        self.assertEqual(self.recorder.reports, [])
        async with self.instrumented.connection():
            self.assertEqual(self.pool.status(), self.instrumented.status())
        with self.assertRaises(TypeError):
            p.InstrumentedPool(self.instrumented, Recorder())
        with self.assertRaises(TypeError):
            p.InstrumentedPool(object(), Recorder())  # type: ignore[arg-type]

        other = p.InstrumentedPool(self.pool, Recorder())
        del other
        gc.collect()
        self.assertFalse(self.pool.closed)
        await self.pool.query(p.RawSQL("SELECT 1"))

    # [spec:pgorm:req:bindings.metrics/test]    LoggingMetrics writes Rust's messages to Python's logging at Rust's levels
    async def test_logging_metrics_writes_to_logging(self):
        logger = logging.getLogger("pgorm.metrics.test")
        instrumented = p.InstrumentedPool(self.pool, p.LoggingMetrics(logger))
        with self.assertLogs(logger, logging.DEBUG) as logs:
            await instrumented.query(p.RawSQL("SELECT id FROM python_metrics.widget WHERE id = 1"))
            with self.assertRaises(p.DatabaseError):
                await instrumented.query(p.RawSQL("SELECT * FROM python_metrics.missing"))
        levels = [(record.levelname, record.getMessage().split(" in ")[0].split(" after ")[0]) for record in logs.records]
        self.assertEqual(
            levels,
            [
                ("DEBUG", "Connection acquired"),
                ("DEBUG", "Query query_all succeeded"),
                ("DEBUG", "Connection acquired"),
                ("WARNING", "Query query_all failed"),
            ],
        )
        self.assertTrue(logs.records[1].getMessage().endswith(", 1 rows [" + str(
            p.QueryContext("query_all", "SELECT id FROM python_metrics.widget WHERE id = 1").fingerprint
        ) + "]"))
        self.assertIs(logging.getLogger("pgorm.metrics"), p.LoggingMetrics().logger)

    # [spec:pgorm:req:bindings.metrics/test]    closing the instrumented pool closes the pool it wraps
    async def test_closing_either_closes_both(self):
        pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        instrumented = p.InstrumentedPool(pool, Recorder())
        await instrumented.close()
        self.assertTrue(pool.closed)
        with self.assertRaises(p.LifecycleError):
            await pool.query(p.RawSQL("SELECT 1"))


if __name__ == "__main__":
    unittest.main()
