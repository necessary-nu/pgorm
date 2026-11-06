// Example 21 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, select, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const next = select(col("id"))
    .from(new Table("order"))
    .where(col("bakery_id").eq(1))
    .lock("update", { wait: "skipLocked" });

  assert.equal(next.inspect().sql, `SELECT "id" FROM "order" WHERE "bakery_id" = $1 FOR UPDATE SKIP LOCKED`);

  const taken = await pool.transaction((tx) => tx.query(next));
  assert.deepEqual(taken, [{ id: 1 }]);
}
