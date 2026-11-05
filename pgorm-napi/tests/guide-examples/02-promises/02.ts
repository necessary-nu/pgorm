// Example 2 of guide/02-promises.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { LifecycleError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using db = await pool.acquire();
  const slow = db.execute("SELECT pg_sleep(0.2)");
  await assert.rejects(db.execute("SELECT 1"), LifecycleError);
  await slow;

  // Once the first has settled, the connection takes the next.
  assert.equal((await db.one("SELECT 1 AS one")).one, 1);
}
