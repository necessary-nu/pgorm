// Example 10 of guide/20-configuration.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { LifecycleError, Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  const closing = new Pool(url);
  const db = await closing.acquire();
  await closing.close();
  assert.equal(closing.closed, true);

  await assert.rejects(closing.acquire(), LifecycleError);
  await assert.rejects(db.one("SELECT 1 AS one"), LifecycleError);
}
