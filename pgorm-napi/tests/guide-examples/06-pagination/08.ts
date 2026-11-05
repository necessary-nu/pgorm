// Example 8 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const prices: string[] = [];
  for await (const cake of Cake.find().orderBy(Cake.col("id").asc()).stream(pool)) {
    prices.push(cake.price.toString());
  }
  assert.deepEqual(prices, ["4.50", "3.20", "3.80", "2.90", "6.00"]);

  const names: string[] = [];
  for await (const row of Cake.select("name").where(Cake.col("bakeryId").eq(2)).orderBy(Cake.col("name").asc()).stream(pool)) {
    names.push(row.name);
  }
  assert.deepEqual(names, ["Carrot Cake", "Vanilla Sponge"]);

  const bakers: unknown[] = [];
  for await (const row of pool.stream("SELECT name FROM baker WHERE bakery_id = $1 ORDER BY name", [1])) {
    bakers.push(row.name);
  }
  assert.deepEqual(bakers, ["Ada", "Bo"]);
}
