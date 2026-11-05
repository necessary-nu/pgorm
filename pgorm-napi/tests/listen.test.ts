// LISTEN and NOTIFY against a live server, in either runtime: a pool's
// listener receiving what pooled connections and transactions send, the
// channel names it takes, how its iteration ends, and what an abort, a busy
// pull and a closing pool do to it.
//
// A notification that must not arrive is never waited for: a later one on a
// channel still listened on fences it, since a session receives
// notifications in commit order.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  ConnectionError,
  ConstructionError,
  DatabaseError,
  LifecycleError,
  type Listener,
  type Notification,
  Pool,
} from "../lib/index.js";
import { runFixture, scratchDatabase } from "./support.ts";

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
  database = await scratchDatabase("pgorm_napi_listen");
  shared = new Pool(scratch(), { maxSize: 4 });
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

/** How long a notification that must arrive is waited for. */
const ARRIVAL = 10_000;

async function next(listener: Listener): Promise<Notification> {
  const result = await listener.next({ signal: AbortSignal.timeout(ARRIVAL) });
  assert.equal(result.done, false, "the listener has not ended");
  return result.value as Notification;
}

async function heard(listener: Listener): Promise<[string, string]> {
  const notification = await next(listener);
  return [notification.channel, notification.payload];
}

function databaseError(sqlstate: string) {
  return (error: unknown) => error instanceof DatabaseError && error.sqlstate === sqlstate;
}

/** The sessions of this database whose last statement was `sql`. */
async function sessions(sql: string): Promise<number> {
  const row = await pool().one(
    "SELECT count(*)::int AS n FROM pg_stat_activity WHERE datname = current_database() AND query = $1",
    [sql],
  );
  return row.n as number;
}

/** Resolve once no session of this database last ran `sql`, failing past the deadline. */
async function sessionEnds(sql: string): Promise<void> {
  const deadline = Date.now() + ARRIVAL;
  while ((await sessions(sql)) !== 0) {
    assert.ok(Date.now() < deadline, `a session still last ran ${sql}`);
  }
}

// [spec:pgorm:req:bindings.listen+2/test]
test("a pooled transaction's notification arrives at commit with its sender's process id", async () => {
  await using listener = await pool().listener();
  await listener.listen("napi_jobs");
  await using connection = await pool().acquire();
  const sender = (await connection.one("SELECT pg_backend_pid() AS pid")).pid;
  await connection.transaction(async (transaction) => {
    await transaction.notify("napi_jobs", "42");
  });
  const notification = await next(listener);
  assert.deepEqual({ ...notification }, { channel: "napi_jobs", payload: "42", processId: sender });
  assert.ok(Object.isFrozen(notification));
});

// [spec:pgorm:req:bindings.listen+2/test]
test("a rolled-back transaction's notification never arrives", async () => {
  await using listener = await pool().listener();
  await listener.listen("napi_rolled_back");
  await using connection = await pool().acquire();
  const undo = new Error("roll the notification back");
  await assert.rejects(
    connection.transaction(async (transaction) => {
      await transaction.notify("napi_rolled_back", "discarded");
      throw undo;
    }),
    (error: unknown) => error === undo,
  );
  await connection.notify("napi_rolled_back", "fence");
  assert.deepEqual(await heard(listener), ["napi_rolled_back", "fence"]);
});

// [spec:pgorm:req:bindings.listen+2/test]
test("notifications arrive in commit order, a repeated payload once", async () => {
  await using listener = await pool().listener();
  await listener.listen("napi_order");
  await using first = await pool().acquire();
  await using second = await pool().acquire();
  await first.transaction(async (early) => {
    await second.transaction(async (late) => {
      for (const payload of ["1", "2", "1", "3"]) await early.notify("napi_order", payload);
      await late.notify("napi_order", "first committed");
    });
  });
  const received = [];
  for (let index = 0; index < 4; index += 1) received.push((await next(listener)).payload);
  assert.deepEqual(received, ["first committed", "1", "2", "3"]);
});

// [spec:pgorm:req:bindings.listen+2/test]
test("unlisten and unlistenAll stop delivery", async () => {
  await using listener = await pool().listener();
  await listener.listen("napi_unlisten");
  await listener.listen("napi_fence");
  await pool().notify("napi_unlisten", "heard");
  assert.deepEqual(await heard(listener), ["napi_unlisten", "heard"]);

  await listener.unlisten("napi_unlisten");
  await pool().notify("napi_unlisten", "unheard");
  await pool().notify("napi_fence", "1");
  assert.deepEqual(await heard(listener), ["napi_fence", "1"]);

  await listener.unlistenAll();
  await listener.listen("napi_after");
  await pool().notify("napi_fence", "unheard");
  await pool().notify("napi_after", "2");
  assert.deepEqual(await heard(listener), ["napi_after", "2"]);
});

