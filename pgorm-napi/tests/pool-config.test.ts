// A pool's shape and manager options — its timeouts, queue mode, recycling,
// tag and hooks — several pools built by tag, a connection discarded, and an
// instrumented pool over a configured one, against a live server in either
// runtime.

import assert from "node:assert/strict";
import { createServer, type Socket } from "node:net";
import { after, before, test } from "node:test";

import {
  ConnectionError,
  ConstructionError,
  type HookConnection,
  InstrumentedPool,
  LifecycleError,
  MultiPool,
  NoOpMetrics,
  Pool,
  type Queryable,
  TimeoutError,
} from "../lib/index.js";
import { deno, scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;

function scratch(): string {
  if (!database) throw new Error("the scratch database was not created");
  return database.dsn;
}

before(async () => {
  database = await scratchDatabase("pgorm_napi_pool_config");
});

after(async () => {
  await database?.drop();
});

async function backend(db: Queryable): Promise<number> {
  return (await db.one("SELECT pg_backend_pid() AS pid")).pid as number;
}

async function setting(db: Queryable, name: string): Promise<string> {
  return (await db.one("SELECT current_setting($1) AS value", [name])).value as string;
}

function timedOut(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof TimeoutError, `expected a TimeoutError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

/**
 * A server that accepts connections and never answers them, so opening one
 * hangs, and what stops it: it reads nothing, so it destroys what it accepted
 * rather than wait to read their end.
 */
async function silentServer(): Promise<[number, () => Promise<void>]> {
  const accepted = new Set<Socket>();
  const server = createServer((socket) => accepted.add(socket));
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (address === null || typeof address === "string") throw new Error("the server has no port");
  return [address.port, async () => {
    for (const socket of accepted) socket.destroy();
    await new Promise((resolve) => server.close(resolve));
  }];
}

/** Whether this runtime may open a socket of its own: Deno runs the suite without `--allow-net`. */
function listens(): boolean {
  if (!deno) return true;
  const permissions = (globalThis as unknown as {
    Deno: { permissions: { querySync(descriptor: { name: "net" }): { state: string } } };
  }).Deno.permissions;
  return permissions.querySync({ name: "net" }).state === "granted";
}

// [spec:pgorm:req:bindings.connections+3/test]
test("waitTimeout ends an acquisition with a TimeoutError naming it", async () => {
  await using small = new Pool(scratch(), { maxSize: 1, waitTimeout: 100 });
  const held = await small.acquire();
  await assert.rejects(small.acquire(), timedOut(/waitTimeout/));
  await held.close();
  assert.equal(await small.ping(), true);
});

// [spec:pgorm:req:bindings.connections+3/test]
test("createTimeout ends an acquisition whose connection never opens", async (context) => {
  if (!listens()) {
    context.skip("the runtime may not listen on a socket, which the silent server needs");
    return;
  }
  const [port, stop] = await silentServer();
  try {
    await using silent = new Pool(`postgres://postgres@127.0.0.1:${port}/postgres?sslmode=disable`, {
      createTimeout: 100,
    });
    await assert.rejects(silent.acquire(), timedOut(/createTimeout/));
  } finally {
    await stop();
  }
});

// [spec:pgorm:req:bindings.connections+3/test]
test("recycleTimeout discards a returned connection whose check outlasts it", async () => {
  await using slow = new Pool(scratch(), { maxSize: 1, recycleTimeout: 100, recycleSql: "SELECT pg_sleep(3)" });
  const first = await slow.acquire();
  const pid = await backend(first);
  await first.close();
  const started = Date.now();
  const second = await slow.acquire();
  assert.notEqual(await backend(second), pid);
  assert.ok(Date.now() - started < 2500, "the check was cut short, not waited out");
  await second.close();
});

