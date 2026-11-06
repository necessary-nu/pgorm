// Example 2 of guide/18-listen-notify.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { connectListener, LifecycleError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool, url }: GuideContext): Promise<void> {
  // `url` names the database; no pool holds this listener.
  const listener = await connectListener(url, { capacity: 64 });
  await listener.listen("deliveries");
  await pool.notify("deliveries", "flour");
  const { value } = await listener.next();
  assert.equal(value?.payload, "flour");

  // Closing the listener closes its connection.
  await listener.close();
  await assert.rejects(listener.listen("deliveries"), LifecycleError);
}
