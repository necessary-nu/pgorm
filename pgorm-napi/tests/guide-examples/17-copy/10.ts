// Example 10 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, copyFrom, copyTo, Decimal } from "@necessary/pgorm";
import { OrderLine } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const bakeries = copyFrom("bakery", "binary").columns(["name", "city"]);
  await assert.rejects(pool.copyInRows(bakeries, [["Daybreak", "Trondheim"], ["Dusk"]]), ConstructionError);
  assert.equal(await pool.copyInRows(bakeries, [["Daybreak", "Trondheim"]]), 1);

  const prices = [];
  const lines = copyTo(OrderLine.select("price").where(OrderLine.col("orderId").eq(1)).statement, "binary");
  for await (const [price] of await pool.copyOutRows(lines)) prices.push(String(price as Decimal));
  assert.deepEqual(prices.sort(), ["3.20", "4.50"]);
}
