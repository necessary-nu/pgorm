// Pools, connections, transactions, cancellation and streams against a live
// server, in either runtime. Every handle here is closed explicitly while the
// test still holds it, so what each release shows was the close's doing, not
// a garbage collector's.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  connect,
  ConnectionError,
  ConstructionError,
  DatabaseError,
  DecodeError,
  LifecycleError,
  Pool,
  POSTGRES_TARGET,
  type Queryable,
  STATEMENT_CACHE_CHANNEL,
  TimeoutError,
  type Transaction,
  Value,
} from "../lib/index.js";
import { dsn, holdTransaction, runFixture, same, scratchDatabase, tlsCa } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function scratch(): string {
  if (!database) throw new Error("the scratch database was not created");
  return database.dsn;
}

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

before(async () => {
  database = await scratchDatabase("pgorm_napi_connections");
  shared = new Pool(scratch(), { maxSize: 4 });
  await pool().execute("CREATE TABLE items (id int PRIMARY KEY, note text NOT NULL)");
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

async function ids(): Promise<number[]> {
  const rows = await pool().query("SELECT id FROM items ORDER BY id");
  return rows.map((row) => row.id as number);
}

async function reset(): Promise<void> {
  await pool().execute("TRUNCATE items");
}

/** `pending`, or a rejection once `ms` pass: a refusal never turns into a wait. */
function within<T>(pending: Promise<T>, ms = 3000): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`still waiting after ${ms} ms`)), ms);
  });
  return Promise.race([pending, deadline]).finally(() => clearTimeout(timer));
}

function lifecycle(pattern?: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof LifecycleError, `expected a LifecycleError, got ${error}`);
    if (pattern) assert.match(error.message, pattern);
    return true;
  };
}

/** The connection string with no `sslmode`, so a pool's TLS default applies. */
function withoutSslmode(text: string, host?: string): string {
  const url = new URL(text);
  url.searchParams.delete("sslmode");
  if (host) {
    url.searchParams.set("hostaddr", url.hostname === "localhost" ? "127.0.0.1" : url.hostname);
    url.hostname = host;
  }
  return url.toString();
}

// [spec:pgorm:req:bindings.connections+3/test]
// [spec:pgorm:req:bindings.argument-errors+1/test]
test("a pool sends nothing until used, and refuses an unusable option at construction", async () => {
  await using unreachable = new Pool("postgres://pgorm@127.0.0.1:1/postgres?sslmode=disable");
  assert.equal(unreachable.closed, false);
  for (
    const options of [
      { maxSize: 0 },
      { maxSize: 1.5 },
      { ca: "not a certificate" },
      { tls: "disable", ca: "-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----\n" },
      { connectTimeout: -1 },
      { acquireTimeout: Number.NaN },
      { statementCacheChannel: "" },
      { statementCacheChannel: "c".repeat(64) },
    ]
  ) {
    assert.throws(() => new Pool(dsn(), options as never), ConstructionError, JSON.stringify(options));
  }
  for (
    const options of [
      { maxSize: "10" },
      { tls: "maybe" },
      { tls: true },
      { ca: 7 },
      { connectTimeout: "1s" },
      { recycle: "never" },
      { statementCacheChannel: 7 },
      { maxsize: 2 },
      { signal: AbortSignal.timeout(1000) },
    ]
  ) {
    assert.throws(() => new Pool(dsn(), options as never), TypeError, JSON.stringify(options));
  }
  await assert.rejects(pool().query("SELECT 1", [], { sginal: undefined } as never), /"sginal" is no option here/);
  await assert.rejects(pool().transaction(async () => {}, { isolationLevel: "serializable" } as never), TypeError);
  await pool().transaction(async (tx) => {
    await assert.rejects(tx.transaction(async () => {}, { mode: "readOnly" } as never), TypeError);
  });
  assert.throws(() => new Pool("postgres://["), ConstructionError);
});

// [spec:pgorm:req:bindings.connections+3/test]
test("connect resolves with a pool whose server answered, and rejects when none does", async () => {
  await using connected = await connect(dsn());
  assert.equal(await connected.ping(), true);
  await assert.rejects(connect("postgres://pgorm@127.0.0.1:1/postgres?sslmode=disable"), ConnectionError);
});

