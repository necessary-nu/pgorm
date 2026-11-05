"""COPY FROM STDIN and COPY TO STDOUT through pgorm's CopyTrait."""

import asyncio
import datetime as dt
from decimal import Decimal
import os
import unittest
from uuid import UUID

import pgorm as p


ITEMS = "python_copy_items"
MIRROR = "python_copy_mirror"


def table_ddl(name):
    return p.RawSQL(
        f'CREATE TABLE "{name}" (id integer PRIMARY KEY, name text NOT NULL, '
        "note text DEFAULT 'none', score integer DEFAULT 7)"
    )


class RolledBack(Exception):
    """Raised to leave a block, so that its transaction rolls back."""


async def collect(stream):
    async with stream:
        return b"".join([chunk async for chunk in stream])


class CopyStatements(unittest.TestCase):
    # [spec:pgorm:req:bindings.copy/test]    the format and direction gate each option
    def test_wrong_format_or_direction_raises_construction_error(self):
        t = p.Table("t")
        refusals = [
            (lambda: p.CopyFrom(t, "text").quote("'"), "csv"),
            (lambda: p.CopyFrom(t, "text").force_null_all(), "csv"),
            (lambda: p.CopyFrom(t, "binary").header(), "binary"),
            (lambda: p.CopyFrom(t, "binary").delimiter(","), "binary"),
            (lambda: p.CopyFrom(t, "binary").on_error_ignore(), "binary"),
            (lambda: p.CopyTo(t, "binary").null_string(""), "binary"),
            (lambda: p.CopyTo(t, "text").force_quote_all(), "csv"),
            (lambda: p.CopyFrom(t, "csv").force_quote_all(), "COPY TO"),
            (lambda: p.CopyTo(t, "csv").freeze(), "COPY FROM"),
            (lambda: p.CopyTo(t, "csv").header_match(), "COPY FROM"),
            (lambda: p.CopyTo(t, "csv").force_null("id"), "COPY FROM"),
            (lambda: p.CopyTo(t, "csv").columns("id"), "Select"),
            (lambda: p.CopyTo(t, "csv").where(p.col("id") > 1), "Select"),
            (lambda: p.CopyFrom(t, "csv").delimiter("ab"), "one-character"),
            (lambda: p.CopyFrom(t, "csv").quote(""), "one-character"),
            (lambda: p.CopyFrom(t, "csv").null_string("a\0b"), "NUL"),
            (lambda: p.CopyFrom(t, "csv").on_error_ignore(reject_limit=0), "reject_limit"),
            (lambda: p.CopyFrom(t, "csv").columns("id", "id"), "duplicate"),
            (lambda: p.CopyFrom(t.as_("x"), "csv"), "alias"),
            (lambda: p.CopyTo(p.update(t).set("id", 1), "csv"), "where"),
        ]
        for build, reason in refusals:
            with self.subTest(reason=reason), self.assertRaises(p.ConstructionError) as caught:
                build()
            self.assertIn(reason, str(caught.exception))
        mistyped = [
            (lambda: p.CopyFrom(t, "yaml"), "format"),
            (lambda: p.CopyFrom(t, "csv").encoding(8), "string"),
            (lambda: p.CopyFrom(t, "csv").delimiter(1), "string"),
            (lambda: p.CopyFrom(t, "csv").on_error_ignore(reject_limit=True), "reject_limit"),
            (lambda: p.CopyFrom(t, "csv").on_error_ignore(log_verbosity="loud"), "log_verbosity"),
            (lambda: p.CopyTo(p.RawSQL("SELECT 1"), "csv"), "Select"),
        ]
        for build, reason in mistyped:
            with self.subTest(reason=reason), self.assertRaises(TypeError) as caught:
                build()
            self.assertIn(reason, str(caught.exception))

    # [spec:pgorm:req:bindings.copy/test]    a write without RETURNING is refused as a COPY TO source
    def test_write_without_returning_is_refused(self):
        t, s = p.Table("t"), p.Table("s")
        merge = p.merge(t, s, t.col("id") == s.col("id")).when_matched(p.MergeAction.delete())
        writes = [
            (p.insert(t).columns("id").values(1), "INSERT"),
            (p.update(t).set("id", 1).all_rows(), "UPDATE"),
            (p.delete(t).where(p.col("id") == 1), "DELETE"),
            (merge, "MERGE"),
        ]
        for write, keyword in writes:
            for format in ("text", "csv", "binary"):
                with self.subTest(write=keyword, format=format):
                    with self.assertRaises(p.ConstructionError) as caught:
                        p.CopyTo(write, format)
                    self.assertIn(f"this {keyword} has no RETURNING list", str(caught.exception))
        returning = [
            p.insert(t).columns("id").values(1).returning(p.col("id")),
            p.update(t).set("id", 1).all_rows().returning(p.col("id")),
            p.delete(t).where(p.col("id") == 1).returning(p.col("id")),
            merge.returning_action(),
        ]
        for write in returning:
            with self.subTest(sql=write.inspect().sql):
                self.assertIn("RETURNING", p.CopyTo(write, "text").inspect().sql)

    # [spec:pgorm:req:bindings.copy/test]    inspection is the Rust rendering, values inline
    def test_inspection_renders_rust_sql_without_parameters(self):
        t = p.Table("glyph", schema="art")
        base = p.CopyFrom(t, "csv")
        built = (
            base.columns("id", "image").header().null_string("").force_null("image")
            .where(p.col("id") > p.bind(2))
        )
        self.assertEqual(base.inspect().sql, 'COPY "art"."glyph" FROM STDIN WITH (FORMAT csv)')
        self.assertEqual(
            built.inspect().sql,
            'COPY "art"."glyph" ("id", "image") FROM STDIN WITH (FORMAT csv, NULL \'\', '
            'HEADER true, FORCE_NULL ("image")) WHERE "id" > 2',
        )
        self.assertEqual(built.inspect().params, [])
        self.assertEqual((base.format, built.format), ("csv", "csv"))
        ignore = p.CopyFrom(t, "text").on_error_ignore(5, "verbose").inspect().sql
        self.assertIn("ON_ERROR ignore, REJECT_LIMIT 5", ignore)
        self.assertIn("LOG_VERBOSITY verbose", ignore)
        query = p.select(p.col("id")).from_(p.Table("glyph")).where(p.col("name") == p.bind("it's"))
        out = p.CopyTo(query, "csv").header().force_quote_all().inspect()
        self.assertEqual(
            out.sql,
            'COPY (SELECT "id" FROM "glyph" WHERE "name" = E\'it\\\'s\') TO STDOUT '
            "WITH (FORMAT csv, HEADER true, FORCE_QUOTE *)",
        )
        self.assertEqual(out.params, [])
        self.assertEqual(p.CopyTo(t, "binary").inspect().sql, 'COPY "art"."glyph" TO STDOUT WITH (FORMAT binary)')


