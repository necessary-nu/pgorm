"""A pool's configuration beyond its size: timeouts, queue mode, recycling,
tags, hooks, discarding, TLS modes and client certificates, several pools and
a listener of no pool. Require PGORM_TEST_DSN, and the TLS fixture's
PGORM_TEST_CA and client certificate, as with_postgres.py provides them."""

import asyncio
import os
from pathlib import Path
import unittest
from urllib.parse import quote, urlsplit, urlunsplit

import pgorm
from pgorm import HookConnection, MultiPool, Pool, RawSQL


PID = RawSQL("SELECT pg_backend_pid() AS pid")


def renamed(dsn: str, host: str) -> str:
    """``dsn`` naming ``host`` while it still connects to 127.0.0.1."""
    parts = urlsplit(dsn)
    netloc = f"{parts.username}:{quote(parts.password or '')}@{host}:{parts.port}"
    return urlunsplit((parts.scheme, netloc, parts.path, f"{parts.query}&hostaddr=127.0.0.1", parts.fragment))


def as_user(dsn: str, user: str) -> str:
    """``dsn`` logging in as ``user``, with no password."""
    parts = urlsplit(dsn)
    return urlunsplit((parts.scheme, f"{user}@{parts.hostname}:{parts.port}", parts.path, parts.query, parts.fragment))


async def pid(pool: Pool) -> int:
    value: int = (await pool.one(PID))["pid"]
    return value


async def ssl(pool: Pool) -> bool:
    row = await pool.one(RawSQL("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()"))
    value: bool = row["ssl"]
    return value


class PoolShapeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.dsn = os.environ["PGORM_TEST_DSN"]

    # [spec:pgorm:req:bindings.connections+3/test]    a wait timeout that runs out is a
    # TimeoutError naming it
    async def test_wait_timeout_names_itself(self) -> None:
        async with Pool(self.dsn, max_size=1, wait_timeout=0.1, acquire_timeout=5) as pool:
            held = await pool.acquire()
            with self.assertRaises(pgorm.TimeoutError) as caught:
                await pool.acquire()
            self.assertEqual(str(caught.exception), "no connection came free within the pool's wait_timeout")
            await held.close()

    # [spec:pgorm:req:bindings.connections+3/test]    a create timeout bounds opening a
    # connection, here to a server that never answers
    async def test_create_timeout_names_itself(self) -> None:
        held: list[asyncio.StreamWriter] = []

        async def silent(_reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
            held.append(writer)

        server = await asyncio.start_server(silent, "127.0.0.1", 0)
        port = server.sockets[0].getsockname()[1]
        pool = Pool(f"postgres://nobody@127.0.0.1:{port}/none?sslmode=disable", create_timeout=0.2)
        try:
            with self.assertRaises(pgorm.TimeoutError) as caught:
                await pool.acquire()
            self.assertEqual(str(caught.exception), "opening a connection outlasted the pool's create_timeout")
        finally:
            await pool.close()
            for writer in held:
                writer.close()
            server.close()
            await server.wait_closed()

    # [spec:pgorm:req:bindings.connections+3/test]    recycle_sql is Custom recycling, and a
    # recycle timeout abandons a check that outlasts it, the pool opening a fresh connection
    async def test_recycle_sql_runs_under_the_recycle_timeout(self) -> None:
        loop = asyncio.get_running_loop()
        async with Pool(self.dsn, max_size=1, recycle_sql="SELECT pg_sleep(5)", recycle_timeout=0.1) as pool:
            first = await pid(pool)
            started = loop.time()
            self.assertNotEqual(await pid(pool), first)
            self.assertLess(loop.time() - started, 4)
        setting = RawSQL("SELECT current_setting('application_name') AS name")
        async with Pool(self.dsn, max_size=1, recycle_sql="SET application_name = 'recycled'") as pool:
            self.assertEqual((await pool.one(setting))["name"], "recycled")

    # [spec:pgorm:req:bindings.connections+3/test]    clean recycling: a setting the last
    # borrower made is gone for the next
    async def test_clean_recycling_resets_the_session(self) -> None:
        seqscan = RawSQL("SELECT current_setting('enable_seqscan') AS value")
        async with Pool(self.dsn, max_size=1, recycle="clean") as pool:
            async with pool.connection() as connection:
                await connection.execute(RawSQL("SET enable_seqscan = off"))
            self.assertEqual((await pool.one(seqscan))["value"], "on")
        with self.assertRaises(TypeError):
            Pool(self.dsn, recycle="fast", recycle_sql="SELECT 1")
        with self.assertRaises(TypeError):
            Pool(self.dsn, recycle_sql=b"SELECT 1")  # type: ignore[arg-type]

    # [spec:pgorm:req:bindings.connections+3/test]    fifo lends the connection returned
    # first, lifo the one returned last
    async def test_queue_mode_orders_the_idle_connections(self) -> None:
        for queue_mode, lent in (("fifo", 0), ("lifo", 1)):
            async with Pool(self.dsn, max_size=2, queue_mode=queue_mode) as pool:
                first, second = await pool.acquire(), await pool.acquire()
                pids = [(await connection.one(PID))["pid"] for connection in (first, second)]
                await first.close()
                await second.close()
                self.assertEqual(await pid(pool), pids[lent], queue_mode)
        with self.assertRaises(TypeError):
            Pool(self.dsn, queue_mode="random")  # type: ignore[arg-type]

    # [spec:pgorm:req:bindings.connections+3/test]    a pool reports its tag, pgorm's
    # default-N without one, and an empty tag is refused
    async def test_a_pool_is_tagged(self) -> None:
        tagged = Pool(self.dsn, tag="kitchen")
        untagged = Pool(self.dsn)
        try:
            self.assertEqual(tagged.tag, "kitchen")
            self.assertRegex(untagged.tag, r"^default-\d+$")
        finally:
            await tagged.close()
            await untagged.close()
        with self.assertRaises(pgorm.ConstructionError):
            Pool(self.dsn, tag="")
        with self.assertRaises(TypeError):
            Pool(self.dsn, tag=1)  # type: ignore[arg-type]

    # [spec:pgorm:req:bindings.connections+3/test]    discard closes the connection rather
    # than return it, and the pool opens a fresh one
    async def test_discard_closes_the_connection(self) -> None:
        async with Pool(self.dsn, max_size=1) as pool:
            connection = await pool.acquire()
            discarded = (await connection.one(PID))["pid"]
            await connection.discard()
            self.assertTrue(connection.closed)
            self.assertNotEqual(await pid(pool), discarded)
            self.assertEqual(pool.status().size, 1)


class InstrumentedConfigurationTests(unittest.IsolatedAsyncioTestCase):
    # [spec:pgorm:req:bindings.connections+3/test]    an instrumented pool wraps a configured
    # pool: its tag and hooks hold, and its connections are discarded as a pool's are
    async def test_an_instrumented_pool_keeps_its_configuration(self) -> None:
        created: list[int] = []
        configured = Pool(
            os.environ["PGORM_TEST_DSN"], max_size=1, tag="kitchen",
            post_create=lambda connection: created.append(connection.recycle_count),
        )
        async with pgorm.InstrumentedPool(configured, pgorm.NoOpMetrics()) as pool:
            self.assertEqual(pool.tag, "kitchen")
            connection = await pool.acquire()
            discarded = (await connection.one(PID))["pid"]
            await connection.discard()
            self.assertTrue(connection.closed)
            self.assertNotEqual(await pid(pool), discarded)
        self.assertEqual(created, [0, 0])
        await configured.close()


class HookTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.dsn = os.environ["PGORM_TEST_DSN"]

    # [spec:pgorm:req:bindings.pool-hooks/test]    post_create runs statements on the
    # connection it opened, before the pool lends it
    async def test_post_create_prepares_the_session(self) -> None:
        seen: list[tuple[float, int]] = []

        async def name_it(connection: HookConnection) -> None:
            await connection.execute(RawSQL("SELECT set_config('application_name', 'hooked', false)"))
            row = await connection.one(RawSQL("SELECT current_setting('application_name') AS name"))
            self.assertEqual(row["name"], "hooked")
            seen.append((connection.age, connection.recycle_count))

        async with Pool(self.dsn, max_size=1, post_create=name_it) as pool:
            row = await pool.one(RawSQL("SELECT current_setting('application_name') AS name"))
            self.assertEqual(row["name"], "hooked")
        self.assertEqual(len(seen), 1)
        self.assertGreaterEqual(seen[0][0], 0)
        self.assertEqual(seen[0][1], 0)

    # [spec:pgorm:req:bindings.pool-hooks/test]    a plain function is a hook too
    async def test_a_plain_function_is_a_hook(self) -> None:
        calls: list[int] = []
        async with Pool(self.dsn, max_size=1, post_create=lambda connection: calls.append(1)) as pool:
            await pool.ping()
        self.assertEqual(calls, [1])
        with self.assertRaises(TypeError):
            Pool(self.dsn, post_create="not callable")  # type: ignore[arg-type]

    # [spec:pgorm:req:bindings.pool-hooks/test]    a post_create hook that raises refuses
    # the connection, and the acquisition fails naming the hook and the error
    async def test_a_raising_post_create_refuses_the_connection(self) -> None:
        async def refuse(connection: HookConnection) -> None:
            raise ValueError("the server is too old")

        pool = Pool(self.dsn, max_size=1, post_create=refuse)
        try:
            with self.assertRaises(pgorm.ConnectionError) as caught:
                await pool.acquire()
            self.assertEqual(
                str(caught.exception),
                "the post_create hook refused the connection: ValueError: the server is too old",
            )
            self.assertEqual(pool.status().size, 0)
        finally:
            await pool.close()

    # [spec:pgorm:req:bindings.pool-hooks/test]    a recycle hook that raises discards the
    # connection, and the pool opens another
    async def test_a_raising_pre_recycle_discards_the_connection(self) -> None:
        recycled: list[int] = []

        async def once(connection: HookConnection) -> None:
            recycled.append(connection.recycle_count)
            if len(recycled) == 1:
                raise RuntimeError("retire it")

        pool = Pool(self.dsn, max_size=1, pre_recycle=once)
        try:
            first = await pid(pool)
            second = await pid(pool)
            third = await pid(pool)
        finally:
            await pool.close()
        self.assertNotEqual(first, second)
        self.assertEqual(second, third)
        self.assertEqual(recycled, [0, 0])

    # [spec:pgorm:req:bindings.pool-hooks/test]    a hook's connection kept past its hook
    # refuses every statement
    async def test_a_kept_hook_connection_is_refused(self) -> None:
        kept: list[HookConnection] = []
        async with Pool(self.dsn, max_size=1, post_recycle=kept.append) as pool:
            await pool.ping()
            await pool.ping()
            self.assertEqual([connection.recycle_count for connection in kept], [0, 1])
            self.assertTrue(all(connection.closed for connection in kept))
            with self.assertRaises(pgorm.LifecycleError):
                await kept[0].one(PID)


class MultiPoolTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.dsn = os.environ["PGORM_TEST_DSN"]

    # [spec:pgorm:req:bindings.multi-pool/test]
    async def test_pools_are_shaped_by_their_keys(self) -> None:
        async with MultiPool(self.dsn, {"web": {"max_size": 3}, "jobs": {"max_size": 1}}, recycle="clean") as pools:
            jobs = pools.get("jobs")
            assert jobs is not None
            self.assertEqual(jobs.tag, "jobs")
            self.assertEqual([tag for tag, _ in pools.items()], ["jobs", "web"])
            self.assertEqual({tag: status.max_size for tag, status in pools.status().items()}, {"jobs": 1, "web": 3})
            self.assertIsNone(pools.get("reports"))
            # The shared options reached every pool: Clean recycling resets a setting.
            async with jobs.connection() as connection:
                await connection.execute(RawSQL("SET enable_seqscan = off"))
            row = await jobs.one(RawSQL("SELECT current_setting('enable_seqscan') AS value"))
            self.assertEqual(row["value"], "on")
        self.assertTrue(pools.closed)
        self.assertTrue(all(pool.closed for _, pool in pools.items()))

    # [spec:pgorm:req:bindings.multi-pool/test]
    async def test_unbuildable_multi_pools_are_refused(self) -> None:
        for pools in ({}, {"": {}}):
            with self.subTest(pools=pools):
                with self.assertRaises(pgorm.ConstructionError):
                    MultiPool(self.dsn, pools)
        for pools in ({"web": {"tag": "other"}}, {"web": 3}, [("web", {})]):
            with self.subTest(pools=pools):
                with self.assertRaises(TypeError):
                    MultiPool(self.dsn, pools)  # type: ignore[arg-type]


class TlsTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.dsn = os.environ["PGORM_TEST_DSN"]
        self.ca = os.environ["PGORM_TEST_CA"]

    # [spec:pgorm:req:bindings.tls/test]    verify-ca verifies the chain and not the name
    async def test_verify_ca_checks_the_chain_alone(self) -> None:
        async with Pool(renamed(self.dsn, "wrong.test"), tls="verify-ca", cafile=self.ca) as pool:
            self.assertTrue(await ssl(pool))
        stranger = Pool(self.dsn, tls="verify-ca", cafile=os.environ["PGORM_TEST_CLIENT_CERT"])
        try:
            with self.assertRaises(pgorm.ConnectionError):
                await stranger.ping()
        finally:
            await stranger.close()

    # [spec:pgorm:req:bindings.tls/test]    require and prefer encrypt and verify nothing
    async def test_require_and_prefer_encrypt_unverified(self) -> None:
        for mode in ("require", "prefer"):
            with self.subTest(mode=mode):
                async with Pool(renamed(self.dsn, "wrong.test"), tls=mode) as pool:
                    self.assertTrue(await ssl(pool))

    # [spec:pgorm:req:bindings.tls/test]    a client certificate authenticates its user
    async def test_a_client_certificate_authenticates(self) -> None:
        user = os.environ["PGORM_TEST_CERT_USER"]
        certified = as_user(self.dsn, user)
        async with Pool(
            certified, tls="verify-full", cafile=self.ca,
            certfile=os.environ["PGORM_TEST_CLIENT_CERT"], keyfile=os.environ["PGORM_TEST_CLIENT_KEY"],
        ) as pool:
            self.assertEqual((await pool.one(RawSQL("SELECT current_user AS name")))["name"], user)
        anonymous = Pool(certified, tls="verify-full", cafile=self.ca)
        try:
            with self.assertRaises(pgorm.PgormError):
                await anonymous.ping()
        finally:
            await anonymous.close()

    # [spec:pgorm:req:bindings.tls/test]    what the pool would not do as asked is refused
    async def test_what_weakens_or_contradicts_tls_is_refused(self) -> None:
        cert = Path(os.environ["PGORM_TEST_CLIENT_CERT"]).read_bytes()
        server = "postgres://postgres@db.example.com/bakery"
        refusals = [
            ({"tls": "verify-ca"}, server,
             "tls='verify-ca' needs ca or cafile: the certificates the server's certificate must chain to"),
            ({"tls": "require", "cafile": self.ca}, server,
             "ca is trusted only to verify the server, which tls='require' does not do"),
            ({"tls": "prefer"}, server + "?sslmode=require",
             "sslmode=require conflicts with tls='prefer', which allows plaintext"),
            ({"tls": "verify-full", "cert": cert}, server,
             "cert and key go together: give the client certificate and its private key, or neither"),
            ({"tls": "require", "cert": cert, "key": "not a key"}, server,
             "key holds no private key rustls can use: an unencrypted PKCS #8, PKCS #1 or SEC1 key in PEM"),
            ({"tls": "disable", "cert": cert}, server, "TLS configuration conflicts with tls='disable'"),
            ({"cert": cert, "certfile": self.ca}, server,
             "pass the client certificate as cert or as certfile, not both"),
        ]
        for options, dsn, reason in refusals:
            with self.subTest(options=options):
                with self.assertRaises(pgorm.ConstructionError) as caught:
                    Pool(dsn, **options)  # type: ignore[arg-type]
                self.assertEqual(str(caught.exception), reason)
        with self.assertRaises(TypeError):
            Pool(server, tls="allow")  # type: ignore[arg-type]


class ConnectListenerTests(unittest.IsolatedAsyncioTestCase):
    # [spec:pgorm:req:bindings.listen+3/test]    a listener of no pool, over TLS too
    async def test_a_poolless_listener_receives_and_closes(self) -> None:
        dsn = os.environ["PGORM_TEST_DSN"]
        for options in ({}, {"tls": "verify-full", "cafile": os.environ["PGORM_TEST_CA"]}):
            with self.subTest(options=options):
                listener = await pgorm.connect_listener(dsn, capacity=4, **options)  # type: ignore[arg-type]
                try:
                    await listener.listen("bakery_orders")
                    async with Pool(dsn, max_size=1) as pool:
                        await pool.notify("bakery_orders", "two loaves")
                    notification = await asyncio.wait_for(anext(aiter(listener)), 5)
                    self.assertEqual((notification.channel, notification.payload), ("bakery_orders", "two loaves"))
                finally:
                    await listener.close()
                self.assertTrue(listener.closed)
        with self.assertRaises(ValueError):
            await pgorm.connect_listener(dsn, capacity=0)
        with self.assertRaises(pgorm.ConstructionError):
            await pgorm.connect_listener("not a dsn")


if __name__ == "__main__":
    unittest.main()
