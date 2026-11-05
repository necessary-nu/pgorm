// Example 7 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, select, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const query = select(col("name"))
    .from(new Table("cake"))
    .where(col("id").isIn(select(col("cake_id")).from(new Table("order_line")).where(col("quantity").gt(1))))
    .where(col("price").cast("integer").lt(10))
    .orderBy(col("name").collate("C").asc());

  assert.equal(
    query.inspect().sql,
    [
      `SELECT "name" FROM "cake"`,
      `WHERE "id" IN (SELECT "cake_id" FROM "order_line" WHERE "quantity" > $1)`,
      `AND CAST("price" AS integer) < $2`,
      `ORDER BY ("name" COLLATE "C") ASC`,
    ].join(" "),
  );
  assert.deepEqual(await pool.query(query), [{ name: "Chocolate Fudge" }]);
}