class CopyExecution(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=3)
        for name in (ITEMS, MIRROR):
            await self.pool.execute(p.RawSQL(f'DROP TABLE IF EXISTS "{name}"'))
            await self.pool.execute(table_ddl(name))
        self.items = p.Table(ITEMS)
        self.mirror = p.Table(MIRROR)

    async def asyncTearDown(self):
        if self.pool.closed:
            self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=3)
        for name in (ITEMS, MIRROR):
            await self.pool.execute(p.RawSQL(f'DROP TABLE IF EXISTS "{name}"'))
        await asyncio.wait_for(self.pool.close(), 2)

    def rows(self, table=None):
        table = self.items if table is None else table
        return p.select(p.col("id"), p.col("name"), p.col("note"), p.col("score")).from_(table).order_by(p.col("id").asc())

    async def stored(self, executor=None, table=None):
        records = await (executor or self.pool).query(self.rows(table))
        return [tuple(record.values()) for record in records]

    # [spec:pgorm:req:bindings.copy/test]    text and csv data both ways
    async def test_text_and_csv_copy_in_and_out(self):
        text = p.CopyFrom(self.items, "text").columns("id", "name", "note", "score")
        self.assertEqual(await self.pool.copy_in(text, b"1\talpha\t\\N\t1\n2\tbeta\tb\t2\n"), 2)
        csv = p.CopyFrom(self.items, "csv").columns("id", "name", "note").header()
        source = [b"id,name,note\n", bytearray(b'3,"gamma, quoted",\n'), memoryview(b"4,delta,d\n")]
        self.assertEqual(await self.pool.copy_in(csv, source), 2)
        self.assertEqual(await self.stored(), [
            (1, "alpha", None, 1), (2, "beta", "b", 2), (3, "gamma, quoted", None, 7), (4, "delta", "d", 7),
        ])
        out = await collect(await self.pool.copy_out(p.CopyTo(self.items, "text")))
        self.assertEqual(out.splitlines()[:2], [b"1\talpha\t\\N\t1", b"2\tbeta\tb\t2"])
        query = p.select(p.col("id"), p.col("name")).from_(self.items).order_by(p.col("id").asc())
        out = await collect(await self.pool.copy_out(p.CopyTo(query, "csv").header().force_quote("name")))
        self.assertEqual(out, b'id,name\n1,"alpha"\n2,"beta"\n3,"gamma, quoted"\n4,"delta"\n')

    # [spec:pgorm:req:python.truth-testing+4/test]    a stream's truth would not say whether data remains
    async def test_copy_out_stream_refuses_truth_testing(self):
        await self.pool.copy_in(p.CopyFrom(self.items, "text").columns("id", "name"), b"1\talpha\n")
        stream = await self.pool.copy_out(p.CopyTo(self.items, "text"))
        with self.assertRaises(p.ConstructionError):
            bool(stream)
        self.assertEqual(await collect(stream), b"1\talpha\tnone\t7\n")
        with self.assertRaises(p.ConstructionError):
            bool(stream)

    # [spec:pgorm:req:bindings.copy/test]    binary out of one table is binary into another
    async def test_binary_round_trip_between_tables(self):
        await self.pool.copy_in(
            p.CopyFrom(self.items, "csv"),
            (f"{n},name {n},{'' if n % 3 else 'n'},{n * 2}\n".encode() for n in range(1, 2001)),
        )
        async with self.pool.connection() as connection:
            data = await collect(await connection.copy_out(p.CopyTo(self.items, "binary")))
            self.assertTrue(data.startswith(b"PGCOPY\n\xff\r\n\0"))
            self.assertEqual(await connection.copy_in(p.CopyFrom(self.mirror, "binary"), data), 2000)
        self.assertEqual(await self.stored(table=self.mirror), await self.stored())

    # [spec:pgorm:req:bindings.copy/test]    omitted columns take their defaults
    async def test_column_subset_takes_defaults(self):
        statement = p.CopyFrom(self.items, "text").columns("id").columns("name")
        self.assertEqual(await self.pool.copy_in(statement, b"5\tfive\n"), 1)
        self.assertEqual(await self.stored(), [(5, "five", "none", 7)])
        where = p.CopyFrom(self.items, "csv").columns("id", "name").where(p.col("id") > 6)
        self.assertEqual(await self.pool.copy_in(where, b"6,six\n7,seven\n"), 1)
        self.assertEqual([row[0] for row in await self.stored()], [5, 7])

    # [spec:pgorm:req:bindings.copy/test]    a refused row raises 23505 and aborts the transaction
    async def test_unique_violation_aborts_transaction_until_rollback(self):
        async with self.pool.connection() as connection:
            with self.assertRaises(p.DatabaseError) as aborted:
                async with connection.transaction() as tx:
                    await tx.execute(p.insert(self.items).columns("id", "name").values(1, "kept"))
                    with self.assertRaises(p.DatabaseError) as caught:
                        await tx.copy_in(p.CopyFrom(self.items, "text").columns("id", "name"), b"2\tb\n1\tdup\n3\tc\n")
                    self.assertEqual(caught.exception.sqlstate, "23505")
                    await tx.query(self.rows())
            self.assertEqual(aborted.exception.sqlstate, "25P02")
            self.assertEqual(await self.stored(connection), [])
            self.assertTrue(await connection.ping())

    # [spec:pgorm:req:bindings.copy/test]    a raising source aborts and the connection stays usable
    # [spec:pgorm:req:bindings.argument-errors+1/test]
    async def test_raising_source_aborts_and_keeps_connection(self):
        marker = ValueError("source failed")

        async def failing():
            yield b"".join(f"{n}\tname\n".encode() for n in range(1, 20001))
            raise marker

        def bad_item():
            yield b"1\tone\n"
            yield "2\ttwo\n"

        statement = p.CopyFrom(self.items, "text").columns("id", "name")
        async with self.pool.connection() as connection:
            pid = (await connection.one(p.RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]
            with self.assertRaises(ValueError) as caught:
                await connection.copy_in(statement, failing())
            self.assertIs(caught.exception, marker)
            self.assertEqual(await self.stored(connection), [])
            with self.assertRaises(TypeError):
                await connection.copy_in(statement, bad_item())
            for invalid in ("1\tone\n", 7, None):
                with self.assertRaises(TypeError):
                    await connection.copy_in(statement, invalid)
            with self.assertRaises(TypeError):
                await connection.execute(statement)
            self.assertEqual(await self.stored(connection), [])
            self.assertEqual(await connection.copy_in(statement, [b"9\tnine\n"]), 1)
            again = (await connection.one(p.RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]
            self.assertEqual(pid, again)
            self.assertFalse(connection.closed)

    # [spec:pgorm:req:bindings.copy/test]    a raising source inside a transaction aborts it until rollback
    async def test_raising_source_aborts_enclosing_transaction(self):
        async def failing():
            yield b"1\tone\n"
            raise RuntimeError("source failed")

        async with self.pool.connection() as connection:
            with self.assertRaises(p.DatabaseError) as aborted:
                async with connection.transaction() as tx:
                    with self.assertRaises(RuntimeError):
                        await tx.copy_in(p.CopyFrom(self.items, "text").columns("id", "name"), failing())
                    self.assertFalse(tx.closed)
                    await tx.execute(p.insert(self.items).columns("id", "name").values(2, "two"))
            self.assertEqual(aborted.exception.sqlstate, "25P02")
            self.assertEqual(await self.stored(connection), [])

    # [spec:pgorm:req:bindings.copy/test]    8 MB streamed lazily in, then counted out chunk by chunk
    async def test_large_lazy_source_streams_both_ways(self):
        rows, width = 80_000, 100
        expected = sum(len(f"{n}\t{'x' * width}\n") for n in range(rows))
        self.assertGreater(expected, 8 * 1024 * 1024)
        progress = []

        async def generate():
            for n in range(rows):
                if n == rows * 3 // 4:
                    # The server has consumed data before the source is exhausted.
                    record = await self.pool.one(p.RawSQL(
                        "SELECT coalesce(max(bytes_processed), 0) AS done FROM pg_stat_progress_copy"
                    ))
                    progress.append(record["done"])
                yield f"{n}\t{'x' * width}\n".encode()

        statement = p.CopyFrom(self.mirror, "text").columns("id", "name")
        self.assertEqual(await self.pool.copy_in(statement, generate()), rows)
        self.assertGreater(progress[0], 0)
        total = chunks = 0
        query = p.select(p.col("id"), p.col("name")).from_(self.mirror).order_by(p.col("id").asc())
        async with await self.pool.copy_out(p.CopyTo(query, "text")) as stream:
            async for chunk in stream:
                self.assertIsInstance(chunk, bytes)
                self.assertLessEqual(len(chunk), 64 * 1024 + width + 16)
                total += len(chunk)
                chunks += 1
        self.assertEqual(total, expected)
        self.assertGreater(chunks, 100)
        self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:bindings.copy/test]    COPY TO over a query with a WHERE value, and each write's RETURNING
    async def test_copy_to_query_with_where_value(self):
        await self.pool.copy_in(p.CopyFrom(self.items, "csv").columns("id", "name"), b"1,a\n2,b\n3,c\n")
        query = (
            p.select(p.col("id"), p.col("name")).from_(self.items)
            .where(p.col("name") != p.bind("b")).order_by(p.col("id").desc())
        )
        self.assertEqual(await collect(await self.pool.copy_out(p.CopyTo(query, "csv"))), b"3,c\n1,a\n")
        write = p.insert(self.items).columns("id", "name").values(4, "d").returning(p.col("id"))
        self.assertEqual(await collect(await self.pool.copy_out(p.CopyTo(write, "text"))), b"4\n")
        update = (p.update(self.items).set("score", 0).where(p.col("id") == 1)
                  .returning(p.col("id"), p.col("score")))
        self.assertEqual(await collect(await self.pool.copy_out(p.CopyTo(update, "csv"))), b"1,0\n")
        delete = p.delete(self.items).where(p.col("id") == 3).returning(p.col("name"))
        self.assertEqual(await collect(await self.pool.copy_out(p.CopyTo(delete, "text"))), b"c\n")
        merge = (p.merge(self.items, self.mirror, self.items.col("id") == self.mirror.col("id"))
                 .when_not_matched(p.MergeAction.insert("id", self.mirror.col("id"))
                                   .set("name", self.mirror.col("name")))
                 .returning_action().returning(self.items.col("id")))
        await self.pool.execute(p.insert(self.mirror).columns("id", "name").values(5, "e"))
        self.assertEqual(await collect(await self.pool.copy_out(p.CopyTo(merge, "text"))), b"INSERT\t5\n")
        with self.assertRaises(p.ConstructionError):
            p.CopyTo(p.delete(self.items).where(p.col("id") == 4), "text")
        self.assertEqual([row[0] for row in await self.stored()], [1, 2, 4, 5])

    # [spec:pgorm:req:bindings.copy/test]    a Select source's refused tie limit or window is refused before COPY TO runs
    async def test_copy_to_checks_select_source(self):
        await self.pool.copy_in(p.CopyFrom(self.items, "csv").columns("id", "name", "score"),
                                b"1,a,3\n2,b,3\n3,c,4\n")
        tied = p.select(p.col("id")).from_(self.items).limit_with_ties(1)
        windowed = p.Select(p.call("count", p.col("id")).over("w")).from_(self.items)
        refused = [(tied, "ORDER BY"), (windowed, '"w"')]
        for query, reason in refused:
            for format in ("text", "csv", "binary"):
                with self.subTest(reason=reason, format=format):
                    with self.assertRaises(p.ConstructionError) as caught:
                        p.CopyTo(query, format)
                    self.assertIn(reason, str(caught.exception))
        ordered = tied.order_by(p.col("score").asc())
        out = await collect(await self.pool.copy_out(p.CopyTo(ordered, "text")))
        # Both rows tie on the one key, so their order is the server's.
        self.assertEqual(sorted(out.split(b"\n")), [b"", b"1", b"2"])
        declared = windowed.window("w", p.Window())
        self.assertEqual(await collect(await self.pool.copy_out(p.CopyTo(declared, "csv"))), b"3\n3\n3\n")

    # [spec:pgorm:req:bindings.copy/test]    a copy in progress reserves its connection or transaction
    async def test_active_copy_reserves_connection_and_transaction(self):
        refused = []
        async with self.pool.connection() as connection:
            async def source():
                yield b"1\tone\n"
                for attempt in (connection.ping, lambda: connection.execute(self.rows())):
                    try:
                        await attempt()
                    except p.LifecycleError as error:
                        refused.append(error)
                yield b"2\ttwo\n"

            statement = p.CopyFrom(self.items, "text").columns("id", "name")
            self.assertEqual(await connection.copy_in(statement, source()), 2)
            self.assertEqual(len(refused), 2)
            stream = await connection.copy_out(p.CopyTo(self.items, "csv"))
            with self.assertRaises(p.LifecycleError):
                await connection.query(self.rows())
            with self.assertRaises(p.LifecycleError):
                await connection.copy_out(p.CopyTo(self.items, "csv"))
            self.assertEqual((await collect(stream)).count(b"\n"), 2)
            self.assertTrue(await connection.ping())
            async with connection.transaction() as tx:
                stream = await tx.copy_out(p.CopyTo(self.items, "text"))
                with self.assertRaises(p.LifecycleError):
                    await tx.query(self.rows())
                with self.assertRaises(p.LifecycleError):
                    async with tx.transaction():
                        self.fail("a savepoint opened under an active COPY")
                self.assertEqual((await collect(stream)).count(b"\n"), 2)
                self.assertEqual(len(await tx.query(self.rows())), 2)
                self.assertEqual(await tx.copy_in(statement, b"3\tthree\n"), 1)
            self.assertEqual(len(await connection.query(self.rows())), 3)

    # [spec:pgorm:req:bindings.copy/test]    a pool copy holds its own connection until the end
    async def test_pool_copy_owns_and_releases_its_connection(self):
        pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1, acquire_timeout=0.5)
        try:
            data = b"".join(f"{n},n\n".encode() for n in range(1, 101))
            self.assertEqual(await pool.copy_in(p.CopyFrom(self.items, "csv").columns("id", "name"), data), 100)
            stream = await pool.copy_out(p.CopyTo(self.items, "csv"))
            with self.assertRaises(p.TimeoutError):
                await pool.acquire()
            self.assertEqual((await collect(stream)).count(b"\n"), 100)
            self.assertTrue(stream.closed)
            self.assertTrue(await pool.ping())
        finally:
            await pool.close()

    # [spec:pgorm:req:bindings.copy/test]    closing COPY TO early discards its connection
    async def test_early_close_discards_connection(self):
        await self.pool.copy_in(
            p.CopyFrom(self.items, "text").columns("id", "name"),
            (f"{n}\t{'y' * 200}\n".encode() for n in range(1, 50001)),
        )
        async with self.pool.connection() as connection:
            async with await connection.copy_out(p.CopyTo(self.items, "text")) as stream:
                self.assertTrue((await anext(stream)).startswith(b"1\t"))
            self.assertTrue(connection.closed)
        async with self.pool.connection() as connection:
            with self.assertRaises(p.LifecycleError):
                async with connection.transaction() as tx:
                    stream = await tx.copy_out(p.CopyTo(self.items, "text"))
                    await anext(stream)
                    await stream.aclose()
                    self.assertTrue(tx.closed)
                    self.assertTrue(connection.closed)
        self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:bindings.copy/test]    a refused COPY FROM raises 42P01 and the pool lends a working connection
    async def test_refused_copy_in_closes_its_connection(self):
        missing = p.Table("python_copy_missing")
        async with self.pool.connection() as connection:
            with self.assertRaises(p.DatabaseError) as caught:
                await connection.copy_in(p.CopyFrom(missing, "text"), b"1\n")
            self.assertEqual(caught.exception.sqlstate, "42P01")
            self.assertTrue(connection.closed)
        with self.assertRaises(p.DatabaseError) as caught:
            await self.pool.copy_in(p.CopyFrom(self.items, "csv").columns("absent"), b"1\n")
        self.assertEqual(caught.exception.sqlstate, "42703")
        async with self.pool.connection() as connection:
            with self.assertRaises(p.DatabaseError) as caught:
                async with connection.transaction() as tx:
                    await tx.copy_in(p.CopyFrom(missing, "text"), b"1\n")
            self.assertEqual(caught.exception.sqlstate, "42P01")
            self.assertTrue(tx.closed)
        async with self.pool.connection() as connection:
            with self.assertRaises(p.DatabaseError) as caught:
                await connection.copy_out(p.CopyTo(missing, "text"))
            self.assertEqual(caught.exception.sqlstate, "42P01")
            self.assertFalse(connection.closed)
            self.assertTrue(await connection.ping())
        for _ in range(3):
            self.assertEqual((await self.pool.one(p.RawSQL("SELECT 1 AS one")))["one"], 1)

    # [spec:pgorm:req:bindings.copy/test]    cancelling a copy during a slow source commits nothing
    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_cancelled_copy_commits_nothing(self):
        statement = p.CopyFrom(self.items, "text").columns("id", "name")
        for in_transaction in (False, True):
            pulled = asyncio.Event()

            async def slow():
                yield b"".join(f"{n}\tname\n".encode() for n in range(1, 10001))
                pulled.set()
                await asyncio.Event().wait()

            async def work():
                if in_transaction:
                    async with self.pool.transaction() as tx:
                        await tx.copy_in(statement, slow())
                else:
                    await self.pool.copy_in(statement, slow())

            with self.subTest(in_transaction=in_transaction):
                task = asyncio.create_task(work())
                await asyncio.wait_for(pulled.wait(), 5)
                task.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await asyncio.wait_for(task, 2)
                self.assertEqual(await self.stored(), [])
                self.assertTrue(await asyncio.wait_for(self.pool.ping(), 2))


    # [spec:pgorm:req:bindings.copy/test]    a copy cancelled while finishing discards its connection
    # [spec:pgorm:req:python.cancellation+1/test]
    async def test_cancel_during_finish_discards_connection(self):
        await self.pool.execute(p.RawSQL(
            "CREATE OR REPLACE FUNCTION python_copy_slow() RETURNS trigger LANGUAGE plpgsql "
            "AS $$ BEGIN PERFORM pg_sleep(1.5); RETURN NULL; END $$"
        ))
        await self.pool.execute(p.RawSQL(
            f'CREATE TRIGGER python_copy_slow AFTER INSERT ON "{MIRROR}" '
            "FOR EACH STATEMENT EXECUTE FUNCTION python_copy_slow()"
        ))
        pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        try:
            connection = await pool.acquire()
            before = (await connection.one(p.RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]
            statement = p.CopyFrom(self.mirror, "text").columns("id", "name")
            # The statement trigger sleeps after the last row, inside finish.
            task = asyncio.create_task(connection.copy_in(statement, b"1\tone\n"))
            await asyncio.sleep(0.5)
            self.assertFalse(task.done())
            task.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await asyncio.wait_for(task, 1)

            async def discarded():
                while not connection.closed:
                    await asyncio.sleep(0.02)

            await asyncio.wait_for(discarded(), 5)
            after = (await pool.one(p.RawSQL("SELECT pg_backend_pid() AS pid")))["pid"]
            self.assertNotEqual(before, after)
        finally:
            await pool.close()


KINDS = (
    "CREATE TABLE python_copy_kinds (id int4 PRIMARY KEY, c_bool bool, c_int2 int2, "
    "c_int8 int8, c_float8 float8, c_text text, c_bytea bytea, c_jsonb jsonb, c_uuid uuid, "
    "c_date date, c_time time, c_timestamp timestamp, c_timestamptz timestamptz, "
    "c_numeric numeric, c_interval interval, c_int4range int4range, c_int4_array int4[], "
    "c_text_array text[], c_mood python_copy_mood, c_positive python_copy_positive, c_null text)"
)

# The values `kind_row` writes, as SQL literals the server parses itself.
LITERALS = (
    "INSERT INTO python_copy_kinds VALUES (2, true, -7, 9000000000, -2.25, 'it''s ü ☃', "
    "'\\x0001ff', '{\"k\": \"v\", \"n\": [1, null]}', "
    "'67e55044-10b1-426f-9247-bb680e5fe0c8', '2026-10-19', '23:59:58.123456', "
    "'1999-12-31 23:00:00', '2023-11-14 22:13:20Z', 12345.6789, "
    "'1 year 2 mons 3 days 00:00:04.000005', '[1,5)', '{1,NULL,3}', '{a,\"b,c\"}', "
    "'loud', 7, NULL)"
)


def kind_row(id):
    """One value of each kind, as Python writes a parameter of it."""
    return [
        id, True, p.Value(-7, "i16"), 9_000_000_000, -2.25, "it's ü ☃", b"\x00\x01\xff",
        p.Value({"k": "v", "n": [1, None]}, "json"),
        UUID("67e55044-10b1-426f-9247-bb680e5fe0c8"),
        dt.date(2026, 10, 19), dt.time(23, 59, 58, 123456), dt.datetime(1999, 12, 31, 23),
        p.Value(dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc), "datetime_utc"),
        Decimal("12345.6789"), p.Interval(14, 3, 4_000_005),
        p.Value(p.Range(1, 5), "int4range"), p.Value.array("i32", [1, None, 3]),
        p.Value.array("text", ["a", "b,c"]), "loud", 7, None,
    ]


