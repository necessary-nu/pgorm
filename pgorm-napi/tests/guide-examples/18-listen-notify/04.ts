// Example 4 of guide/18-listen-notify.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using listener = await pool.listener();
  await listener.listen("orders");

  // Rolled back: never sent.
  await assert.rejects(
    pool.transaction(async (tx) => {
      await tx.notify("orders", "order 3 placed");
      throw new Error("the card was declined");
    }),
    /the card was declined/,
  );

  // Committed: sent at the commit, the repeated payload once.
  await pool.transaction(async (tx) => {
    await tx.notify("orders", "order 4 placed");
    await tx.notify("orders", "order 4 placed");
    await tx.notify("orders", "order 4 paid");
  });

  const payloads: string[] = [];
  for (let read = 0; read < 2; read++) {
    const heard = await listener.next({ signal: AbortSignal.timeout(5000) });
    assert.ok(!heard.done);
    payloads.push(heard.value.payload);
  }
  assert.deepEqual(payloads, ["order 4 placed", "order 4 paid"]);

  // Nothing else came: an aborted pull leaves the listener open.
  await assert.rejects(listener.next({ signal: AbortSignal.timeout(200) }), { name: "TimeoutError" });
  assert.equal(listener.closed, false);
}
