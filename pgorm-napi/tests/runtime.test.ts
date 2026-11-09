// The runtime the addon needs and runs on: the two paths no query reaches on
// its own, driven through exports that only debug builds carry — a panic on
// the runtime, and a promise whose Deferred is dropped unsettled on a runtime
// thread and rejected through Neon's drop queue — and a JavaScript runtime
// without the Temporal the module's values need.

import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { test } from "node:test";

import { InternalError, PgormError } from "../lib/index.js";
import { runFixture } from "./support.ts";

interface Probes {
  probePanic?: () => Promise<undefined>;
  probeDropQueue?: () => Promise<undefined>;
}

const native = createRequire(import.meta.url)("../lib/pgorm_napi.node") as Probes;
const release = native.probePanic === undefined && "the probes exist only in debug builds";

// [spec:pgorm:req:napi.promises/test]
// [spec:pgorm:req:bindings.errors+6/test]
// [spec:pgorm:req:napi.errors+3/test]
test("a panic on the runtime rejects its promise with an InternalError", { skip: release }, async () => {
  await assert.rejects(native.probePanic?.() ?? Promise.resolve(), (error: unknown) => {
    assert.ok(error instanceof InternalError);
    assert.ok(error instanceof PgormError);
    assert.match(error.message, /the probe's deliberate panic/);
    return true;
  });
});

// [spec:pgorm:req:napi.exit/test]
test("a promise dropped unsettled on a runtime thread is rejected through the drop queue", { skip: release }, async () => {
  // The queue does not keep the event loop alive, and Node.js would finish
  // the loop with its deliveries pending, so a timer holds it open here.
  const hold = setInterval(() => {}, 60_000);
  try {
    const dropped = Array.from({ length: 100 }, () => native.probeDropQueue?.() ?? Promise.resolve());
    for (const outcome of await Promise.allSettled(dropped)) {
      assert.equal(outcome.status, "rejected");
      assert.match(String((outcome as PromiseRejectedResult).reason), /dropped without being settled/);
    }
  } finally {
    clearInterval(hold);
  }
});

// [spec:pgorm:req:napi.runtimes+1/test]
test("a runtime without Temporal cannot load the module, and is told why", async () => {
  const outcome = await runFixture("no-temporal", 30_000);
  assert.equal(outcome.code, 0, outcome.stderr);
  assert.match(outcome.stdout, /^pgorm-napi needs a runtime with Temporal as a global: Node\.js 26 or later/);
});
