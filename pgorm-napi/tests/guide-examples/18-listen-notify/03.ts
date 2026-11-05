// Example 3 of guide/18-listen-notify.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using listener = await pool.listener();
  await listener.listen("Orders");

  await pool.notify("orders", "not heard: another channel");
  await pool.notify("Orders", "heard");
  const heard = await listener.next({ signal: AbortSignal.timeout(5000) });
  assert.ok(!heard.done);
  assert.equal(heard.value.payload, "heard");

  await assert.rejects(listener.listen("o".repeat(64)), ConstructionError);
}
