"""Native PostgreSQL resource tests; require PGORM_TEST_DSN for a disposable DB."""

import asyncio
from contextlib import contextmanager
from decimal import Decimal
import os
from pathlib import Path
import tempfile
import unittest
from urllib.parse import urlsplit, urlunsplit, quote

import pgorm


TRUST_STORE_VARIABLES = ("SSL_CERT_FILE", "SSL_CERT_DIR")


@contextmanager
def trust_store(**variables):
    """Run with SSL_CERT_FILE and SSL_CERT_DIR exactly as given, unset otherwise.

    With neither given the pool reads the platform's own certificate store;
    either one replaces that store entirely.
    """
    saved = {name: os.environ.get(name) for name in TRUST_STORE_VARIABLES}
    try:
        for name in TRUST_STORE_VARIABLES:
            os.environ.pop(name, None)
        os.environ.update({name.upper(): str(value) for name, value in variables.items()})
        yield
    finally:
        for name, value in saved.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value


class RuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.dsn = os.environ["PGORM_TEST_DSN"]
        self.pool = pgorm.Pool(self.dsn, max_size=1, acquire_timeout=0.2)

    async def asyncTearDown(self):
        await self.pool.close()

    # [spec:pgorm:req:bindings.runtime/test]
    # [spec:pgorm:req:python.lifecycle+4/test]
    # [spec:pgorm:req:bindings.connections+3/test]
    async def test_connection_reuse_and_context_cleanup(self):
        self.assertTrue(await self.pool.ping())
        self.assertEqual(self.pool.status().size, 1)
        async with self.pool.connection() as connection:
            self.assertTrue(await connection.ping())
            self.assertEqual(self.pool.status().available, 0)
        self.assertTrue(connection.closed)
        self.assertEqual(self.pool.status().available, 1)
        self.assertTrue(await self.pool.ping())
        self.assertEqual(self.pool.status().size, 1)

    # [spec:pgorm:req:python.runtime+2/test]
    async def test_acquisition_does_not_block_asyncio(self):
        async with self.pool.connection():
            waiting = asyncio.create_task(self.pool.acquire())
            ticks = 0
            for _ in range(5):
                await asyncio.sleep(0.01)
                ticks += 1
            self.assertEqual(ticks, 5)
            self.assertFalse(waiting.done())
            waiting.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await waiting
        self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:python.runtime+2/test]
    # [spec:pgorm:req:bindings.errors+6/test]    a panic in native work is an InternalError, never left pending
    async def test_native_panic_is_internal_error(self):
        probe = getattr(pgorm._native, "_probe_panic", None)
        if probe is None:
            self.skipTest("the probe exists only in debug builds")
        with self.assertRaises(pgorm.InternalError) as caught:
            await asyncio.wait_for(probe(), 5)
        self.assertIsInstance(caught.exception, pgorm.PgormError)
        self.assertEqual(str(caught.exception), "pgorm panicked: the probe's deliberate panic")
        self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:bindings.connections+3/test]
    async def test_shutdown_releases_checked_out_resources_and_waiters(self):
        connection = await self.pool.acquire()
        waiting = asyncio.create_task(self.pool.acquire())
        await asyncio.sleep(0.02)
        await asyncio.wait_for(self.pool.close(), 1)
        with self.assertRaises(pgorm.LifecycleError):
            await waiting
        self.assertTrue(connection.closed)
        with self.assertRaises(pgorm.LifecycleError):
            await connection.ping()
        with self.assertRaises(pgorm.LifecycleError):
            await self.pool.acquire()
        self.pool.clear_statement_caches()
        await self.pool.close()
        await connection.close()

    # [spec:pgorm:req:python.runtime+2/test]
    async def test_another_event_loop_is_rejected_without_deadlock(self):
        def foreign():
            return asyncio.run(self.pool.ping())
        with self.assertRaises(pgorm.LifecycleError):
            await asyncio.wait_for(asyncio.to_thread(foreign), 1)
        self.assertTrue(await self.pool.ping())

    # [spec:pgorm:req:bindings.errors+6/test]
    async def test_acquisition_timeout_is_distinct_from_cancellation(self):
        async with self.pool.connection():
            with self.assertRaises(pgorm.TimeoutError):
                await self.pool.acquire()
        self.assertTrue(await self.pool.ping())
        self.assertIs(pgorm.CancelledError, asyncio.CancelledError)

    # [spec:pgorm:req:bindings.errors+6/test]
    # [spec:pgorm:req:bindings.connections+3/test]    an empty or overlong
    # statement_cache_channel is a ConstructionError, and one of another type a TypeError
    # [spec:pgorm:req:bindings.argument-errors+1/test]
    async def test_bad_configuration_is_an_explicit_construction_error(self):
        for options in (
            {"max_size": 0}, {"max_size": -1},
            {"max_size": 2**100}, {"statement_cache_size": -1},
            {"acquire_timeout": 0}, {"connect_timeout": float("nan")},
            {"statement_cache_channel": ""}, {"statement_cache_channel": "c" * 64},
            {"ca": "not PEM"}, {"ca": b""},
            {"ca": "x", "cafile": os.environ["PGORM_TEST_CA"]},
        ):
            with self.subTest(options=options):
                with self.assertRaises(pgorm.ConstructionError):
                    pgorm.Pool(self.dsn, **options)
        for options in (
            {"max_size": True}, {"max_size": 1.0}, {"statement_cache_size": "1"},
            {"acquire_timeout": True}, {"connect_timeout": "10"},
            {"tls": "ignore-certificate"}, {"tls": 1}, {"recycle": "invented"},
            {"statement_cache_channel": 1}, {"ca": 1}, {"ca": ["pem"]},
        ):
            with self.subTest(options=options):
                with self.assertRaises(TypeError):
                    pgorm.Pool(self.dsn, **options)
        with self.assertRaises(TypeError):
            pgorm.Pool(b"postgres://")
        secret = "do-not-show-this-password"
        with self.assertRaises(pgorm.ConstructionError) as caught:
            pgorm.Pool(f"unknown-key={secret}")
        self.assertNotIn(secret, str(caught.exception))

    # [spec:pgorm:req:bindings.errors+6/test]    a value its parameter's type refuses is a
    # ConstructionError naming the parameter and carrying pgorm's refusal, as JavaScript reports it
    async def test_bind_refusal_names_parameter_and_reason(self):
        for sql, value, refusal in (
            ("SELECT 1 AS one, $1::integer AS n", pgorm.Value("text", "text"),
             "cannot bind a `String` value to Postgres type `int4`"),
            ("SELECT $1::money AS m", Decimal("1.5"),
             "cannot bind a `Decimal` value to Postgres type `money`"),
        ):
            with self.subTest(sql=sql):
                with self.assertRaises(pgorm.ConstructionError) as refused:
                    await self.pool.one(pgorm.RawSQL(sql, [value]))
                self.assertEqual(str(refused.exception), f"parameter $1 cannot be bound: {refusal}")

    # [spec:pgorm:req:bindings.errors+6/test]
    async def test_sqlstate_and_credential_redaction(self):
        parts = urlsplit(self.dsn)
        password = "deliberately-invalid-private-password"
        netloc = f"{parts.username}:{quote(password)}@{parts.hostname}:{parts.port}"
        dsn = urlunsplit((parts.scheme, netloc, parts.path, parts.query, parts.fragment))
        pool = pgorm.Pool(dsn)
        try:
            self.assertNotIn(password, repr(pool))
            self.assertNotIn(password, repr(pool._native))
            with self.assertRaises(pgorm.PgormError) as caught:
                await pool.ping()
            self.assertEqual(caught.exception.sqlstate, "28P01")
            self.assertNotIn(password, repr(caught.exception))
            self.assertNotIn(parts.username, caught.exception.message)
        finally:
            await pool.close()

    # [spec:pgorm:req:bindings.connections+3/test]
    async def test_tls_never_falls_back_to_plaintext(self):
        pool = pgorm.Pool(self.dsn, tls="verify-full")
        try:
            with self.assertRaises(pgorm.PgormError):
                await pool.ping()
        finally:
            await pool.close()

    # [spec:pgorm:req:bindings.connections+3/test]
    async def test_platform_store_refuses_an_unknown_ca(self):
        # The test server's certificate is signed by a CA minted for this run,
        # which no platform store holds: without cafile it must be refused.
        # Connection failures are reported without their cause, so the control
        # is the next test: the same pool, with that CA in the store, connects.
        with trust_store():
            pool = pgorm.Pool(self.dsn, tls="verify-full")
        try:
            with self.assertRaises(pgorm.ConnectionError):
                await pool.ping()
        finally:
            await pool.close()

    # [spec:pgorm:req:bindings.connections+3/test]
    async def test_default_trust_is_the_certificate_store(self):
        # SSL_CERT_FILE replaces the platform's store, so naming the test CA
        # there and passing no cafile shows the default trust is the store's.
        with trust_store(ssl_cert_file=os.environ["PGORM_TEST_CA"]):
            pool = pgorm.Pool(self.dsn, tls="verify-full")
        async with pool:
            self.assertTrue(await pool.ping())

    # [spec:pgorm:req:bindings.connections+3/test]
    async def test_rootless_store_fails_construction(self):
        with tempfile.TemporaryDirectory(prefix="pgorm-python-empty-store-") as directory:
            empty_file = Path(directory) / "none.pem"
            empty_file.write_text("")
            empty_dir = Path(directory) / "certs"
            empty_dir.mkdir()
            for variables in (
                {"ssl_cert_file": empty_file},
                {"ssl_cert_dir": empty_dir},
                {"ssl_cert_file": Path(directory) / "missing.pem"},
            ):
                with self.subTest(variables=variables), trust_store(**variables):
                    with self.assertRaises(pgorm.ConstructionError) as caught:
                        pgorm.Pool(self.dsn, tls="verify-full")
                    self.assertIn("no usable root certificate", str(caught.exception))
                    async with pgorm.Pool(
                        self.dsn, tls="verify-full", cafile=os.environ["PGORM_TEST_CA"]
                    ) as pool:
                        self.assertTrue(await pool.ping())
            with trust_store(ssl_cert_file=empty_file):
                async with pgorm.Pool(self.dsn) as pool:
                    self.assertTrue(await pool.ping())

    # [spec:pgorm:req:bindings.connections+3/test]
    # [spec:pgorm:req:python.lifecycle+4/test]    the CA as PEM data or as a file
    async def test_verified_tls_accepts_the_configured_ca(self):
        async with pgorm.Pool(self.dsn, tls="verify-full", cafile=os.environ["PGORM_TEST_CA"]) as pool:
            self.assertTrue(await pool.ping())
        pem = Path(os.environ["PGORM_TEST_CA"]).read_bytes()
        for ca in (pem, pem.decode()):
            with self.subTest(ca=type(ca).__name__):
                async with pgorm.Pool(self.dsn, tls="verify-full", ca=ca) as pool:
                    self.assertTrue(await pool.ping())

    # [spec:pgorm:req:bindings.connections+3/test]
    async def test_verified_tls_binds_scram_to_the_certificate(self):
        # channel_binding=require refuses SCRAM unless it is bound to the TLS
        # session (SCRAM-SHA-256-PLUS over the server certificate's
        # tls-server-end-point hash), so the TLS connector has to report that
        # binding for this to connect. Plaintext has nothing to bind to, and
        # the same setting refuses it.
        parts = urlsplit(self.dsn)
        query = f"{parts.query}&channel_binding=require"
        dsn = urlunsplit((parts.scheme, parts.netloc, parts.path, query, parts.fragment))
        async with pgorm.Pool(dsn, tls="verify-full", cafile=os.environ["PGORM_TEST_CA"]) as pool:
            self.assertTrue(await pool.ping())
        pool = pgorm.Pool(dsn, tls="disable")
        try:
            with self.assertRaises(pgorm.PgormError):
                await pool.ping()
        finally:
            await pool.close()

    # [spec:pgorm:req:bindings.connections+3/test]
    async def test_certificate_hostname_is_verified(self):
        parts = urlsplit(self.dsn)
        netloc = f"{parts.username}:{quote(parts.password)}@wrong.test:{parts.port}"
        query = f"{parts.query}&hostaddr=127.0.0.1"
        dsn = urlunsplit((parts.scheme, netloc, parts.path, query, parts.fragment))
        pool = pgorm.Pool(dsn, tls="verify-full", cafile=os.environ["PGORM_TEST_CA"])
        try:
            with self.assertRaises(pgorm.PgormError):
                await pool.ping()
        finally:
            await pool.close()