// [spec:pgorm:req:bindings.connections+3/test]
test("queueMode lends the idle connection returned first, or with lifo the one returned last", async () => {
  for (const [queueMode, lent] of [["fifo", 0], ["lifo", 1]] as const) {
    await using pool = new Pool(scratch(), { maxSize: 2, queueMode });
    const first = await pool.acquire();
    const second = await pool.acquire();
    const pids = [await backend(first), await backend(second)];
    await first.close();
    await second.close();
    const next = await pool.acquire();
    assert.equal(await backend(next), pids[lent], queueMode);
    await next.close();
  }
  // @ts-expect-error: a pool lends first in first out, or last in first out
  assert.throws(() => new Pool(scratch(), { queueMode: "random" }), TypeError);
});

// [spec:pgorm:req:bindings.connections+3/test]
test("clean recycling resets a session, and recycleSql runs the program's own SQL", async () => {
  await using clean = new Pool(scratch(), { maxSize: 1, recycle: "clean" });
  await clean.connection((db) => db.execute("SET enable_seqscan = off"));
  assert.equal(await setting(clean, "enable_seqscan"), "on");

  await using custom = new Pool(scratch(), {
    maxSize: 1,
    recycleSql: "SELECT set_config('application_name', 'recycled', false)",
  });
  await custom.connection(async (db) => assert.notEqual(await setting(db, "application_name"), "recycled"));
  assert.equal(await setting(custom, "application_name"), "recycled");

  assert.throws(() => new Pool(scratch(), { recycle: "fast", recycleSql: "SELECT 1" }), TypeError);
  assert.throws(() => new Pool(scratch(), { recycleSql: 7 as never }), TypeError);
});

// [spec:pgorm:req:bindings.connections+3/test]
test("a pool reports its tag, or the default-N pgorm names an untagged pool", async () => {
  await using kitchen = new Pool(scratch(), { tag: "kitchen" });
  assert.equal(kitchen.tag, "kitchen");
  await using untagged = new Pool(scratch());
  assert.match(untagged.tag, /^default-\d+$/);
  assert.throws(() => new Pool(scratch(), { tag: "" }), ConstructionError);
  assert.throws(() => new Pool(scratch(), { tag: 7 as never }), TypeError);
});

// [spec:pgorm:req:bindings.connections+3/test]
test("discard closes a connection without returning it, and the pool opens a fresh one", async () => {
  await using single = new Pool(scratch(), { maxSize: 1 });
  const db = await single.acquire();
  const pid = await backend(db);
  await db.discard();
  assert.equal(db.closed, true);
  await assert.rejects(db.one("SELECT 1 AS one"), LifecycleError);
  const fresh = await single.acquire();
  assert.notEqual(await backend(fresh), pid);
  await fresh.close();
});

/** A collector counting the connections an instrumented pool lends. */
class Acquisitions extends NoOpMetrics {
  acquired = 0;

  override recordConnectionAcquired() {
    this.acquired += 1;
  }
}

// [spec:pgorm:req:bindings.metrics/test]    an instrumented pool is the pool it wraps, its configuration included, and a connection it lends discards
// [spec:pgorm:req:bindings.connections+3/test]
test("an InstrumentedPool keeps its pool's tag and hooks, and a connection it lends discards", async () => {
  let created = 0;
  const inner = new Pool(scratch(), {
    maxSize: 1,
    tag: "metered",
    postCreate: async (db) => {
      created += 1;
      await db.execute("SELECT set_config('application_name', $1, false)", ["bakery-metered"]);
    },
  });
  const counted = new Acquisitions();
  await using metered = new InstrumentedPool(inner, counted);
  assert.equal(metered.tag, "metered");
  const db = await metered.acquire();
  assert.equal(await setting(db, "application_name"), "bakery-metered");
  const pid = await backend(db);
  await db.discard();
  assert.equal(db.closed, true);
  const fresh = await metered.acquire();
  assert.notEqual(await backend(fresh), pid);
  assert.equal(created, 2);
  await fresh.close();
  assert.equal(counted.acquired, 2);
});

