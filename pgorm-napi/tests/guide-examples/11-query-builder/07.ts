// Example 7 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, ReturningRow, Table, update } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const doubled = update(new Table("cake"))
    .set("price", col("price").mul(2))
    .where(col("id").eq(1))
    .returning([ReturningRow.old.col("price").as("was"), ReturningRow.new.col("price").as("now")]);

  assert.equal(
    doubled.inspect().sql,
    `UPDATE "cake" SET "price" = "price" * $1 WHERE "id" = $2 RETURNING old."price" AS "was", new."price" AS "now"`,
  );
  const [row] = await pool.query(doubled);
  assert.deepEqual([String(row?.was), String(row?.now)], ["4.50", "9.00"]);
}
