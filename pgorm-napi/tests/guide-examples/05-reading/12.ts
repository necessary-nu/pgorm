// Example 12 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, select } from "@necessary/pgorm";
import { Bakery, Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cakes = call("count", Cake.col("id"));
  const busy = select(Bakery.col("name"), cakes.as("cakes"))
    .from(Cake.table)
    .join(Bakery.table, Cake.col("bakeryId").eq(Bakery.col("id")))
    .groupBy(Bakery.col("name"))
    .having(cakes.gt(2));
  assert.deepEqual(await pool.query(busy), [{ name: "Sunrise", cakes: 3n }]);

  const glutenFree = await Cake.find().where(Cake.col("glutenFree").eq(true)).count(pool);
  assert.equal(glutenFree, 3);
}
