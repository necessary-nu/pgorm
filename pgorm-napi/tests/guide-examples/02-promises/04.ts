// Example 4 of guide/02-promises.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const stop = new AbortController();
  const order = pool.transaction(async (tx) => {
    await tx.execute(`INSERT INTO "order" (customer_id, bakery_id, placed_at) VALUES (1, 2, now())`);
    stop.abort(new Error("the customer left"));
    await tx.execute("SELECT pg_sleep(10)", [], { signal: stop.signal });
  });
  await assert.rejects(order, { message: "the customer left" });

  // The insert went with the transaction.
  assert.equal((await pool.one(`SELECT count(*) AS n FROM "order"`)).n, 2n);
}
