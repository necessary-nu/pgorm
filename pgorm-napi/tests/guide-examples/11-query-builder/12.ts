// Example 12 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, col, FrameType, select, Table, Window } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const running = new Window()
    .partitionBy(col("bakery_id"))
    .orderBy(col("price").asc())
    .frame(FrameType.rows.unboundedPreceding().andCurrentRow());
  const query = select(col("name"), call("sum", col("price")).over(running).as("running"))
    .from(new Table("cake"))
    .where(col("bakery_id").eq(1))
    .orderBy(col("price").asc());

  assert.equal(
    query.inspect().sql,
    [
      `SELECT "name", SUM("price") OVER ( PARTITION BY "bakery_id" ORDER BY "price" ASC`,
      `ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW ) AS "running"`,
      `FROM "cake" WHERE "bakery_id" = $1 ORDER BY "price" ASC`,
    ].join(" "),
  );

  const rows = await pool.query(query);
  assert.deepEqual(rows.map((row) => [row.name, String(row.running)]), [
    ["Lemon Drizzle", "3.20"],
    ["Chocolate Fudge", "7.70"],
    ["Chocolate Torte", "13.70"],
  ]);
}
