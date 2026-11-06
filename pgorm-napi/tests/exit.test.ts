// A process that has finished its work exits on its own, in either runtime.
//
// The addon's promises are settled through Node-API threadsafe functions, and
// its queries run on a tokio runtime whose threads outlive any one call. Either
// could keep a finished process alive or crash it as it exits. Each case runs a
// fixture in a fresh process of the runtime running the suite and holds it to
// exiting by itself, before a deadline, with the status it should have and
// nothing on stderr it should not.

import assert from "node:assert/strict";
import { test } from "node:test";

import { type Outcome, runFixture } from "./support.ts";

/** Long enough for a slow machine to start a runtime and connect; far short of hanging. */
const DEADLINE = 30_000;

function exitedCleanly(outcome: Outcome, stdout: string): void {
  assert.equal(outcome.hung, false, `killed after ${DEADLINE} ms without exiting:\n${outcome.stderr}`);
  assert.equal(outcome.signal, null, `ended by ${outcome.signal}:\n${outcome.stderr}`);
  assert.equal(outcome.stderr, "");
  assert.equal(outcome.code, 0);
  assert.equal(outcome.stdout, stdout);
}

// [spec:pgorm:req:napi.exit/test]
test("a process that only loads the addon exits", async () => {
  const outcome = await runFixture("idle", DEADLINE);
  exitedCleanly(outcome, outcome.stdout);
  assert.match(outcome.stdout, /^loaded \d+\.\d+\.\d+\n$/);
});

// [spec:pgorm:req:napi.exit/test]
test("a process whose query has settled exits", async () => {
  exitedCleanly(await runFixture("settled", DEADLINE), "settled 2\n");
});

// [spec:pgorm:req:napi.exit/test]
test("a process whose hundred concurrent queries have settled exits", async () => {
  exitedCleanly(await runFixture("concurrent", DEADLINE), "settled 5050\n");
});

// [spec:pgorm:req:napi.exit/test]
test("a query still in flight when the script ends keeps the process up until it settles", async () => {
  const outcome = await runFixture("pending", DEADLINE);
  exitedCleanly(outcome, "script ended\nsettled 2\n");
  assert.ok(outcome.elapsed >= 1000, "the process waited for the one-second query");
});

// [spec:pgorm:req:napi.exit/test]
test("a process whose promises the drop queue rejected exits once nothing else holds it", async () => {
  const outcome = await runFixture("dropped", DEADLINE, { DROP_MODE: "held" });
  exitedCleanly(outcome, outcome.stdout === "release build\n" ? outcome.stdout : "rejected 10\n");
});

// [spec:pgorm:req:napi.exit/test]
test("promises left to the drop queue never hold a process open", async () => {
  const outcome = await runFixture("dropped", DEADLINE, { DROP_MODE: "unheld" });
  exitedCleanly(outcome, outcome.stdout);
  assert.match(outcome.stdout, /^(release build|rejected (\d|10))\n$/);
});

// [spec:pgorm:req:napi.exit/test]
// [spec:pgorm:req:napi.lifecycle+3/test]
test("a process that ends holding a transaction and a stream open exits", async () => {
  exitedCleanly(await runFixture("holding", DEADLINE), "holding 1 false\n");
});

// [spec:pgorm:req:napi.exit/test]
test("a process whose rejected query was handled exits", async () => {
  exitedCleanly(await runFixture("rejected", DEADLINE), "rejected 22003\n");
});

// [spec:pgorm:req:napi.exit/test]
test("an unhandled rejection ends the process with a failure status, not a hang or a crash", async () => {
  const outcome = await runFixture("unhandled", DEADLINE);
  assert.equal(outcome.hung, false, "killed without exiting");
  assert.equal(outcome.signal, null);
  assert.equal(outcome.code, 1);
  assert.match(outcome.stderr, /DatabaseError/);
  assert.match(outcome.stderr, /integer out of range/);
  assert.doesNotMatch(outcome.stderr, /panicked|Segmentation fault|Abort|FATAL/i);
});

// [spec:pgorm:req:napi.exit/test]
test("exiting with a query still running neither waits for it nor crashes", async () => {
  const outcome = await runFixture("abandoned", DEADLINE);
  exitedCleanly(outcome, "exiting\n");
  assert.ok(outcome.elapsed < 8000, `exited after ${outcome.elapsed} ms, not before the ten-second query`);
});

// [spec:pgorm:req:napi.runtime+1/test]
// [spec:pgorm:req:bindings.runtime/test]
test("a worker thread's instance runs queries beside the main thread's, and both exit", async () => {
  const outcome = await runFixture("worker", DEADLINE, { WORKER_MODE: "finish" });
  exitedCleanly(outcome, "main 2\nworker 3\nworker exited\n");
});

// [spec:pgorm:req:napi.exit/test]
test("terminating a worker whose query is in flight tears its instance down cleanly", async () => {
  const outcome = await runFixture("worker", DEADLINE, { WORKER_MODE: "terminate" });
  exitedCleanly(outcome, "main 2\nstarted\nworker exited\nmain 4\n");
});

// [spec:pgorm:req:napi.exit/test]
test("exiting while settlements stream in from the runtime crashes nothing", async () => {
  for (let round = 0; round < 5; round += 1) {
    exitedCleanly(await runFixture("racing", DEADLINE), "racing\n");
  }
});
