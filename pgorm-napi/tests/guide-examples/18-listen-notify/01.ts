// Example 1 of guide/18-listen-notify.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using listener = await pool.listener();
  await listener.listen("orders");

  await pool.notify("orders", "order 3 placed");

  const heard = await listener.next({ signal: AbortSignal.timeout(5000) });
  assert.ok(!heard.done);
  assert.equal(heard.value.channel, "orders");
  assert.equal(heard.value.payload, "order 3 placed");
}
