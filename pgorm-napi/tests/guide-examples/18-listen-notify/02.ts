// Example 2 of guide/18-listen-notify.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { LifecycleError, Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  // `url` names the database; this pool only ever listens.
  await using listening = new Pool(url, { maxSize: 1 });
  const listener = await listening.listener();
  await listener.listen("deliveries");
  await listener.unlistenAll();
  assert.equal(listening.status().size, 0);

  // Closing the listener closes its connection.
  await listener.close();
  await assert.rejects(listener.listen("deliveries"), LifecycleError);
}
