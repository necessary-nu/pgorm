// Example 12 of guide/17-copy.md, written by tests/guide.test.ts: edit the
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

  // Every cake, copied into a table of the same columns as it is read.
  await pool.execute("CREATE TABLE cake_copy (LIKE cake)");
  const copied = await pool.copyInRows(copyFrom("cake_copy", "binary"), await pool.copyOutRows(copyTo("cake", "binary")));
  const { differ } = await pool.one(
    "SELECT count(*)::int AS differ FROM (SELECT * FROM cake EXCEPT ALL SELECT * FROM cake_copy) AS d",
  );
  assert.equal(differ, 0);
  assert.ok(copied > 0);
}
