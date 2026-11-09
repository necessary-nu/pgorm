// pgorm's metrics layer: an InstrumentedPool reports what runs through it to
// a JavaScript collector, against a live server, in either runtime.

import assert from "node:assert/strict";
import process from "node:process";
import { after, before, test } from "node:test";

import {
  column,
  copyFrom,
  DatabaseError,
  InstrumentedPool,
  LoggingMetrics,
  model,
  NoOpMetrics,
  pipeline as pl,
  Pool,
  type PgormError,
  QueryContext,
  QueryFingerprint,
  Table,
} from "../lib/index.js";
import { scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

before(async () => {
  database = await scratchDatabase("pgorm_napi_metrics");
  shared = new Pool(database.dsn, { maxSize: 2 });
  await pool().execute("CREATE TABLE widget (id int PRIMARY KEY, name text NOT NULL)");
  await pool().execute("INSERT INTO widget VALUES (1, 'bolt'), (2, 'nut')");
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

type Report = readonly unknown[];

/** Every report, in order, each naming its hook first. */
class Recorder extends NoOpMetrics {
  reports: Report[] = [];

  override recordQuerySuccess(query: QueryContext, duration: number, rows: number | null) {
    this.reports.push(["ok", query.operation, query.sql, rows, query.fingerprint?.toString() ?? null, typeof duration]);
  }

  override recordQueryError(query: QueryContext, _duration: number, error: PgormError) {
    this.reports.push(["error", query.operation, query.sql, error.constructor.name]);
  }

  override recordConnectionAcquired(duration: number) {
    this.reports.push(["acquired", typeof duration]);
  }

  override recordTransactionBegin() {
    this.reports.push(["begin"]);
  }

  override recordTransactionCommit() {
    this.reports.push(["commit"]);
  }

  override recordTransactionRollback() {
    this.reports.push(["rollback"]);
  }

  hooks(): unknown[] {
    return this.reports.map((report) => report[0]);
  }

  operations(): unknown[] {
    return this.reports.filter((report) => report[0] === "ok").map((report) => report[1]);
  }
}

function instrumented(): [InstrumentedPool<Recorder>, Recorder] {
  const recorder = new Recorder();
  return [new InstrumentedPool(pool(), recorder), recorder];
}

/** The warnings the process emits while `work` runs. */
async function warned(work: () => Promise<void>): Promise<Error[]> {
  const seen: Error[] = [];
  const listener = (warning: Error) => seen.push(warning);
  process.on("warning", listener);
  try {
    await work();
    await new Promise((resolve) => setTimeout(resolve, 50));
  } finally {
    process.off("warning", listener);
  }
  return seen;
}

// [spec:pgorm:req:bindings.metrics/test]    a fingerprint is pgorm's, the same for statements differing in constants alone
test("fingerprints are pgorm's, and a context with no statement has none", () => {
  const print = (sql: string) => {
    const fingerprint = new QueryContext("queryAll", sql).fingerprint;
    assert.ok(fingerprint instanceof QueryFingerprint);
    return fingerprint;
  };
  const byId = print("SELECT id FROM widget WHERE id = 1");
  assert.equal(String(byId), "394a2f90c244bffe");
  assert.equal(byId.value, 0x394a2f90c244bffen);
  assert.ok(byId.equals(print("SELECT id FROM widget WHERE id = 4242")));
  assert.ok(byId.equals(print("SELECT id FROM public.widget WHERE id = 1")));
  assert.ok(!byId.equals(print("SELECT name FROM widget WHERE id = 1")));
  assert.equal(new QueryContext("begin").fingerprint, null);
  assert.equal(new QueryContext("execute", "SELEKT nonsense").fingerprint, null);
  assert.throws(() => new (QueryFingerprint as unknown as new () => unknown)(), TypeError);
  assert.throws(() => new QueryContext(7 as never), TypeError);
});

// [spec:pgorm:req:bindings.metrics/test]    a collector has all seven hooks, and only a Pool not instrumented is wrapped
test("a collector lacking a hook, and a pool that cannot be wrapped, are refused", () => {
  const { recordQueryError: _, ...missing } = Object.fromEntries(
    ["recordQuerySuccess", "recordQueryError", "recordConnectionAcquired", "recordConnectionError",
      "recordTransactionBegin", "recordTransactionCommit", "recordTransactionRollback"].map((name) => [name, () => {}]),
  );
  assert.throws(() => new InstrumentedPool(pool(), missing as never), /recordQueryError/);
  assert.throws(() => new InstrumentedPool({} as never, new NoOpMetrics()), TypeError);
  const [wrapped] = instrumented();
  assert.throws(() => new InstrumentedPool(wrapped, new NoOpMetrics()), /not instrumented/);
});

// [spec:pgorm:req:bindings.metrics/test]    statements, their rows and their failures are reported, and the failure still reaches the caller
test("statements and acquisitions are reported", async () => {
  const [db, recorder] = instrumented();
  assert.equal((await db.query("SELECT id FROM widget WHERE id > $1", [0])).length, 2);
  await assert.rejects(db.query("SELECT * FROM missing"), DatabaseError);
  assert.equal(await db.execute("UPDATE widget SET name = name"), 2);
  assert.deepStrictEqual(recorder.reports, [
    ["acquired", "number"],
    ["ok", "queryAll", "SELECT id FROM widget WHERE id > $1", 2,
      String(new QueryContext("queryAll", "SELECT id FROM widget WHERE id > $1").fingerprint), "number"],
    ["acquired", "number"],
    ["error", "queryAll", "SELECT * FROM missing", "DatabaseError"],
    ["acquired", "number"],
    ["ok", "execute", "UPDATE widget SET name = name", 2,
      String(new QueryContext("execute", "UPDATE widget SET name = name").fingerprint), "number"],
  ]);
});

// [spec:pgorm:req:bindings.metrics/test]    a model's query, a pipeline, a stream, a COPY's start and a notification are reported
test("every executor path is reported", async () => {
  const [db, recorder] = instrumented();
  const Widget = model("widget", { columns: { id: column("i32", { primaryKey: true }), name: column("text") } });
  assert.equal((await Widget.find().all(db)).length, 2);
  const name = pl.col("widget", "name");
  assert.deepStrictEqual(await pl.from(new Table("widget")).filter(name.eq("bolt")).select(name).one(db), { name: "bolt" });
  await db.connection(async (connection) => {
    const streamed: unknown[] = [];
    for await (const row of connection.stream("SELECT id FROM widget")) streamed.push(row.id);
    assert.equal(streamed.length, 2);
    assert.equal(await connection.copyIn(copyFrom("widget", "text"), new TextEncoder().encode("3\tscrew\n")), 1);
    await connection.notify("napi_metrics", "done");
  });
  assert.deepStrictEqual(recorder.operations(), ["queryAll", "queryAll", "queryRaw", "copyIn", "execute"]);
  assert.equal(recorder.hooks().filter((hook) => hook === "acquired").length, 3);
  await pool().execute("DELETE FROM widget WHERE id = 3");
});

// [spec:pgorm:req:bindings.metrics/test]    a transaction, its savepoint, its mode and its rollback are reported as Rust reports them
test("transactions are reported", async () => {
  const [db, recorder] = instrumented();
  await db.transaction(async (tx) => {
    await tx.query("SELECT 1");
    await tx.transaction((savepoint) => savepoint.execute("UPDATE widget SET name = 'x' WHERE id = 1"));
  });
  await assert.rejects(
    db.transaction((tx) => tx.execute("DELETE FROM widget"), { mode: "readOnly" }),
    DatabaseError,
  );
  await assert.rejects(db.transaction(async () => {
    throw new RangeError("changed my mind");
  }), RangeError);
  assert.deepStrictEqual(recorder.hooks(), [
    "acquired", "begin", "ok", "begin", "ok", "commit", "commit",
    "acquired", "begin", "error", "rollback",
    "acquired", "begin", "rollback",
  ]);
});

// [spec:pgorm:req:bindings.metrics/test]    a hook returning a promise is awaited: once the statement settles, its collector has been told
test("a promise a hook returns is awaited", async () => {
  class Slow extends NoOpMetrics {
    told: string[] = [];
    override async recordQuerySuccess(query: QueryContext) {
      await new Promise((resolve) => setTimeout(resolve, 50));
      this.told.push(query.operation);
    }
  }
  const slow = new Slow();
  await new InstrumentedPool(pool(), slow).one("SELECT 1 AS one");
  assert.deepStrictEqual(slow.told, ["queryAll"]);
});

// [spec:pgorm:req:bindings.metrics/test]    a hook that throws, or a promise that rejects, breaks no statement: a MetricsWarning carries it
test("failing hooks become process warnings and break no statement", async () => {
  class Broken extends NoOpMetrics {
    override recordQuerySuccess(): void {
      throw new SyntaxError("plain hook");
    }
    override async recordTransactionCommit(): Promise<void> {
      throw new URIError("async hook");
    }
  }
  const db = new InstrumentedPool(pool(), new Broken());
  const warnings = await warned(async () => {
    assert.deepStrictEqual(await db.one("SELECT 7 AS seven"), { seven: 7 });
    await db.transaction((tx) => tx.execute("UPDATE widget SET name = 'y' WHERE id = 2"));
  });
  assert.deepStrictEqual((await pool().one("SELECT name FROM widget WHERE id = 2")).name, "y");
  assert.deepStrictEqual(
    warnings.map((warning) => [warning.name, warning.message, (warning.cause as Error).constructor.name]),
    [
      ["MetricsWarning", "pgorm's metrics hook recordQuerySuccess failed", "SyntaxError"],
      ["MetricsWarning", "pgorm's metrics hook recordQuerySuccess failed", "SyntaxError"],
      ["MetricsWarning", "pgorm's metrics hook recordTransactionCommit failed", "URIError"],
    ],
  );
});

// [spec:pgorm:req:bindings.metrics/test]    the wrapped pool runs unreported, shares the instrumented pool's connections, and closes with it
test("the wrapped pool is shared, unreported, and closed with it", async () => {
  const [db, recorder] = instrumented();
  assert.equal(db.inner, pool());
  assert.equal(db.metrics, recorder);
  assert.ok(db instanceof Pool);
  await pool().query("SELECT 1");
  assert.deepStrictEqual(recorder.reports, []);
  await db.connection(async () => {
    assert.deepStrictEqual(db.status(), pool().status());
  });

  if (!database) throw new Error("the scratch database was not created");
  const inner = new Pool(database.dsn, { maxSize: 1 });
  await new InstrumentedPool(inner, new NoOpMetrics()).close();
  assert.equal(inner.closed, true);
});

// [spec:pgorm:req:bindings.metrics/test]    LoggingMetrics writes Rust's messages at Rust's levels to the logger it is given
test("LoggingMetrics writes to its logger", async () => {
  const lines: [string, string][] = [];
  const logger = {
    debug: (message: string) => lines.push(["debug", message]),
    warn: (message: string) => lines.push(["warn", message]),
    error: (message: string) => lines.push(["error", message]),
  };
  const db = new InstrumentedPool(pool(), new LoggingMetrics(logger));
  await db.query("SELECT id FROM widget WHERE id = 1");
  await assert.rejects(db.query("SELECT * FROM missing"), DatabaseError);
  assert.deepStrictEqual(lines.map(([level, message]) => [level, message.split(/ in | after /)[0]]), [
    ["debug", "Connection acquired"],
    ["debug", "Query queryAll succeeded"],
    ["debug", "Connection acquired"],
    ["warn", "Query queryAll failed"],
  ]);
  const fingerprint = new QueryContext("queryAll", "SELECT id FROM widget WHERE id = 1").fingerprint;
  assert.ok(lines[1]?.[1].endsWith(`, 1 rows [${fingerprint}]`));
  assert.throws(() => new LoggingMetrics({} as never), /debug/);
});