// [spec:pgorm:req:bindings.listen+2/test]
test("a channel name is exact: case, spaces and quotes kept", async () => {
  await using listener = await pool().listener();
  const odd = 'Napi Mixed "quoted"; DROP';
  await listener.listen(odd);
  await listener.listen("napi_exact_fence");
  await pool().notify(odd.toLowerCase(), "another channel");
  await pool().notify(odd, "exact");
  await pool().notify("napi_exact_fence", "1");
  assert.deepEqual(await heard(listener), [odd, "exact"]);
  assert.deepEqual(await heard(listener), ["napi_exact_fence", "1"]);
  assert.equal(await sessions('LISTEN "napi_exact_fence"'), 1, "the LISTEN was sent quoted");
});

// [spec:pgorm:req:bindings.listen+2/test]
test("a channel name past 63 bytes is a ConstructionError, sent nothing", async () => {
  await using listener = await pool().listener();
  const longest = `é${"c".repeat(61)}`;
  const tooLong = `${longest}c`;
  await assert.rejects(listener.listen(tooLong), ConstructionError);
  await assert.rejects(listener.unlisten(tooLong), ConstructionError);
  assert.equal(listener.closed, false);
  await listener.listen(longest);
  await pool().notify(longest, "whole");
  assert.deepEqual(await heard(listener), [longest, "whole"]);
  await assert.rejects(pool().notify(tooLong, "x"), databaseError("22023"));
  await assert.rejects(listener.listen(""), databaseError("42601"));
  await assert.rejects(listener.listen(7 as unknown as string), TypeError);
  await assert.rejects(pool().notify("napi_typed", 7 as unknown as string), TypeError);
});

// [spec:pgorm:req:bindings.listen+2/test]
test("the payload bound is the server's: 7999 bytes arrive, 8000 are refused", async () => {
  await using listener = await pool().listener();
  await listener.listen("napi_payload");
  const largest = "p".repeat(7999);
  await pool().notify("napi_payload", largest);
  assert.deepEqual(await heard(listener), ["napi_payload", largest]);
  await assert.rejects(pool().notify("napi_payload", "p".repeat(8000)), databaseError("22023"));
});

// [spec:pgorm:req:bindings.listen+2/test]
test("a terminated backend ends iteration with its error, after what had arrived", async () => {
  const listener = await pool().listener();
  await listener.listen("napi_doomed");
  const backend = await pool().one(
    "SELECT pid FROM pg_stat_activity WHERE datname = current_database() AND query = 'LISTEN \"napi_doomed\"'",
  );
  const pid = backend.pid as number;
  await pool().notify("napi_doomed", "1");
  assert.deepEqual(await heard(listener), ["napi_doomed", "1"]);
  await pool().notify("napi_doomed", "2");
  await pool().notify("napi_doomed", "3");
  // The listening session sends what it has pending before it answers, so
  // both are waiting once this resolves.
  await listener.listen("napi_doomed");
  const terminated = await pool().execute(
    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE pid = $1 AND datname = current_database()",
    [pid],
  );
  assert.equal(terminated, 1, "the listening session is in this file's database");

  assert.deepEqual(await heard(listener), ["napi_doomed", "2"]);
  assert.deepEqual(await heard(listener), ["napi_doomed", "3"]);
  await assert.rejects(listener.next({ signal: AbortSignal.timeout(ARRIVAL) }), databaseError("57P01"));
  assert.equal(listener.closed, true);
  assert.deepEqual(await listener.next(), { done: true, value: undefined });
  await assert.rejects(listener.listen("napi_doomed"), LifecycleError);
});

// [spec:pgorm:req:bindings.listen+2/test]
test("past its capacity the listener closes with a ConnectionError", async () => {
  const listener = await pool().listener({ capacity: 2 });
  await listener.listen("napi_burst");
  await pool().transaction(async (transaction) => {
    for (const payload of ["1", "2", "3", "4"]) await transaction.notify("napi_burst", payload);
  });
  // Nothing pulls until the listener's connection has closed, so the overflow
  // is decided by the capacity alone. Each probe is a round trip the
  // listening session answers after sending what it has pending.
  const deadline = Date.now() + ARRIVAL;
  for (;;) {
    try {
      await listener.listen("napi_probe");
    } catch (error) {
      assert.ok(error instanceof ConnectionError, String(error));
      break;
    }
    assert.ok(Date.now() < deadline, "the listener closes in time");
  }
  assert.deepEqual(await heard(listener), ["napi_burst", "1"]);
  assert.deepEqual(await heard(listener), ["napi_burst", "2"]);
  await assert.rejects(listener.next(), (error: unknown) => {
    return error instanceof ConnectionError && /2 were waiting unread/.test(error.message);
  });
  assert.deepEqual(await listener.next(), { done: true, value: undefined });
  await assert.rejects(pool().listener({ capacity: 0 }), RangeError);
  await assert.rejects(pool().listener({ capacity: 1.5 }), RangeError);
});

