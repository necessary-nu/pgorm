// Example 13 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Order } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const next = Order.select("id")
    .orderBy(Order.col("placedAt").asc())
    .limit(1)
    .lock("update", { wait: "skipLocked" });
  assert.equal(
    next.inspect().sql,
    `SELECT "order"."id" FROM "order" ORDER BY "order"."placed_at" ASC LIMIT $1 FOR UPDATE SKIP LOCKED`,
  );

  const taken = await pool.transaction(async (tx) => next.one(tx));
  assert.deepEqual(taken, { id: 1 });
}
