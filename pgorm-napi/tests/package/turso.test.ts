// Installed-package coverage shared by Node and Deno. Each lane explicitly
// declares whether its addon includes Turso; a missing native build cannot
// silently turn this suite into skipped coverage.

import assert from "node:assert/strict";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import process from "node:process";
import { test } from "node:test";

import {
  column,
  connect,
  ConnectionError,
  ConstructionError,
  copyFrom,
  copyTo,
  DatabaseError,
  features,
  model,
  Pool,
  UniqueViolationError,
  Value,
} from "@necessary/pgorm";

const native = features.includes("turso");

async function withPath(use: (path: string) => Promise<void>): Promise<void> {
  const directory = mkdtempSync(join(tmpdir(), "pgorm-installed-turso-"));
  try {
    await use(join(directory, "database.db"));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

function within<T>(pending: Promise<T>, ms = 5000): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`still waiting after ${ms} ms`)), ms);
  });
  return Promise.race([pending, deadline]).finally(() => clearTimeout(timer));
}

function unsupported(operation: string): (error: unknown) => true {
  return (error) => {
    assert.ok(error instanceof ConnectionError, `${error}`);
    assert.ok(!(error instanceof DatabaseError), "local refusal has no database diagnostics");
    assert.match(error.message, /does not support/);
    assert.ok(error.message.includes(operation), error.message);
    return true;
  };
}

// [spec:pgorm:req:turso.javascript+1/test]
test("installed feature availability matches its lane", async () => {
  const expected = process.env.PGORM_NAPI_TURSO;
  assert.ok(expected === "0" || expected === "1", "PGORM_NAPI_TURSO must name the installed build's feature state");
  assert.equal(native, expected === "1", `installed features: ${features.join(", ")}`);
  assert.ok(features.every((feature) => typeof feature === "string"));
  if (native) return;

  await withPath(async (path) => {
    assert.throws(() => new Pool(path), ConstructionError);
    await assert.rejects(connect(path), ConstructionError);
    assert.equal(existsSync(path), false, "an unavailable backend must not open the file");
    assert.throws(() => new Pool(":memory:"), ConstructionError);
  });
});

// [spec:pgorm:req:turso.javascript+1/test]
test("native pools validate options and open lazily", { skip: !native }, async () => {
  await withPath(async (path) => {
    for (const options of [
      { tls: "disable" },
      { connectTimeout: 1000 },
      { recycle: "clean" },
      { statementCacheChannel: "native_cache" },
      { maxSize: 0 },
    ]) {
      assert.throws(() => new Pool(path, options as never), ConstructionError);
    }
    assert.throws(() => new Pool(path, { maxSize: "2" } as never), TypeError);
    assert.equal(existsSync(path), false, "argument refusals precede opening");

    let created = 0;
    await using pool = new Pool(path, {
      maxSize: 1,
      tag: "native-installed",
      postCreate: async (connection) => {
        created += 1;
        await connection.execute("CREATE TABLE hook_marker(id INTEGER)");
      },
    });
    assert.equal(pool.tag, "native-installed");
    assert.equal(pool.status().size, 0);
    assert.equal(created, 0);
    assert.equal(existsSync(path), false);
    assert.equal((await pool.one("SELECT count(*) AS n FROM hook_marker")).n, 0n);
    assert.equal(created, 1);
    assert.equal(existsSync(path), true);
    pool.clearStatementCaches();
    assert.equal((await pool.one("SELECT count(*) AS n FROM hook_marker")).n, 0n);
    assert.equal(created, 1, "healthy reuse keeps the created connection");
  });

  await withPath(async (path) => {
    await using lazy = new Pool(join(path, "missing.db"));
    assert.equal(existsSync(path), false);
    const openingFailure = (error: unknown) => {
      assert.ok(error instanceof DatabaseError, `${error}`);
      assert.equal(error.sqlstate, "XX000");
      assert.match(error.message, /I\/O error \(open\)/);
      return true;
    };
    await assert.rejects(lazy.acquire(), openingFailure);
    await assert.rejects(connect(join(path, "missing.db")), openingFailure);
    assert.equal(existsSync(path), false);
  });
  await assert.rejects(
    connect(":memory:", { postCreate: () => { throw new Error("native hook refusal"); } }),
    (error: unknown) => error instanceof ConnectionError && /native hook refusal/.test(error.message),
  );
  await using unfinished = new Pool(":memory:", {
    postCreate: async (connection) => { await connection.execute("BEGIN"); },
  });
  await assert.rejects(within(unfinished.acquire()), (error: unknown) => {
    assert.ok(error instanceof ConnectionError, `${error}`);
    assert.match(error.message, /unfinished work|open transaction/);
    return true;
  });
});

