// Example 5 of guide/18-listen-notify.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConnectionError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using listener = await pool.listener({ capacity: 2 });
  await listener.listen("orders");
  await pool.transaction(async (tx) => {
    for (const order of [3, 4, 5]) await tx.notify("orders", `order ${order} placed`);
  });

  // Each `listen` is a round trip, answered after the listener has been sent
  // what is pending, so this waits until the overflow has closed it.
  const deadline = AbortSignal.timeout(5000);
  for (;;) {
    try {
      await listener.listen("orders", { signal: deadline });
    } catch {
      break;
    }
  }

  const first = await listener.next();
  const second = await listener.next();
  assert.ok(!first.done && !second.done);
  assert.deepEqual([first.value.payload, second.value.payload], ["order 3 placed", "order 4 placed"]);
  await assert.rejects(listener.next(), ConnectionError);
  assert.equal((await listener.next()).done, true);
}