async def rows_of(stream):
    async with stream:
        return [row async for row in stream]


# The multiranges created beside a range over float8 and one over int4 named
# apart from its range, which has no canonical function and so keeps [1,5] as
# written; a domain over one; and a table of them and an array of one.
MULTISPANS = (
    "CREATE SCHEMA python_copy_geo",
    "CREATE TYPE python_copy_geo.floatrange AS RANGE (subtype = float8)",
    'CREATE TYPE python_copy_geo.slot AS RANGE (subtype = int4, multirange_type_name = python_copy_geo."Slot Set")',
    "CREATE DOMAIN python_copy_geo.reaches AS python_copy_geo.floatmultirange",
    "CREATE TABLE python_copy_geo.spans (id int PRIMARY KEY, reach python_copy_geo.floatmultirange NOT NULL, "
    'slots python_copy_geo."Slot Set", kept python_copy_geo.reaches)',
    "INSERT INTO python_copy_geo.spans VALUES (1, '{[1.5,2.5), [3,4]}', '{[1,5], [7,9)}', '{[0,1)}'), "
    "(2, '{}', NULL, NULL), (3, '{[-Infinity,0]}', '{(,7)}', '{}')",
    'CREATE TABLE python_copy_geo.many (slots python_copy_geo."Slot Set"[])',
    """INSERT INTO python_copy_geo.many VALUES ('{"{[1,3)}"}')""",
)


