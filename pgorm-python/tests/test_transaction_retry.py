"""Transactions pgorm runs again, the errors that say so, a failed rollback's
two errors, a discarded connection and a driver refusal's fields."""
import os
import unittest

import pgorm as p
from pgorm import RawSQL


class Retrying(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=4)
        await self.pool.batch_execute(
            "DROP TABLE IF EXISTS python_retry_items;"
            "CREATE TABLE python_retry_items (id integer PRIMARY KEY, n integer NOT NULL);"
            "INSERT INTO python_retry_items VALUES (1, 10)"
        )

    async def asyncTearDown(self):
        if self.pool.closed:
            self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=4)
        await self.pool.execute(RawSQL("DROP TABLE IF EXISTS python_retry_items"))
        await self.pool.close()

    async def current(self):
        return (await self.pool.one(RawSQL("SELECT n FROM python_retry_items WHERE id = 1")))["n"]

    # [spec:pgorm:req:bindings.transaction-retry/test]
    async def test_a_serialization_failure_runs_the_block_again(self):
        attempts = 0

        async def double(tx: p.Transaction) -> int:
            nonlocal attempts
            attempts += 1
            row = await tx.one(RawSQL("SELECT n FROM python_retry_items WHERE id = 1"))
            if attempts == 1:
                await self.pool.execute(RawSQL("UPDATE python_retry_items SET n = n + 1 WHERE id = 1"))
            await tx.execute(RawSQL("UPDATE python_retry_items SET n = $1 WHERE id = 1", [row["n"] * 2]))
            return row["n"] * 2

        result = await self.pool.transaction_with_retry(double, retries=3, mode="read_write", isolation="serializable")
        self.assertEqual((attempts, result, await self.current()), (2, 22, 22))

    # [spec:pgorm:req:bindings.transaction-retry/test]
    async def test_retries_run_out_and_others_never_retry(self):
        attempts = 0

        async def always_conflicts(tx: p.Transaction) -> None:
            nonlocal attempts
            attempts += 1
            await tx.one(RawSQL("SELECT n FROM python_retry_items WHERE id = 1"))
            await self.pool.execute(RawSQL("UPDATE python_retry_items SET n = n + 1 WHERE id = 1"))
            await tx.execute(RawSQL("UPDATE python_retry_items SET n = 0 WHERE id = 1"))

        with self.assertRaises(p.DatabaseError) as caught:
            await self.pool.transaction_with_retry(always_conflicts, retries=2, mode="read_write", isolation="serializable")
        self.assertEqual((attempts, caught.exception.sqlstate, caught.exception.is_retryable), (3, "40001", True))

        attempts = 0

        async def fails(tx: p.Transaction) -> None:
            nonlocal attempts
            attempts += 1
            await tx.execute(RawSQL("UPDATE python_retry_items SET n = 0 WHERE id = 1"))
            raise ValueError("not retryable")

        with self.assertRaises(ValueError):
            await self.pool.transaction_with_retry(fails, retries=5)
        self.assertEqual((attempts, await self.current()), (1, 13))

        async with self.pool.connection() as connection:
            self.assertEqual(await connection.transaction_with_retry(self.read, retries=0), 13)
            self.assertTrue(await connection.ping())
        with self.assertRaises(TypeError):
            await self.pool.transaction_with_retry(self.read, retries="3")  # type: ignore[arg-type]
        with self.assertRaises(p.ConstructionError):
            await self.pool.transaction_with_retry(self.read, retries=-1)

    async def read(self, tx: p.Transaction) -> int:
        return (await tx.one(RawSQL("SELECT n FROM python_retry_items WHERE id = 1")))["n"]

    # [spec:pgorm:req:bindings.transactions+2/test]    a retrying transaction and a transaction's stream meet the aborted COMMIT
    async def test_an_aborted_transaction_is_not_retried(self):
        attempts = 0

        async def swallows(tx: p.Transaction) -> None:
            nonlocal attempts
            attempts += 1
            try:
                await tx.one(RawSQL("SELECT 1 / 0 AS n"))
            except p.DatabaseError:
                pass

        async with self.pool.connection() as connection:
            with self.assertRaises(p.DatabaseError) as caught:
                await connection.transaction_with_retry(swallows, retries=3)
            self.assertEqual((attempts, caught.exception.sqlstate, caught.exception.is_retryable), (1, "25P02", False))
            with self.assertRaises(p.DatabaseError) as streamed:
                async with connection.transaction() as tx:
                    try:
                        async with await tx.stream(RawSQL("SELECT 1 / (n - 10) AS n FROM python_retry_items")) as rows:
                            async for _ in rows:
                                pass
                    except p.DatabaseError:
                        pass
            self.assertEqual(streamed.exception.sqlstate, "25P02")
            self.assertTrue(await connection.ping())

    # [spec:pgorm:req:bindings.transaction-retry/test]
    async def test_only_pgorm_marks_an_error_retryable(self):
        try:
            await self.pool.execute(RawSQL("SELECT * FROM no_such_table"))
        except p.DatabaseError as error:
            self.assertFalse(error.is_retryable)
        self.assertFalse(p.PgormError("x").is_retryable)

    # [spec:pgorm:req:bindings.rollback-failed/test]
    async def test_a_failed_rollback_carries_both_errors(self):
        marker = ValueError("the block's own failure")
        with self.assertRaises(p.RollbackFailedError) as caught:
            async with self.pool.transaction() as tx:
                # Ending the session makes the ROLLBACK fail.
                await self.pool.execute(RawSQL(
                    "SELECT pg_terminate_backend($1) FROM pg_stat_activity WHERE datname = current_database() AND pid = $1",
                    [(await tx.one(RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]],
                ))
                raise marker
        self.assertIs(caught.exception.cause, marker)
        self.assertIs(caught.exception.__cause__, marker)
        self.assertIsInstance(caught.exception.rollback, p.PgormError)
        self.assertTrue(str(caught.exception).startswith("the block's own failure; its ROLLBACK failed: "))

    # [spec:pgorm:req:python.discard/test]
    async def test_a_discarded_connection_is_closed_not_returned(self):
        async with self.pool.connection() as connection:
            pid = (await connection.one(RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]
            await connection.discard()
            self.assertTrue(connection.closed)
            with self.assertRaises(p.LifecycleError):
                await connection.ping()
        pids = {(await self.pool.one(RawSQL("SELECT pg_backend_pid() AS pid")))["pid"] for _ in range(4)}
        self.assertNotIn(pid, pids)

    # [spec:pgorm:req:bindings.refusal-fields/test]
    async def test_a_driver_refusal_carries_its_fields(self):
        with self.assertRaises(p.ConstructionError) as bound:
            await self.pool.one(RawSQL("SELECT $1::smallint AS n", [100_000]))
        self.assertEqual(bound.exception.parameter, 1)
        self.assertEqual(bound.exception.reason, "value `100000` is out of range for Postgres type `int2`")
        self.assertIsNone(p.ConstructionError("x").parameter)
        self.assertIsNone(p.DecodeError("x").reading)


if __name__ == "__main__":
    unittest.main()