// [spec:pgorm:req:bindings.connections+3/test]
test("closing a connection returns it to its pool at once, and a closed connection refuses work", async () => {
  await using single = new Pool(scratch(), { maxSize: 1, acquireTimeout: 1000 });
  const first = await single.acquire();
  assert.equal((await first.one("SELECT 1 AS n")).n, 1);
  assert.deepEqual(single.status(), { maxSize: 1, size: 1, available: 0, waiting: 0 });
  await first.close();
  assert.equal(first.closed, true);
  assert.deepEqual(single.status(), { maxSize: 1, size: 1, available: 1, waiting: 0 });
  const second = await single.acquire();
  assert.equal((await second.one("SELECT 2 AS n")).n, 2);
  await assert.rejects(first.query("SELECT 1"), lifecycle(/closed/));
  await second.close();
  await first.close();
});

/**
 * The statement the connection `db` runs on holds prepared for `sql`, as the
 * server names and stamps it, or null. A clear closes it and the next use prepares it
 * again under a new name, which is how a test tells whether a pool cleared.
 * What the statement writes cannot tell: PostgreSQL re-plans a statement a
 * session keeps whenever the session's caches are reset, and an insert
 * re-planned so writes a domain's new default under its old name.
 */
async function preparedStatement(db: Queryable, sql: string): Promise<string | null> {
  const row = await db.optional(
    `SELECT name || ' prepared at ' || prepare_time AS statement
      FROM pg_prepared_statements WHERE starts_with(statement, $1)`,
    [sql],
  );
  return row === null ? null : (row.statement as string);
}

// [spec:pgorm:req:bindings.connections+3/test]
// [spec:pgorm:req:conn.pool.statement-cache.clear+1/test]    from JavaScript
test("clearing the statement caches makes a connection that ran an insert prepare it again", async () => {
  await using single = new Pool(scratch(), { maxSize: 1 });
  await single.execute("CREATE DOMAIN stock AS integer DEFAULT 1");
  await single.execute("CREATE TABLE shelf (id int PRIMARY KEY, stock stock)");
  const insert = "INSERT INTO shelf (id) VALUES ($1) RETURNING stock";

  assert.equal((await single.one(insert, [1])).stock, 1);
  const before = await preparedStatement(single, insert);
  assert.notEqual(before, null);
  await single.execute("ALTER DOMAIN stock SET DEFAULT 2");
  await single.one(insert, [2]);
  assert.equal(await preparedStatement(single, insert), before, "nothing has cleared the connection");

  single.clearStatementCaches();
  assert.equal(await preparedStatement(single, insert), null, "the old statement is closed on the server");
  assert.equal((await single.one(insert, [3])).stock, 2);
  const after = await preparedStatement(single, insert);
  assert.notEqual(after, null);
  assert.notEqual(after, before);

  await single.close();
  single.clearStatementCaches();
});