// [spec:pgorm:req:bindings.listen+2/test]
test("one pull runs at a time, and listen runs while it waits", async () => {
  await using listener = await pool().listener();
  const first = listener.next({ signal: AbortSignal.timeout(ARRIVAL) });
  await assert.rejects(listener.next(), LifecycleError);
  await listener.listen("napi_busy");
  await pool().notify("napi_busy", "1");
  const result = await first;
  assert.equal(result.done, false);
  assert.equal(result.value?.payload, "1");
});

// [spec:pgorm:req:bindings.listen+2/test]
// [spec:pgorm:req:bindings.cancellation/test]
test("an aborted pull leaves the listener open; an aborted listen closes it", async () => {
  await using listener = await pool().listener();
  await listener.listen("napi_aborted");
  const controller = new AbortController();
  const pending = listener.next({ signal: controller.signal });
  controller.abort(new Error("stop waiting"));
  await assert.rejects(pending, /stop waiting/);
  assert.equal(listener.closed, false);
  await pool().notify("napi_aborted", "kept");
  assert.deepEqual(await heard(listener), ["napi_aborted", "kept"]);

  const signal = AbortSignal.abort(new Error("never sent"));
  await assert.rejects(listener.listen("napi_unsent", { signal }), /never sent/);
  assert.equal(listener.closed, false, "a signal fired before the call sends nothing");

  const cut = new AbortController();
  const listening = listener.listen("napi_uncertain", { signal: cut.signal });
  cut.abort(new Error("cut off"));
  await assert.rejects(listening, /cut off/);
  assert.equal(listener.closed, true);
  await assert.rejects(listener.listen("napi_aborted"), LifecycleError);
});

// [spec:pgorm:req:bindings.listen+2/test]
// [spec:pgorm:req:bindings.cancellation/test]
test("an abort before a finished listen settles still rejects it and closes the listener", async () => {
  await using listener = await pool().listener();
  const cut = new AbortController();
  const listening = listener.listen("napi_settled_late", { signal: cut.signal });
  // Holding this thread, as a busy runner does, lets the runtime finish the
  // LISTEN before the abort reaches it.
  const until = performance.now() + 50;
  while (performance.now() < until);
  cut.abort(new Error("cut off late"));
  await assert.rejects(listening, /cut off late/);
  assert.equal(listener.closed, true);
});

// [spec:pgorm:req:bindings.listen+2/test]
test("closing the pool closes its listeners, ending a pull in flight", async () => {
  const closing = new Pool(scratch(), { maxSize: 1 });
  const listener = await closing.listener();
  await listener.listen("napi_pool_closed");
  const pending = assert.rejects(listener.next(), LifecycleError);
  await closing.close();
  await pending;
  assert.equal(listener.closed, true);
  await assert.rejects(listener.listen("napi_pool_closed"), LifecycleError);
  await sessionEnds('LISTEN "napi_pool_closed"');
  await assert.rejects(closing.listener(), LifecycleError);
});

// [spec:pgorm:req:bindings.listen+2/test]
test("closing a listener ends its session, and its waiting pull as done; leaving a for await loop closes it", async () => {
  const closed = await pool().listener();
  await closed.listen("napi_closed");
  const pending = closed.next();
  await closed.listen("napi_closed");
  await closed.close();
  assert.deepEqual(await pending, { done: true, value: undefined });
  assert.deepEqual(await closed.next(), { done: true, value: undefined });
  await assert.rejects(closed.unlistenAll(), LifecycleError);
  await sessionEnds('LISTEN "napi_closed"');

  const listener = await pool().listener();
  await listener.listen("napi_loop");
  await pool().notify("napi_loop", "1");
  for await (const notification of listener) {
    assert.equal(notification.payload, "1");
    break;
  }
  assert.equal(listener.closed, true);
  await sessionEnds('LISTEN "napi_loop"');
});

// [spec:pgorm:req:bindings.listen+2/test]
// [spec:pgorm:req:napi.exit/test]
test("a collected listener releases its session, and an open one holds no process up", async () => {
  const outcome = await runFixture("listening", 30_000, { PGORM_TEST_DSN: scratch() }, { exposeGc: true });
  assert.equal(outcome.hung, false, `killed without exiting:\n${outcome.stderr}`);
  assert.equal(outcome.stderr, "");
  assert.equal(outcome.code, 0);
  assert.equal(outcome.stdout, "listening 1\ncollected 0\nholding\n");
});
