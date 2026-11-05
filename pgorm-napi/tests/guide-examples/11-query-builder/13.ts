// Example 13 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, col, GroupingElement, select, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const query = select(col("bakery_id"), col("gluten_free"), call("count", col("id")).as("cakes"))
    .from(new Table("cake"))
    .groupBy(GroupingElement.rollup(col("bakery_id"), col("gluten_free")))
    .orderBy(col("bakery_id").asc(), col("gluten_free").asc());

  assert.equal(
    query.inspect().sql,
    [
      `SELECT "bakery_id", "gluten_free", COUNT("id") AS "cakes" FROM "cake"`,
      `GROUP BY ROLLUP ("bakery_id", "gluten_free")`,
      `ORDER BY "bakery_id" ASC, "gluten_free" ASC`,
    ].join(" "),
  );

  const rows = await pool.query(query);
  assert.deepEqual(rows.map((row) => [row.bakery_id, row.gluten_free, row.cakes]), [
    [1, false, 1n],
    [1, true, 2n],
    [1, null, 3n],
    [2, false, 1n],
    [2, true, 1n],
    [2, null, 2n],
    [null, null, 5n],
  ]);
}