class CopyRows(unittest.IsolatedAsyncioTestCase):
    async def drop(self):
        for statement in (
            "DROP TABLE IF EXISTS python_copy_kinds, python_copy_rows, python_copy_points",
            "DROP TYPE IF EXISTS python_copy_mood",
            "DROP DOMAIN IF EXISTS python_copy_positive",
            "DROP SCHEMA IF EXISTS python_copy_geo CASCADE",
        ):
            await self.pool.execute(p.RawSQL(statement))

    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=3)
        await self.drop()
        await self.pool.execute(p.RawSQL("CREATE TYPE python_copy_mood AS ENUM ('calm', 'loud')"))
        await self.pool.execute(p.RawSQL(
            "CREATE DOMAIN python_copy_positive AS int4 CHECK (VALUE > 0)"
        ))
        await self.pool.execute(p.RawSQL(KINDS))
        await self.pool.execute(p.RawSQL(
            "CREATE TABLE python_copy_rows (id int PRIMARY KEY, name text, qty int NOT NULL DEFAULT 7)"
        ))
        self.kinds = p.Table("python_copy_kinds")
        self.rows = p.Table("python_copy_rows")

    async def asyncTearDown(self):
        if self.pool.closed:
            self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=3)
        await self.drop()
        await asyncio.wait_for(self.pool.close(), 2)

    # [spec:pgorm:req:bindings.copy-rows+1/test]    each value kind is written as a parameter binds and read back as a result decodes
    async def test_rows_round_trip_every_kind(self):
        operations = p.capabilities()["operations"]
        for name in ("connection.copy_in_rows", "transaction.copy_out_rows"):
            self.assertIn("CopyTrait", operations[name]["rust_api"])
        self.assertEqual(await self.pool.copy_in_rows(p.CopyFrom(self.kinds, "binary"), [kind_row(1)]), 1)
        await self.pool.execute(p.RawSQL(LITERALS))
        same = await self.pool.one(p.RawSQL(
            "SELECT (to_jsonb(a) - 'id') = (to_jsonb(b) - 'id') AS same "
            "FROM python_copy_kinds a, python_copy_kinds b WHERE a.id = 1 AND b.id = 2"
        ))
        self.assertTrue(same["same"])
        query = p.select(self.kinds.star()).from_(self.kinds).order_by(self.kinds.col("id").asc())
        stream = await self.pool.copy_out_rows(p.CopyTo(query, "binary"))
        self.assertEqual(stream.columns[:3], ("id", "c_bool", "c_int2"))
        self.assertEqual(len(stream.columns), 21)
        rows = await rows_of(stream)
        records = await self.pool.query(p.RawSQL("SELECT * FROM python_copy_kinds ORDER BY id"))
        self.assertEqual(rows, [tuple(record.values()) for record in records])
        self.assertEqual(rows[0][1:4], (True, -7, 9_000_000_000))
        self.assertEqual(rows[0][18], "loud")
        self.assertIsNone(rows[0][20])

    # [spec:pgorm:req:python.truth-testing+4/test]    a stream's truth would not say whether rows remain
    async def test_copy_row_stream_refuses_truth_testing(self):
        await self.pool.copy_in_rows(p.CopyFrom(self.rows, "binary").columns("id", "name"), [(1, "a")])
        stream = await self.pool.copy_out_rows(p.CopyTo(self.rows, "binary"))
        with self.assertRaises(p.ConstructionError):
            bool(stream)
        self.assertEqual(await rows_of(stream), [(1, "a", 7)])
        with self.assertRaises(p.ConstructionError):
            bool(stream)

    # [spec:pgorm:req:bindings.copy-rows+1/test]    a row of the wrong width or type raises a construction error that aborts the COPY
    # [spec:pgorm:req:bindings.argument-errors+1/test]
    async def test_wrong_width_or_type_aborts_copy(self):
        copy = p.CopyFrom(self.rows, "binary").columns("id", "name")
        refusals = [
            ([[1, "one"], [2]], "1 values for the COPY's 2 columns (id, name)"),
            ([[1, "one", 3]], "3 values for the COPY's 2 columns"),
            ([["one", "one"]], "COPY column `id` of type `int4`"),
            ([[1, Decimal("1.5")]], "COPY column `name` of type `text`"),
        ]
        for rows, reason in refusals:
            with self.subTest(reason=reason), self.assertRaises(p.ConstructionError) as caught:
                await self.pool.copy_in_rows(copy, rows)
            self.assertIn(reason, str(caught.exception))
        for rows in ([[1, "a"], "bc"], [[1, "a"], 5]):
            with self.subTest(rows=rows), self.assertRaises(TypeError) as caught:
                await self.pool.copy_in_rows(copy, rows)
            self.assertIn("sequence", str(caught.exception))
        for source in ("rows", b"rows", 5):
            with self.subTest(source=source), self.assertRaises(TypeError):
                await self.pool.copy_in_rows(copy, source)
        with self.assertRaisesRegex(TypeError, "binary"):
            await self.pool.copy_in_rows(p.CopyFrom(self.rows, "text"), [[1, "a"]])
        with self.assertRaisesRegex(TypeError, "binary"):
            await self.pool.copy_out_rows(p.CopyTo(self.rows, "csv"))
        count = await self.pool.one(p.RawSQL("SELECT count(*) AS n FROM python_copy_rows"))
        self.assertEqual(count["n"], 0)

        self.assertEqual(await self.pool.copy_in_rows(copy, [(1, "one"), [2, None]]), 2)
        rows = await rows_of(await self.pool.copy_out_rows(p.CopyTo(self.rows, "binary")))
        self.assertEqual(sorted(rows), [(1, "one", 7), (2, None, 7)])
        with self.assertRaises(p.DatabaseError) as caught:
            await self.pool.copy_in_rows(copy, [[1, "again"]])
        self.assertEqual(caught.exception.sqlstate, "23505")

    # [spec:pgorm:req:bindings.copy-rows+1/test]    a money column, an array of money and a domain over
    # money copy in from a Decimal or an int of whole units, rounded to the currency's places, and
    # copy out as the Decimal the server's numeric cast gives
    async def test_money_rows_move_as_decimals(self):
        await self.pool.execute(p.RawSQL(
            "CREATE DOMAIN python_copy_takings AS money CHECK (VALUE >= 0::money)"))
        await self.pool.execute(p.RawSQL(
            "CREATE TABLE python_copy_tills (id int PRIMARY KEY, cash money, floats money[], "
            "banked python_copy_takings)"))
        try:
            tills = p.Table("python_copy_tills")
            rows = [
                (1, Decimal("12.345"), [Decimal("-0.005"), None, Decimal("3")], 40),
                (2, None, None, None),
            ]
            self.assertEqual(await self.pool.copy_in_rows(p.CopyFrom(tills, "binary"), rows), 2)
            stored = await self.pool.one(p.RawSQL(
                "SELECT cash::numeric AS cash, floats::numeric[] AS floats, banked::numeric AS banked "
                "FROM python_copy_tills WHERE id = 1"))
            self.assertEqual(
                (stored["cash"], stored["floats"], stored["banked"]),
                (Decimal("12.35"), [Decimal("-0.01"), None, Decimal("3.00")], Decimal("40.00")))
            query = p.select(tills.star()).from_(tills).order_by(tills.col("id").asc())
            read = await rows_of(await self.pool.copy_out_rows(p.CopyTo(query, "binary")))
            self.assertEqual(read, [
                (1, Decimal("12.35"), [Decimal("-0.01"), None, Decimal("3.00")], Decimal("40.00")),
                (2, None, None, None),
            ])
            with self.assertRaises(p.ConstructionError) as caught:
                await self.pool.copy_in_rows(p.CopyFrom(tills, "binary"), [(3, Decimal("100000000000000000000"), None, None)])
            self.assertIn("COPY column `cash` of type `money`", str(caught.exception))
            with self.assertRaises(p.DatabaseError) as caught:
                await self.pool.copy_in_rows(p.CopyFrom(tills, "binary"), [(3, None, None, Decimal("-1"))])
            self.assertEqual(caught.exception.sqlstate, "23514")
        finally:
            await self.pool.execute(p.RawSQL("DROP TABLE IF EXISTS python_copy_tills"))
            await self.pool.execute(p.RawSQL("DROP DOMAIN IF EXISTS python_copy_takings"))

    # [spec:pgorm:req:bindings.copy-rows+1/test]    rows run on a connection and in a transaction, from an async generator
    async def test_rows_on_connection_and_transaction(self):
        copy = p.CopyFrom(self.rows, "binary").columns("id", "name")

        async def generated():
            for id in range(1, 601):
                yield (id, f"n{id}")

        async with self.pool.connection() as connection:
            self.assertEqual(await connection.copy_in_rows(copy, generated()), 600)
            self.assertEqual(await connection.copy_in_rows(copy, []), 0)
            ids = p.select(self.rows.col("id")).from_(self.rows)
            self.assertEqual(len(await rows_of(await connection.copy_out_rows(p.CopyTo(ids, "binary")))), 600)
            with self.assertRaises(RolledBack):
                async with connection.transaction() as transaction:
                    self.assertEqual(await transaction.copy_in_rows(copy, [[601, "late"]]), 1)
                    returning = (p.insert(self.rows).columns("id", "name").values(602, "returned")
                                 .returning(self.rows.col("qty"), self.rows.col("name")))
                    stream = await transaction.copy_out_rows(p.CopyTo(returning, "binary"))
                    self.assertEqual(stream.columns, ("qty", "name"))
                    self.assertEqual(await rows_of(stream), [(7, "returned")])
                    count = await transaction.one(p.RawSQL("SELECT count(*) AS n FROM python_copy_rows"))
                    self.assertEqual(count["n"], 602)
                    raise RolledBack
            count = await connection.one(p.RawSQL("SELECT count(*) AS n FROM python_copy_rows"))
            self.assertEqual(count["n"], 600)

    # [spec:pgorm:req:bindings.copy-rows+1/test]    a refused description keeps the connection; an undecodable field raises and closes the stream
    async def test_refused_description_and_undecodable_field(self):
        async with self.pool.connection() as connection:
            with self.assertRaises(p.DatabaseError) as caught:
                await connection.copy_in_rows(p.CopyFrom(p.Table("python_copy_missing"), "binary"), [[1]])
            self.assertEqual(caught.exception.sqlstate, "42P01")
            self.assertEqual((await connection.one(p.RawSQL("SELECT 1 AS n")))["n"], 1)
            with self.assertRaises(p.DatabaseError):
                async with connection.transaction() as transaction:
                    await transaction.copy_out_rows(p.CopyTo(p.Table("python_copy_missing"), "binary"))
            self.assertEqual((await connection.one(p.RawSQL("SELECT 2 AS n")))["n"], 2)

        await self.pool.execute(p.RawSQL(
            "CREATE TABLE python_copy_points AS SELECT point(n, n) AS spot FROM generate_series(1, 3) AS n"
        ))
        stream = await self.pool.copy_out_rows(p.CopyTo(p.Table("python_copy_points"), "binary"))
        with self.assertRaises(p.DecodeError):
            await anext(stream)
        self.assertTrue(stream.closed)
        with self.assertRaises(StopAsyncIteration):
            await anext(stream)
        self.assertEqual((await self.pool.one(p.RawSQL("SELECT 3 AS n")))["n"], 3)

    # [spec:pgorm:req:bindings.copy-rows+1/test]    a created multirange reads as a Multirange and writes back from a Value of its kind or from its text
    # [spec:pgorm:req:bindings.created-ranges+5/test]
    async def test_created_multiranges_round_trip(self):
        for statement in MULTISPANS:
            await self.pool.execute(p.RawSQL(statement))
        spans = p.Table("spans", schema="python_copy_geo")
        reach = p.CreatedMultirange("floatmultirange", "f64", schema="python_copy_geo")
        slots = p.CreatedMultirange("Slot Set", "i32", schema="python_copy_geo")
        ordered = p.CopyTo(p.select(spans.star()).from_(spans).order_by(spans.col("id").asc()), "binary")

        stream = await self.pool.copy_out_rows(ordered)
        self.assertEqual(stream.columns, ("id", "reach", "slots", "kept"))
        rows = await rows_of(stream)
        self.assertEqual(rows, [
            (1, p.Multirange([p.Range(1.5, 2.5), p.Range(3.0, 4.0, "[]")]),
             p.Multirange([p.Range(1, 5, "[]"), p.Range(7, 9)]), p.Multirange([p.Range(0.0, 1.0)])),
            (2, p.Multirange([]), None, None),
            (3, p.Multirange([p.Range(float("-inf"), 0.0, "[]")]), p.Multirange([p.Range(None, 7, "()")]),
             p.Multirange([])),
        ])
        # A query's column of one has no subtype the driver reports, so only a
        # COPY, whose description completes it, reads one.
        with self.assertRaises(p.DecodeError):
            await self.pool.one(p.RawSQL("SELECT reach FROM python_copy_geo.spans WHERE id = 1"))

        written = [
            (11, p.Value(rows[0][1], reach), p.Value(rows[0][2], slots), p.Value(rows[0][3], reach)),
            (12, p.Value(rows[1][1], reach), None, None),
            (13, "{[-Infinity,0]}", "{(,7)}", "{}"),
        ]
        self.assertEqual(await self.pool.copy_in_rows(p.CopyFrom(spans, "binary"), written), 3)
        compared = await self.pool.one(p.RawSQL(
            "SELECT count(*) AS n FROM python_copy_geo.spans a JOIN python_copy_geo.spans b ON b.id = a.id + 10 "
            "WHERE a.reach = b.reach AND a.slots IS NOT DISTINCT FROM b.slots "
            "AND a.kept::python_copy_geo.floatmultirange IS NOT DISTINCT FROM b.kept::python_copy_geo.floatmultirange"
        ))
        self.assertEqual(compared["n"], 3)

        copy = p.CopyFrom(spans, "binary").columns("id", "reach")
        for text in ("{[a,b)}", "[1,2)"):
            with self.subTest(text=text), self.assertRaises(p.ConstructionError) as caught:
                await self.pool.copy_in_rows(copy, [(20, text)])
            self.assertIn("COPY column `reach` of type `python_copy_geo.floatmultirange`", str(caught.exception))
        count = await self.pool.one(p.RawSQL("SELECT count(*) AS n FROM python_copy_geo.spans"))
        self.assertEqual(count["n"], 6)

        # An array of one reads and writes as an array of its kind, its elements completed as the column's are.
        many = p.Table("many", schema="python_copy_geo")
        self.assertEqual(await rows_of(await self.pool.copy_out_rows(p.CopyTo(many, "binary"))),
                         [([p.Multirange([p.Range(1, 3)])],)])
        array = p.Value.array(slots, [p.Multirange([p.Range(4, 6)]), None])
        self.assertEqual(await self.pool.copy_in_rows(p.CopyFrom(many, "binary"), [(array,), (["{[8,9)}"],)]), 2)
        read = await rows_of(await self.pool.copy_out_rows(p.CopyTo(many, "binary")))
        self.assertEqual(read[1:], [([p.Multirange([p.Range(4, 6)]), None],), ([p.Multirange([p.Range(8, 9)])],)])


if __name__ == "__main__":
    unittest.main()