class ConstraintViolationTests(unittest.IsolatedAsyncioTestCase):
    """Each constraint class pgorm's SqlError names raises its own DatabaseError."""

    NAMESPACE = "python_violations"

    async def asyncSetUp(self):
        self.pool = pgorm.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        ns = self.NAMESPACE
        for sql in (
            f"DROP SCHEMA IF EXISTS {ns} CASCADE",
            f"CREATE SCHEMA {ns}",
            f"CREATE DOMAIN {ns}.positive AS int4 NOT NULL CONSTRAINT positive_check CHECK (VALUE > 0)",
            f"CREATE TABLE {ns}.parent (id int4 PRIMARY KEY)",
            f"""CREATE TABLE {ns}.gauge (
                id int4 PRIMARY KEY,
                reading int4 NOT NULL CONSTRAINT reading_bounded CHECK (reading < 100),
                level {ns}.positive,
                parent_id int4 CONSTRAINT gauge_parent REFERENCES {ns}.parent,
                span int4range,
                CONSTRAINT gauge_apart EXCLUDE USING gist (span WITH &&)
            )""",
            f"INSERT INTO {ns}.gauge (id, reading, level, span) VALUES (1, 1, 1, '[1,5)')",
        ):
            await self.pool.execute(pgorm.RawSQL(sql))

    async def asyncTearDown(self):
        await self.pool.execute(pgorm.RawSQL(f"DROP SCHEMA {self.NAMESPACE} CASCADE"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.errors+6/test]
    async def test_constraint_violations_raise_their_own_class(self):
        ns = self.NAMESPACE
        insert = f"INSERT INTO {ns}.gauge (id, reading, level, parent_id, span) VALUES "
        cases = (
            (insert + "(1, 1, 1, NULL, NULL)", pgorm.UniqueViolationError, "23505",
             {"table": "gauge", "constraint": "gauge_pkey"}),
            (insert + "(2, 1, 1, 9, NULL)", pgorm.ForeignKeyViolationError, "23503",
             {"table": "gauge", "constraint": "gauge_parent"}),
            (insert + "(2, 100, 1, NULL, NULL)", pgorm.CheckViolationError, "23514",
             {"table": "gauge", "constraint": "reading_bounded"}),
            (insert + "(2, 1, 0, NULL, NULL)", pgorm.CheckViolationError, "23514",
             {"datatype": "positive", "constraint": "positive_check"}),
            (insert + "(2, NULL, 1, NULL, NULL)", pgorm.NotNullViolationError, "23502",
             {"table": "gauge", "column": "reading"}),
            (f"INSERT INTO {ns}.gauge (id, reading) VALUES (2, 1)", pgorm.NotNullViolationError,
             "23502", {"datatype": "positive"}),
            (insert + "(2, 1, 1, NULL, '[4,8)')", pgorm.ExclusionViolationError, "23P01",
             {"table": "gauge", "constraint": "gauge_apart"}),
        )
        classes = {case[1] for case in cases}
        for sql, cls, sqlstate, names in cases:
            with self.subTest(sqlstate=sqlstate, names=names):
                with self.assertRaises(cls) as caught:
                    await self.pool.execute(pgorm.RawSQL(sql))
                error = caught.exception
                self.assertIsInstance(error, pgorm.DatabaseError)
                self.assertIsInstance(error, pgorm.PgormError)
                for other in classes - {cls}:
                    self.assertNotIsInstance(error, other)
                self.assertEqual(error.sqlstate, sqlstate)
                self.assertEqual(error.severity, "ERROR")
                self.assertEqual(str(error), error.message)
                self.assertEqual(
                    {
                        field: getattr(error, field)
                        for field in ("schema", "table", "column", "datatype", "constraint")
                    },
                    {
                        "schema": ns,
                        "table": None,
                        "column": None,
                        "datatype": None,
                        "constraint": None,
                    } | names,
                )

    # [spec:pgorm:req:bindings.errors+6/test]
    async def test_other_errors_stay_plain_database_errors(self):
        with self.assertRaises(pgorm.DatabaseError) as caught:
            await self.pool.execute(pgorm.RawSQL("SELECT 1 / 0"))
        self.assertIs(type(caught.exception), pgorm.DatabaseError)
        self.assertEqual(caught.exception.sqlstate, "22012")
        self.assertIsNone(caught.exception.datatype)
        for cls in (
            pgorm.UniqueViolationError, pgorm.ForeignKeyViolationError, pgorm.CheckViolationError,
            pgorm.NotNullViolationError, pgorm.ExclusionViolationError,
        ):
            self.assertTrue(issubclass(cls, pgorm.DatabaseError))
            self.assertIsNone(cls.datatype)


if __name__ == "__main__":
    unittest.main()