/** Poll `probe` until it answers true, failing past ten seconds. */
async function eventually(what: string, probe: () => Promise<boolean>): Promise<void> {
  const deadline = performance.now() + 10_000;
  while (!(await probe())) {
    assert.ok(performance.now() < deadline, `${what} did not happen in time`);
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

// [spec:pgorm:req:bindings.connections+3/test]    a pool given statementCacheChannel listens from its
// first acquisition, clears on a notification there whatever its payload, and closes its
// listening connection with the pool; a pool without it keeps its statements
// [spec:pgorm:req:conn.pool.statement-cache.listen+1/test]    from JavaScript
//
// The clear the listener makes as its LISTEN takes effect can land after the server shows the
// LISTEN, closing the insert as the notification's clear does, and JavaScript sees no clear to
// wait for. Each insert is read back on the connection that ran it, which no clear reaches
// before it is next lent, and no assertion depends on which clear closed it, so the late one
// changes no outcome; the Rust suites count the clears.
test("a pool listening on statementCacheChannel clears when notified, and only that pool", async () => {
  assert.equal(STATEMENT_CACHE_CHANNEL, "pgorm_statement_caches");
  await using quiet = new Pool(scratch(), { maxSize: 1 });
  await quiet.execute("CREATE DOMAIN crate AS integer DEFAULT 1");
  await quiet.execute("CREATE TABLE pallet (id int PRIMARY KEY, crate crate)");
  const insert = "INSERT INTO pallet (id) VALUES ($1) RETURNING crate";
  const listening = `SELECT count(*)::int AS n FROM pg_stat_activity
    WHERE datname = current_database() AND query = $1 AND state = 'idle'`;
  const listen = [`LISTEN "${STATEMENT_CACHE_CHANNEL}"`];

  /** Insert on one connection of `pool`, answering the crate written and the statement that wrote it. */
  async function inserted(pool: Pool, id: number): Promise<[unknown, string | null]> {
    await using connection = await pool.acquire();
    const crate = (await connection.one(insert, [id])).crate;
    return [crate, await preparedStatement(connection, insert)];
  }

  const notified = new Pool(scratch(), { maxSize: 1, statementCacheChannel: STATEMENT_CACHE_CHANNEL });
  try {
    assert.equal(await notified.ping(), true);
    await eventually("the listener's LISTEN", async () => (await quiet.one(listening, listen)).n === 1);
    const [first, closed] = await inserted(notified, 1);
    assert.equal(first, 1);
    assert.notEqual(closed, null);
    const [second, kept] = await inserted(quiet, 2);
    assert.equal(second, 1);
    assert.notEqual(kept, null);

    await quiet.execute("ALTER DOMAIN crate SET DEFAULT 2");
    await quiet.execute("SELECT pg_notify($1, 'not read')", [STATEMENT_CACHE_CHANNEL]);
    await eventually("the clear", async () => (await preparedStatement(notified, insert)) === null);
    const [third, again] = await inserted(notified, 3);
    assert.equal(third, 2);
    assert.notEqual(again, null);
    assert.notEqual(again, closed);
    assert.equal((await inserted(quiet, 4))[1], kept, "the quiet pool kept its insert");

    await notified.close();
    await eventually("the listener's close", async () => (await quiet.one(listening, listen)).n === 0);
  } finally {
    await notified.close();
  }
});

// [spec:pgorm:req:bindings.connections+3/test]
// [spec:pgorm:req:bindings.errors+6/test]
test("acquiring waits for a free connection within acquireTimeout, then fails with a TimeoutError", async () => {
  await using single = new Pool(scratch(), { maxSize: 1, acquireTimeout: 200 });
  await using held = await single.acquire();
  const started = performance.now();
  await assert.rejects(within(single.acquire()), TimeoutError);
  assert.ok(performance.now() - started < 2000);
  assert.equal((await held.one("SELECT 3 AS n")).n, 3);
});

// [spec:pgorm:req:bindings.connections+3/test]
test("a connection runs one operation at a time and refuses a second rather than racing it", async () => {
  await using connection = await pool().acquire();
  // Refusal follows the order JavaScript calls in, whichever runtime thread
  // gets to the connection first.
  for (let round = 0; round < 5; round += 1) {
    const sleeping = connection.execute("SELECT pg_sleep(0.05)");
    const refused = Array.from({ length: 50 }, () => connection.query("SELECT 1"));
    for (const attempt of refused) await assert.rejects(attempt, lifecycle(/busy/));
    await sleeping;
  }
  assert.equal((await connection.one("SELECT 4 AS n")).n, 4);
});

// [spec:pgorm:req:bindings.connections+3/test]
// [spec:pgorm:req:bindings.cancellation/test]
test("closing a pool cancels what runs on it and releases every connection", async () => {
  const closing = new Pool(scratch(), { maxSize: 3 });
  const busy = await closing.acquire();
  const idle = await closing.acquire();
  const held = await holdTransaction(async (use) => await (await closing.acquire()).transaction(use));
  const running = busy.execute("SELECT pg_sleep(10)").then(
    () => assert.fail("the statement outlived its pool"),
    (error: unknown) => error,
  );
  const started = performance.now();
  await closing.close();
  assert.ok(performance.now() - started < 2000, "close did not wait for the running statement");
  lifecycle(/closed/)(await running);
  assert.equal(closing.closed, true);
  assert.equal(idle.closed, true);
  assert.equal(held.transaction.closed, true);
  await assert.rejects(held.end(), lifecycle(/closed/));
  await assert.rejects(idle.query("SELECT 1"), lifecycle(/closed/));
  await assert.rejects(closing.query("SELECT 1"), lifecycle(/closed/));
  await assert.rejects(closing.acquire(), lifecycle(/closed/));
});

// [spec:pgorm:req:napi.lifecycle+3/test]
test("collecting unreferenced handles never cuts work short, and rolls back and returns what is idle", async () => {
  const outcome = await runFixture("collected", 30_000, {}, { exposeGc: true });
  assert.equal(outcome.stderr, "");
  assert.equal(outcome.code, 0);
  assert.equal(
    outcome.stdout,
    "finished 1 false\nidle size 1 available 1\nabandoned size 1 available 1\nrolled back true\n",
  );
});

// [spec:pgorm:req:bindings.postgres-target/test]
test("POSTGRES_TARGET names the release the build targets, the one the suite's server runs", async () => {
  assert.ok(POSTGRES_TARGET === 18 || POSTGRES_TARGET === 19);
  const { release } = await pool().one("SELECT current_setting('server_version_num')::int / 10000 AS release");
  assert.equal(release, POSTGRES_TARGET);
});

// [spec:pgorm:req:bindings.connections+3/test]
test("verify-full TLS never falls back to plaintext", async () => {
  await using strict = new Pool(withoutSslmode(dsn()));
  await assert.rejects(strict.ping(), ConnectionError);
});

// [spec:pgorm:req:bindings.connections+3/test]
test("verify-full TLS connects through a configured CA, and checks the host name", async (context) => {
  const ca = tlsCa();
  if (ca === undefined) {
    context.skip("PGORM_TEST_CA names no CA: the server runs without TLS");
    return;
  }
  await using verified = new Pool(withoutSslmode(dsn(), "localhost"), { ca });
  const row = await verified.one("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()");
  assert.equal(row.ssl, true);
  await using misnamed = new Pool(withoutSslmode(dsn(), "wrong.test"), { ca });
  await assert.rejects(misnamed.ping(), ConnectionError);
});

// [spec:pgorm:req:bindings.connections+3/test]
test("verify-full TLS binds SCRAM authentication to the server's certificate", async (context) => {
  const ca = tlsCa();
  if (ca === undefined || new URL(dsn()).password === "") {
    context.skip("needs a TLS server that authenticates with a password: PGORM_TEST_CA and SCRAM");
    return;
  }
  // channel_binding=require refuses SCRAM unless it is bound to the TLS
  // session, over the certificate's tls-server-end-point hash, so the TLS
  // connector has to report that binding for this to connect. Plaintext has
  // nothing to bind to, and the same setting refuses it.
  const bound = new URL(withoutSslmode(dsn(), "localhost"));
  bound.searchParams.set("channel_binding", "require");
  await using verified = new Pool(bound.toString(), { ca });
  assert.equal(await verified.ping(), true);
  const plain = new URL(dsn());
  plain.searchParams.set("channel_binding", "require");
  await using refused = new Pool(plain.toString());
  await assert.rejects(refused.ping(), ConnectionError);
});

// [spec:pgorm:req:bindings.results+2/test]
test("execute resolves with the number of rows a statement affected", async () => {
  await reset();
  assert.equal(await pool().execute("INSERT INTO items VALUES (1, 'a'), (2, 'b'), (3, 'c')"), 3);
  assert.equal(await pool().execute("UPDATE items SET note = $1 WHERE id > $2", ["z", 1]), 2);
  assert.equal(await pool().execute("DELETE FROM items WHERE id = $1", [9]), 0);
  assert.equal(await pool().execute("SELECT * FROM items"), 3);
});

// [spec:pgorm:req:bindings.results+2/test]
// [spec:pgorm:req:bindings.row-count/test]
test("one takes exactly one row and optional at most one, any other count a DecodeError", async () => {
  assert.equal((await pool().one("SELECT 1 AS n")).n, 1);
  await assert.rejects(pool().one("SELECT 1 WHERE false"), (error: unknown) => {
    assert.ok(error instanceof DecodeError);
    assert.match(error.message, /exactly one row, received 0/);
    return true;
  });
  await assert.rejects(pool().one("SELECT generate_series(1, 2)"), DecodeError);
  assert.equal(await pool().optional("SELECT 1 WHERE false"), null);
  assert.equal((await pool().optional("SELECT 5 AS n"))?.n, 5);
  await assert.rejects(pool().optional("SELECT generate_series(1, 2)"), /at most one row, received 2/);
  await assert.rejects(pool().one("SELECT 1 / 0"), (error: unknown) => {
    assert.ok(error instanceof DatabaseError);
    assert.equal(error.sqlstate, "22012");
    return true;
  });
});

// [spec:pgorm:req:napi.records/test]
test("every row terminal can give its columns tagged", async () => {
  const one = await pool().one("SELECT 1::int2 AS n", [], { tagged: true });
  assert.ok(one.n instanceof Value);
  assert.equal(one.n.kind, "i16");
  const optional = await pool().optional("SELECT 1::int8 AS n", [], { tagged: true });
  assert.equal(optional?.n?.kind, "i64");
  const [all] = await pool().query("SELECT NULL::text AS n", [], { tagged: true });
  assert.equal(all?.n?.isNull, true);
});

// [spec:pgorm:req:bindings.transactions+2/test]
test("a callback transaction commits when it resolves and rolls back when it throws", async () => {
  await reset();
  const result = await pool().transaction(async (tx) => {
    await tx.execute("INSERT INTO items VALUES (1, 'kept')");
    return "done";
  });
  assert.equal(result, "done");
  await using connection = await pool().acquire();
  const failure = new Error("the callback failed");
  await assert.rejects(
    connection.transaction(async (tx) => {
      await tx.execute("INSERT INTO items VALUES (2, 'undone')");
      throw failure;
    }),
    (error: unknown) => error === failure,
  );
  assert.equal(connection.closed, false, "the rollback left the connection usable");
  assert.equal((await within(connection.one("SELECT count(*)::int AS n FROM items"))).n, 1);
  assert.deepEqual(await ids(), [1]);
});

// [spec:pgorm:req:bindings.transactions+2/test]
test("a transaction holds its connection until its callback settles, and is closed after", async () => {
  await reset();
  await using connection = await pool().acquire();
  let kept = undefined as Transaction | undefined;
  await connection.transaction(async (tx) => {
    kept = tx;
    assert.equal(tx.closed, false);
    await tx.execute("INSERT INTO items VALUES (1, 'x')");
    await assert.rejects(connection.query("SELECT 1"), lifecycle(/busy/));
  });
  assert.ok(kept);
  assert.equal(kept.closed, true);
  await assert.rejects(kept.query("SELECT 1"), lifecycle(/closed/));
  assert.equal((await connection.one("SELECT count(*)::int AS n FROM items")).n, 1);
  assert.deepEqual(await ids(), [1]);
});

// [spec:pgorm:req:napi.transactions+3/test]
test("a transaction opens only around a callback, which alone ends it", async () => {
  await using connection = await pool().acquire();
  // @ts-expect-error a transaction opens only through transaction(fn)
  assert.equal(connection.begin, undefined);
  await connection.transaction(async (tx) => {
    // @ts-expect-error a savepoint opens only through transaction(fn)
    assert.equal(tx.begin, undefined);
    // @ts-expect-error the callback resolving commits
    assert.equal(tx.commit, undefined);
    // @ts-expect-error the callback throwing rolls back
    assert.equal(tx.rollback, undefined);
    // @ts-expect-error the transaction ends with its callback
    assert.equal(tx.close, undefined);
    assert.equal(Symbol.asyncDispose in tx, false);
    // @ts-expect-error a transaction is not disposable
    const disposable: AsyncDisposable = tx;
    assert.ok(disposable);
  });
});

// [spec:pgorm:req:bindings.transactions+2/test]
test("a savepoint rolls back alone inside a transaction that commits", async () => {
  await reset();
  await pool().transaction(async (tx) => {
    await tx.execute("INSERT INTO items VALUES (1, 'outer')");
    await assert.rejects(
      tx.transaction(async (inner) => {
        await inner.execute("INSERT INTO items VALUES (3, 'inner')");
        throw new Error("undo the savepoint");
      }),
      /undo the savepoint/,
    );
    await tx.transaction(async (inner) => {
      await inner.execute("INSERT INTO items VALUES (4, 'released')");
    });
  });
  assert.deepEqual(await ids(), [1, 4]);
});

// [spec:pgorm:req:bindings.transactions+2/test]
test("a callback that catches a failed statement's error cannot commit, and its connection is kept", async () => {
  await reset();
  const aborted =
    "the transaction was rolled back, not committed: a statement that failed inside it had aborted it";
  await using connection = await pool().acquire();
  await assert.rejects(
    connection.transaction(async (tx) => {
      await tx.execute("INSERT INTO items VALUES (1, 'lost')");
      await assert.rejects(tx.execute("INSERT INTO items VALUES (1, 'again')"), DatabaseError);
    }),
    (error: unknown) => {
      assert.ok(error instanceof DatabaseError, `${error}`);
      assert.equal(error.sqlstate, "25P02");
      assert.equal(error.message, aborted);
      return true;
    },
  );
  assert.equal(connection.closed, false);
  assert.deepEqual(await connection.query("SELECT id FROM items"), []);
  await connection.transaction(async (tx) => {
    await tx.execute("INSERT INTO items VALUES (2, 'kept')");
    await assert.rejects(
      tx.transaction(async (savepoint) => {
        await savepoint.execute("INSERT INTO items VALUES (3, 'lost')");
        await assert.rejects(savepoint.execute("INSERT INTO items VALUES (2, 'again')"), DatabaseError);
      }),
      (error: unknown) => error instanceof DatabaseError && error.sqlstate === "25P02",
    );
    await tx.execute("INSERT INTO items VALUES (4, 'kept')");
  });
  assert.deepEqual(await ids(), [2, 4]);
});

// [spec:pgorm:req:napi.transactions+3/test]
// [spec:pgorm:req:bindings.rollback-failed/test]    the rollback's failure and the callback's, both carried
test("a rollback the server cannot answer rejects with its failure and discards the connection", async () => {
  await reset();
  const connection = await pool().acquire();
  const pid = (await connection.one("SELECT pg_backend_pid() AS pid")).pid as number;
  const failure = new Error("the callback failed");
  let tx = undefined as Transaction | undefined;
  await assert.rejects(
    connection.transaction(async (opened) => {
      tx = opened;
      await opened.execute("INSERT INTO items VALUES (1, 'lost')");
      await pool().execute("SELECT pg_terminate_backend($1, 5000)", [pid]);
      throw failure;
    }),
    (error: unknown) => {
      assert.ok(error instanceof SuppressedError, `${error}`);
      assert.ok(error.error instanceof ConnectionError || error.error instanceof DatabaseError, `${error.error}`);
      assert.equal(error.suppressed, failure);
      return true;
    },
  );
  assert.equal(tx?.closed, true);
  assert.equal(connection.closed, true);
  await connection.close();
  assert.deepEqual(await ids(), []);
});

// [spec:pgorm:req:bindings.transactions+2/test]
test("a transaction is refused while its savepoint is open, not raced", async () => {
  await reset();
  await using connection = await pool().acquire();
  await connection.transaction(async (tx) => {
    await tx.transaction(async (savepoint) => {
      await assert.rejects(within(tx.query("SELECT 1")), lifecycle(/savepoint of it is open/));
      await assert.rejects(within(tx.transaction(async () => {})), lifecycle(/busy/));
      await assert.rejects(connection.query("SELECT 1"), lifecycle(/busy/));
      await savepoint.execute("INSERT INTO items VALUES (1, 'savepoint')");
    });
    await tx.execute("INSERT INTO items VALUES (2, 'transaction')");
  });
  assert.deepEqual(await ids(), [1, 2]);
});

// [spec:pgorm:req:bindings.transactions+2/test]
test("a second statement on a transaction already running one is refused, not queued", async () => {
  await using connection = await pool().acquire();
  await connection.transaction(async (tx) => {
    for (let round = 0; round < 5; round += 1) {
      const sleeping = tx.execute("SELECT pg_sleep(0.05)");
      const refused = Array.from({ length: 50 }, () => tx.query("SELECT 1"));
      for (const attempt of refused) await assert.rejects(attempt, lifecycle(/busy/));
      await sleeping;
    }
  });
});

// [spec:pgorm:req:napi.transactions+3/test]
test("a callback that returns with its savepoint open fails, and nothing it wrote commits", async () => {
  await reset();
  const connection = await pool().acquire();
  let savepoint: Promise<unknown> | undefined;
  const release = Promise.withResolvers<void>();
  await assert.rejects(
    connection.transaction(async (tx) => {
      await tx.execute("INSERT INTO items VALUES (1, 'left open')");
      const opened = Promise.withResolvers<void>();
      savepoint = tx.transaction(async () => {
        opened.resolve();
        await release.promise;
      });
      await opened.promise;
    }),
    lifecycle(/busy/),
  );
  release.resolve();
  await assert.rejects(within(savepoint ?? Promise.resolve()), lifecycle(/closed/));
  assert.equal(connection.closed, true);
  await connection.close();
  assert.deepEqual(await ids(), []);
});

// [spec:pgorm:req:bindings.transactions+2/test]
test("a failed statement aborts the transaction, and a savepoint recovers from one", async () => {
  await reset();
  await pool().transaction(async (tx) => {
    await tx.execute("INSERT INTO items VALUES (1, 'before')");
    await assert.rejects(
      tx.transaction(async (savepoint) => {
        await assert.rejects(savepoint.execute("INSERT INTO items VALUES (1, 'duplicate')"), (error: unknown) => {
          assert.ok(error instanceof DatabaseError);
          assert.equal(error.sqlstate, "23505");
          return true;
        });
        await savepoint.query("SELECT 1");
      }),
      (error: unknown) => {
        assert.ok(error instanceof DatabaseError);
        assert.equal(error.sqlstate, "25P02");
        return true;
      },
    );
    await tx.execute("INSERT INTO items VALUES (2, 'after')");
  });
  assert.deepEqual(await ids(), [1, 2]);
});

// [spec:pgorm:req:bindings.transactions+2/test]
test("a transaction's mode and isolation level reach the server", async () => {
  const settings =
    "SELECT current_setting('transaction_isolation') AS isolation, current_setting('transaction_read_only') AS read_only, current_setting('transaction_deferrable') AS deferrable";
  same(
    await pool().transaction((tx) => tx.one(settings), { mode: "readOnly", isolation: "repeatableRead" }),
    { isolation: "repeatable read", read_only: "on", deferrable: "off" },
  );
  same(await pool().transaction((tx) => tx.one(settings), { mode: "deferrable" }), {
    isolation: "serializable",
    read_only: "on",
    deferrable: "on",
  });
  await assert.rejects(
    pool().transaction((tx) => tx.execute("INSERT INTO items VALUES (9, 'read only')"), { mode: "readOnly" }),
    (error: unknown) => error instanceof DatabaseError && error.sqlstate === "25006",
  );
  await assert.rejects(pool().transaction(async () => {}, { mode: "default", isolation: "serializable" }), ConstructionError);
  await assert.rejects(pool().transaction(async () => {}, { mode: "sometimes" as never }), TypeError);
  await assert.rejects(pool().transaction(async () => {}, { isolation: "eventual" as never }), TypeError);
});

// [spec:pgorm:req:napi.temporal+1/test]
test("a timestamptz reads as one instant whatever time zone the session is set to", async () => {
  const instant = Temporal.Instant.from("2026-01-01T12:00:00.000001Z");
  const row = await pool().transaction(async (tx) => {
    await tx.execute("SET LOCAL TIME ZONE 'Pacific/Chatham'");
    return await tx.one("SELECT $1::timestamptz AS v, $1::timestamptz::text AS t, now() AS n", [instant]);
  });
  same(row.v, instant);
  assert.equal(row.t, "2026-01-02 01:45:00.000001+13:45");
  assert.ok(row.n instanceof Temporal.Instant);
});

// [spec:pgorm:req:napi.cancellation+2/test]
// [spec:pgorm:req:bindings.cancellation/test]
test("aborting a statement rejects with the signal's reason and discards its connection", async () => {
  await using single = new Pool(scratch(), { maxSize: 1 });
  const connection = await single.acquire();
  const controller = new AbortController();
  const reason = new Error("stop waiting");
  const started = performance.now();
  setTimeout(() => controller.abort(reason), 100);
  await assert.rejects(
    connection.execute("SELECT pg_sleep(10)", [], { signal: controller.signal }),
    (error: unknown) => error === reason,
  );
  assert.ok(performance.now() - started < 2000);
  assert.equal(connection.closed, true);
  assert.equal(single.status().size, 0, "the discarded connection did not go back to the pool");
  assert.equal((await single.one("SELECT 6 AS n")).n, 6);
  await connection.close();
});

// [spec:pgorm:req:bindings.cancellation/test]
test("an already aborted signal rejects before anything is sent", async () => {
  await using connection = await pool().acquire();
  const signal = AbortSignal.abort(new Error("never started"));
  await assert.rejects(connection.query("SELECT 1", [], { signal }), /never started/);
  assert.equal(connection.closed, false);
  assert.equal((await connection.one("SELECT 7 AS n")).n, 7);
});

// [spec:pgorm:req:bindings.cancellation/test]
test("aborting a statement in a transaction ends the transaction and nothing commits", async () => {
  await reset();
  const connection = await pool().acquire();
  let tx = undefined as Transaction | undefined;
  await assert.rejects(
    connection.transaction(async (opened) => {
      tx = opened;
      await opened.execute("INSERT INTO items VALUES (1, 'uncertain')");
      await opened.execute("SELECT pg_sleep(10)", [], { signal: AbortSignal.timeout(100) });
    }),
    (error: unknown) => error instanceof DOMException && error.name === "TimeoutError",
  );
  assert.ok(tx);
  assert.equal(tx.closed, true);
  assert.equal(connection.closed, true);
  await assert.rejects(tx.query("SELECT 1"), lifecycle(/closed/));
  await connection.close();
  assert.deepEqual(await ids(), []);
});

// [spec:pgorm:req:napi.cancellation+2/test]
test("a signal bounds a pool's own statement and acquisition", async () => {
  await assert.rejects(
    pool().execute("SELECT pg_sleep(10)", [], { signal: AbortSignal.timeout(100) }),
    (error: unknown) => error instanceof DOMException && error.name === "TimeoutError",
  );
  await using single = new Pool(scratch(), { maxSize: 1 });
  await using held = await single.acquire();
  await assert.rejects(single.acquire({ signal: AbortSignal.timeout(100) }), (error: unknown) => {
    return error instanceof DOMException && error.name === "TimeoutError";
  });
  assert.equal((await held.one("SELECT 8 AS n")).n, 8);
});

// [spec:pgorm:req:bindings.streams+1/test]
test("a stream yields its rows one per pull and returns its connection after the last", async () => {
  await using single = new Pool(scratch(), { maxSize: 1, acquireTimeout: 1000 });
  const rows = [];
  for await (const row of single.stream("SELECT generate_series(1, $1::int) AS n", [5])) rows.push(row.n);
  assert.deepEqual(rows, [1, 2, 3, 4, 5]);
  assert.deepEqual(single.status(), { maxSize: 1, size: 1, available: 1, waiting: 0 });
  assert.equal((await single.one("SELECT 9 AS n")).n, 9);
  const tagged = single.stream("SELECT 1::int2 AS n", [], { tagged: true });
  const first = await tagged.next();
  assert.equal(first.value?.n?.kind, "i16");
  assert.deepEqual(await tagged.next(), { done: true, value: undefined });
  assert.equal(tagged.closed, true);
});

// [spec:pgorm:req:bindings.streams+1/test]
test("leaving a stream early releases its connection", async () => {
  await using single = new Pool(scratch(), { maxSize: 1, acquireTimeout: 1000 });
  let seen = 0;
  for await (const row of single.stream("SELECT generate_series(1, 100000) AS n")) {
    seen = row.n as number;
    if (seen === 2) break;
  }
  assert.equal(seen, 2);
  assert.equal((await single.one("SELECT 10 AS n")).n, 10);
  await using stream = single.stream("SELECT generate_series(1, 100000) AS n");
  await stream.next();
  await stream.close();
  assert.equal((await single.one("SELECT 11 AS n")).n, 11);
});

// [spec:pgorm:req:bindings.streams+1/test]
test("a connection's stream holds the connection, freeing it at the end and discarding it if left early", async () => {
  const connection = await pool().acquire();
  const stream = connection.stream("SELECT generate_series(1, 3) AS n");
  assert.deepEqual((await stream.next()).value, { n: 1 });
  await assert.rejects(connection.query("SELECT 1"), lifecycle(/busy/));
  const rest = [];
  for await (const row of stream) rest.push(row.n);
  assert.deepEqual(rest, [2, 3]);
  assert.equal(stream.closed, true);
  assert.equal((await connection.one("SELECT 12 AS n")).n, 12);
  const early = connection.stream("SELECT generate_series(1, 100) AS n");
  await early.next();
  await early.close();
  assert.equal(connection.closed, true);
  await connection.close();
});

// [spec:pgorm:req:bindings.streams+1/test]
test("a stream holds the server back while nothing pulls", async () => {
  await using streaming = new Pool(scratch(), { maxSize: 2 });
  const marker = `held back ${process.pid}`;
  const stream = streaming.stream(`SELECT n, $1::text AS marker FROM generate_series(1, 50000000) AS n`, [marker]);
  await stream.next();
  let state;
  for (let attempt = 0; attempt < 50; attempt += 1) {
    state = await streaming.optional(
      "SELECT state, wait_event FROM pg_stat_activity WHERE datname = current_database() AND query LIKE '%generate_series(1, 50000000)%' AND pid <> pg_backend_pid()",
    );
    if (state?.wait_event === "ClientWrite") break;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  same(state, { state: "active", wait_event: "ClientWrite" });
  await stream.close();
});

// [spec:pgorm:req:bindings.streams+1/test]
// [spec:pgorm:req:bindings.cancellation/test]
test("closing a pool ends an idle stream and an idle transaction without waiting for them", async () => {
  const closing = new Pool(scratch(), { maxSize: 2 });
  const stream = closing.stream("SELECT generate_series(1, 100000) AS n");
  await stream.next();
  const held = await holdTransaction(async (use) => await (await closing.acquire()).transaction(use));
  const started = performance.now();
  await closing.close();
  assert.ok(performance.now() - started < 2000);
  await assert.rejects(stream.next(), lifecycle(/closed/));
  assert.equal(stream.closed, true);
  assert.equal(held.transaction.closed, true);
  await assert.rejects(held.end(), lifecycle(/closed/));
});
