// An abort that fires after the addon finished an operation, but before its
// promise settled, in either runtime. Each test holds this thread between the
// call and the abort, as a busy runner or a garbage collection does, so the
// runtime finishes the work first and its result waits behind the abort.
// What a test asserts holds whichever side wins: the call rejects with the
// signal's reason, and what it opened is released, a transaction ends with
// nothing committed, and a notification is not lost.

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import { copyFrom, copyTo, Pool } from "../lib/index.js";
import { scratchDatabase } from "./support.ts";

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
  database = await scratchDatabase("pgorm_napi_late_aborts");
  shared = new Pool(scratch(), { maxSize: 4 });
  await pool().execute("CREATE TABLE items (id int4 PRIMARY KEY, note text)");
});

beforeEach(async () => {
  await pool().execute("TRUNCATE items");
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

/** Blocks this thread long enough for the runtime to finish a round trip. */
function hold(): void {
  const until = performance.now() + 50;
  while (performance.now() < until);
}

/**
 * Start `call` with a fresh signal, hold the thread, abort, and assert the
 * call rejects with the abort's reason.
 */
async function abortedLate<T>(call: (signal: AbortSignal) => Promise<T>): Promise<void> {
  const cut = new AbortController();
  const reason = new Error("aborted late");
  const pending = call(cut.signal);
  hold();
  cut.abort(reason);
  await assert.rejects(pending, (error: unknown) => error === reason);
}

async function ids(): Promise<number[]> {
  return (await pool().query("SELECT id FROM items ORDER BY id")).map((row) => row.id as number);
}

// [spec:pgorm:req:napi.cancellation+2/test]
test("a statement aborted after it ran rejects, its result discarded", async () => {
  await abortedLate((signal) => pool().query("SELECT 1 AS n", [], { signal }));
  await using connection = await pool().acquire();
  await abortedLate((signal) => connection.execute("INSERT INTO items VALUES (1, 'ran')", [], { signal }));
  await abortedLate((signal) => connection.ping({ signal }));
  assert.equal((await pool().one("SELECT 2 AS n")).n, 2);
});

// [spec:pgorm:req:napi.cancellation+2/test]
// [spec:pgorm:req:bindings.cancellation/test]
test("a statement in a transaction aborted after it ran ends the transaction, nothing committed", async () => {
  const connection = await pool().acquire();
  await assert.rejects(
    connection.transaction(async (tx) => {
      await tx.execute("INSERT INTO items VALUES (1, 'first')");
      await abortedLate((signal) => tx.execute("INSERT INTO items VALUES (2, 'second')", [], { signal }));
      assert.equal(tx.closed, true);
      throw new Error("the transaction ended");
    }),
    /the transaction ended/,
  );
  await connection.close();
  assert.deepEqual(await ids(), []);
});

// [spec:pgorm:req:napi.cancellation+2/test]
test("a transaction aborted after it began ends before its callback runs", async () => {
  const connection = await pool().acquire();
  let ran = false;
  await abortedLate((signal) =>
    connection.transaction(async (tx) => {
      ran = true;
      await tx.execute("INSERT INTO items VALUES (1, 'began')");
    }, { signal })
  );
  assert.equal(ran, false);
  await connection.close();
  assert.deepEqual(await ids(), []);
});

// [spec:pgorm:req:napi.cancellation+2/test]
test("a connection acquired after its abort goes back to the pool", async () => {
  await using single = new Pool(scratch(), { maxSize: 1 });
  await abortedLate((signal) => single.acquire({ signal }));
  await using again = await single.acquire({ signal: AbortSignal.timeout(5_000) });
  assert.equal((await again.one("SELECT 3 AS n")).n, 3);
});

// [spec:pgorm:req:napi.cancellation+2/test]
test("a COPY aborted after it ran rejects, and in a transaction commits nothing", async () => {
  await abortedLate((signal) => pool().copyOut(copyTo("items", "text"), { signal }));
  const data = new TextEncoder().encode("1\tcopied\n");
  const connection = await pool().acquire();
  await assert.rejects(
    connection.transaction(async (tx) => {
      await abortedLate((signal) => tx.copyIn(copyFrom("items", "text"), data, { signal }));
      throw new Error("the transaction ended");
    }),
    /the transaction ended/,
  );
  await connection.close();
  assert.deepEqual(await ids(), []);
});

// [spec:pgorm:req:napi.cancellation+2/test]
test("a row stream's pull aborted after it ran rejects and closes the stream", async () => {
  const cut = new AbortController();
  const stream = pool().stream("SELECT generate_series(1, 3) AS n", [], { signal: cut.signal });
  assert.equal((await stream.next()).value?.n, 1);
  const reason = new Error("aborted late");
  const pending = stream.next();
  hold();
  cut.abort(reason);
  await assert.rejects(pending, (error: unknown) => error === reason);
  assert.equal(stream.closed, true);
  assert.equal((await pool().one("SELECT 4 AS n")).n, 4);
});

// [spec:pgorm:req:napi.cancellation+2/test]
// [spec:pgorm:req:bindings.listen+2/test]
test("a listener's pull aborted after a notification arrived keeps it for the next pull", async () => {
  await using listener = await pool().listener();
  await listener.listen("napi_late");
  await pool().notify("napi_late", "kept");
  await abortedLate((signal) => listener.next({ signal }));
  assert.equal(listener.closed, false);
  const next = await listener.next({ signal: AbortSignal.timeout(10_000) });
  assert.equal(next.done, false);
  assert.equal(next.value?.payload, "kept");
  await abortedLate((signal) => pool().listener({ signal }));
});
