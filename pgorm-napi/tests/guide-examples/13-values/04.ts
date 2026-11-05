// Example 4 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, select } from "@necessary/pgorm";
import { OrderLine } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const total = OrderLine.col("price").mul(OrderLine.col("quantity"));

  const lines = await pool.query(
    select(total.as("total"))
      .from(OrderLine.table)
      .where(OrderLine.col("orderId").eq(1))
      .orderBy(OrderLine.col("id").asc()),
  );
  assert.deepEqual(lines.map((line) => String(line.total)), ["9.00", "3.20"]);

  const order = await pool.one(
    select(call("sum", total).as("total")).from(OrderLine.table).where(OrderLine.col("orderId").eq(1)),
  );
  assert.equal(String(order.total), "12.20");
}