// [spec:pgorm:req:bindings.multi-pool/test]
test("a MultiPool builds a tagged pool per entry, each shaped by its own options", async () => {
  await using pools = new MultiPool(
    scratch(),
    { web: { maxSize: 8 }, jobs: { maxSize: 2, queueMode: "lifo" } },
    { recycle: "clean" },
  );
  assert.deepEqual(pools.entries().map(([tag, pool]) => [tag, pool.tag]), [["jobs", "jobs"], ["web", "web"]]);
  assert.deepEqual(
    Object.entries(pools.status()).map(([tag, status]) => [tag, status.maxSize]),
    [["jobs", 2], ["web", 8]],
  );
  assert.equal(pools.get("reports"), null);
  const jobs = pools.get("jobs");
  assert.ok(jobs instanceof Pool);
  await jobs.connection((db) => db.execute("SET enable_seqscan = off"));
  assert.equal(await setting(jobs, "enable_seqscan"), "on", "the shared clean recycling reached the pool");
  await pools.close();
  assert.equal(pools.closed, true);
  assert.ok(pools.entries().every(([, pool]) => pool.closed));
});

// [spec:pgorm:req:bindings.multi-pool/test]
test("a MultiPool refuses a shared tag, an empty key and no pools before building any", () => {
  assert.throws(() => new MultiPool(scratch(), {}), ConstructionError);
  assert.throws(() => new MultiPool(scratch(), { "": {} }), ConstructionError);
  // @ts-expect-error: each pool's tag is its key
  assert.throws(() => new MultiPool(scratch(), { web: {} }, { tag: "web" }), TypeError);
  assert.throws(() => new MultiPool(scratch(), { web: { maxSize: 0 } }), ConstructionError);
  assert.throws(() => new MultiPool(scratch(), { web: 8 as never }), TypeError);
});

// [spec:pgorm:req:bindings.pool-hooks/test]
test("postCreate runs statements on the connection before the pool lends it", async () => {
  let seen: HookConnection | undefined;
  await using hooked = new Pool(scratch(), {
    maxSize: 1,
    postCreate: async (db) => {
      seen = db;
      assert.equal(db.recycleCount, 0);
      assert.ok(db.age >= 0);
      await db.execute("SELECT set_config('application_name', $1, false)", ["bakery-hooked"]);
      assert.equal((await db.one("SELECT current_setting('application_name') AS name")).name, "bakery-hooked");
    },
  });
  assert.equal(await setting(hooked, "application_name"), "bakery-hooked");
  assert.ok(seen);
  assert.equal(seen.closed, true);
  await assert.rejects(seen.one("SELECT 1 AS one"), LifecycleError);
});

// [spec:pgorm:req:bindings.pool-hooks/test]
test("a postCreate hook that throws or rejects refuses the connection, naming the hook", async () => {
  for (
    const postCreate of [
      () => {
        throw new RangeError("too old");
      },
      () => Promise.reject(new RangeError("too old")),
    ]
  ) {
    await using refusing = new Pool(scratch(), { postCreate });
    await assert.rejects(refusing.acquire(), (error: unknown) => {
      assert.ok(error instanceof ConnectionError, `expected a ConnectionError, got ${error}`);
      assert.equal(error.message, "the postCreate hook refused the connection: RangeError: too old");
      return true;
    });
  }
  // @ts-expect-error: a hook is a function, not the SQL it would run
  assert.throws(() => new Pool(scratch(), { postCreate: "SELECT 1" }), TypeError);
});

// [spec:pgorm:req:bindings.pool-hooks/test]
test("a recycle hook that refuses discards the connection, and the pool opens another", async () => {
  let checks = 0;
  await using refusing = new Pool(scratch(), {
    maxSize: 1,
    preRecycle: (db) => {
      checks += 1;
      if (db.recycleCount === 0) throw new Error("not this one");
    },
  });
  const first = await refusing.acquire();
  const pid = await backend(first);
  await first.close();
  const second = await refusing.acquire();
  assert.notEqual(await backend(second), pid);
  assert.equal(checks, 1);
  await second.close();

  // The count a hook sees is the recycles the connection passed before this one.
  const recycled: number[] = [];
  await using counting = new Pool(scratch(), {
    maxSize: 1,
    postRecycle: (db) => {
      recycled.push(db.recycleCount);
    },
  });
  for (let checkout = 0; checkout < 3; checkout += 1) await (await counting.acquire()).close();
  assert.deepEqual(recycled, [0, 1]);
});