// [spec:pgorm:req:turso.javascript+1/test]
test("memory borrowers share one isolated database", { skip: !native }, async () => {
  await using pool = await connect(":memory:", { maxSize: 2 });
  const [first, second] = await Promise.all([pool.acquire(), pool.acquire()]);
  try {
    assert.equal(pool.status().size, 2);
    await first.execute("CREATE TABLE shared_items(id INTEGER PRIMARY KEY)");
    await first.execute("INSERT INTO shared_items VALUES($1)", [new Value(7, "i32")]);
    assert.equal((await second.one("SELECT id FROM shared_items")).id, 7);
    await using other = await connect(":memory:");
    await other.execute("CREATE TABLE shared_items(id INTEGER PRIMARY KEY)");
    assert.equal((await other.one("SELECT count(*) AS n FROM shared_items")).n, 0n);
    assert.equal((await second.one("SELECT count(*) AS n FROM shared_items")).n, 1n);
  } finally {
    await first.close();
    await second.close();
  }
  assert.equal((await pool.one("SELECT id FROM shared_items")).id, 7);
});

// [spec:pgorm:req:turso.javascript+1/test]
test("native parameters preserve values across row streams", { skip: !native }, async () => {
  await using pool = await connect(":memory:", { maxSize: 1 });
  await pool.execute("CREATE TABLE scalar_items(id INTEGER PRIMARY KEY, amount BIGINT, flag BOOLEAN, ratio DOUBLE PRECISION, note TEXT, data BYTEA, missing INTEGER)");
  const amount = 9_007_199_254_740_993n;
  assert.equal(await pool.execute("INSERT INTO scalar_items VALUES($1,$2,$3,$4,$5,$6,$7)", [
    new Value(1, "i32"), amount, true, new Value(1.25, "f64"), "", new Uint8Array(), Value.null("i32"),
  ]), 1);
  const row = await pool.one("SELECT * FROM scalar_items WHERE id = $1", [new Value(1, "i32")]);
  assert.equal(row.id, 1);
  assert.equal(row.amount, amount);
  assert.equal(row.flag, true);
  assert.equal(row.ratio, 1.25);
  assert.equal(row.note, "");
  assert.deepEqual(row.data, new Uint8Array());
  assert.equal(row.missing, null);

  await using raw = pool.stream("SELECT id, amount, missing FROM scalar_items WHERE id = $1", [new Value(1, "i32")]);
  const rawRows = [];
  for await (const item of raw) rawRows.push(item);
  assert.deepEqual(rawRows, [{ id: 1, amount, missing: null }]);
  await pool.transaction(async (tx) => {
    assert.equal(await tx.execute("UPDATE scalar_items SET note = $1 WHERE id = $2", ["updated", new Value(1, "i32")]), 1);
    await using rows = tx.stream("SELECT note FROM scalar_items WHERE id = $1", [new Value(1, "i32")]);
    const notes = [];
    for await (const item of rows) notes.push(item.note);
    assert.deepEqual(notes, ["updated"]);
  });
  assert.equal((await pool.one("SELECT note FROM scalar_items")).note, "updated");
});

// [spec:pgorm:req:turso.javascript+1/test]
test("file transactions persist only committed changes", { skip: !native }, async () => {
  await withPath(async (path) => {
    {
      await using pool = await connect(path, { maxSize: 1 });
      await pool.execute("CREATE TABLE durable_items(id INTEGER PRIMARY KEY)");
      const rollback = new Error("rollback this callback");
      await pool.transaction(async (tx) => {
        await tx.execute("INSERT INTO durable_items VALUES($1)", [new Value(1, "i32")]);
        await assert.rejects(tx.transaction(async (savepoint) => {
          await savepoint.execute("INSERT INTO durable_items VALUES($1)", [new Value(2, "i32")]);
          throw rollback;
        }), (error: unknown) => error === rollback);
        await tx.transaction(async (savepoint) => {
          await savepoint.execute("INSERT INTO durable_items VALUES($1)", [new Value(3, "i32")]);
        });
      });
      await assert.rejects(pool.transaction(async (tx) => {
        await tx.execute("INSERT INTO durable_items VALUES(4)");
        throw rollback;
      }), (error: unknown) => error === rollback);
      await assert.rejects(pool.transaction(async (tx) => {
        await tx.execute("INSERT INTO durable_items VALUES(5)");
        await assert.rejects(tx.execute("INSERT INTO durable_items VALUES(5)"), UniqueViolationError);
      }), (error: unknown) => error instanceof DatabaseError && error.sqlstate === "25P02");
      assert.deepEqual(await pool.query("SELECT id FROM durable_items ORDER BY id"), [{ id: 1 }, { id: 3 }]);
    }
    assert.equal(existsSync(path), true);
    await using reopened = await connect(path);
    assert.deepEqual(await reopened.query("SELECT id FROM durable_items ORDER BY id"), [{ id: 1 }, { id: 3 }]);
  });
});

