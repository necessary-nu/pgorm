// Example 5 of guide/02-promises.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { LifecycleError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const db = await pool.acquire();
  await db.close();
  assert.equal(db.closed, true);
  await assert.rejects(db.execute("SELECT 1"), LifecycleError);

  // A loop that leaves early closes its stream.
  let first: string | undefined;
  for await (const cake of pool.stream("SELECT name FROM cake ORDER BY id")) {
    first = String(cake.name);
    break;
  }
  assert.equal(first, "Chocolate Fudge");
}
