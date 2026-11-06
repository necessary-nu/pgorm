// Example 16 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, col, GroupingElement, select, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const query = select(col("bakery_id"), call("grouping", col("bakery_id")).as("total"), call("count", col("id")).as("cakes"))
    .from(new Table("cake"))
    .groupBy(GroupingElement.rollup(col("bakery_id")))
    .orderBy(col("bakery_id").asc());

  assert.ok(query.inspect().sql.startsWith(`SELECT "bakery_id", GROUPING("bakery_id") AS "total"`));
  const rows = await pool.query(query);
  assert.deepEqual(rows.map((row) => [row.bakery_id, row.total, row.cakes]), [[1, 0, 3n], [2, 0, 2n], [null, 1, 5n]]);
}