// [spec:pgorm:req:turso.javascript+1/test]
test("native models write and read supported scalars", { skip: !native }, async () => {
  await using pool = await connect(":memory:");
  await pool.execute("CREATE TABLE native_records(id INTEGER PRIMARY KEY, amount BIGINT, enabled BOOLEAN, ratio DOUBLE PRECISION, note TEXT, data BYTEA)");
  const Record = model("native_records", {
    columns: {
      id: column("i32", { primaryKey: true }),
      amount: column("i64"),
      enabled: column("bool"),
      ratio: column("f64"),
      note: column("text", { nullable: true }),
      data: column("bytes"),
    },
  });
  const input = { id: 17, amount: 9_007_199_254_740_993n, enabled: true, ratio: 0.5, note: null, data: new Uint8Array([0, 127, 255]) };
  const written = await Record.insert(input).returning().one(pool);
  const found = await Record.find().where(Record.col("id").eq(17)).one(pool);
  for (const row of [written, found]) {
    assert.equal(row.id, input.id);
    assert.equal(row.amount, input.amount);
    assert.equal(row.enabled, input.enabled);
    assert.equal(row.ratio, input.ratio);
    assert.equal(row.note, null);
    assert.deepEqual(row.data, input.data);
  }
});

// [spec:pgorm:req:turso.javascript+1/test]
test("cancellation and early stream close release connections", { skip: !native }, async () => {
  await using pool = await connect(":memory:", { maxSize: 1 });
  const connection = await pool.acquire();
  const controller = new AbortController();
  const reason = new Error("stop native work");
  const timer = setTimeout(() => controller.abort(reason), 50);
  try {
    await assert.rejects(within(connection.one(
      "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 1000000000) SELECT count(*) AS total FROM n",
      [],
      { signal: controller.signal },
    )), (error: unknown) => error === reason);
    assert.equal(connection.closed, true);
  } finally {
    clearTimeout(timer);
    await connection.close();
  }
  assert.equal((await within(pool.one("SELECT 7 AS n"))).n, 7);
  await using stream = pool.stream(
    "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 100000) SELECT x::BIGINT AS n FROM n",
  );
  assert.deepEqual(await within(stream.next()), { done: false, value: { n: 1n } });
  await within(stream.close());
  assert.equal(stream.closed, true);
  assert.equal((await within(pool.one("SELECT 9 AS n"))).n, 9);
});

// [spec:pgorm:req:turso.javascript+1/test]
test("native refusals retain errors and permit reuse", { skip: !native }, async () => {
  await using pool = await connect(":memory:", { maxSize: 1 });
  await pool.execute("CREATE TABLE native_errors(id INTEGER PRIMARY KEY)");
  await assert.rejects(pool.query("COPY native_errors TO STDOUT"), ConstructionError);
  await assert.rejects(pool.copyIn(copyFrom("native_errors", "text"), new TextEncoder().encode("1\n")), unsupported("COPY"));
  await assert.rejects(pool.copyOut(copyTo("native_errors", "text")), unsupported("COPY"));
  await assert.rejects(within(pool.listener()), unsupported("notification"));
  assert.equal((await pool.one("SELECT count(*) AS n FROM native_errors")).n, 0n);

  await pool.execute("INSERT INTO native_errors VALUES($1)", [new Value(1, "i32")]);
  await assert.rejects(pool.execute("INSERT INTO native_errors VALUES($1)", [new Value(1, "i32")]), (error: unknown) => {
    assert.ok(error instanceof UniqueViolationError, `${error}`);
    assert.equal(error.sqlstate, "23505");
    assert.equal(error.table, "native_errors");
    assert.equal(error.constraint, "native_errors_pkey");
    assert.ok(error.detail);
    return true;
  });
  await assert.rejects(pool.notify("native_events", "message"), (error: unknown) => {
    assert.ok(error instanceof DatabaseError, `${error}`);
    assert.equal(error.sqlstate, "42883");
    assert.match(error.message, /no such function: pg_notify/);
    return true;
  });
  assert.equal((await pool.one("SELECT id FROM native_errors")).id, 1);
});
