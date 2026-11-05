// Example 1 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery, Cake, bakeryCakes, cakeBakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const sunrise = await Bakery.findByKey({ id: 1 }).one(pool);
  const cakes = await bakeryCakes.find(sunrise).orderBy(Cake.col("price").asc()).all(pool);
  assert.deepEqual(cakes.map((cake) => cake.name), ["Lemon Drizzle", "Chocolate Fudge", "Chocolate Torte"]);

  const lemonDrizzle = cakes[0]!;
  const bakery = await cakeBakery.find(lemonDrizzle).one(pool);
  assert.equal(bakery.name, "Sunrise");
}
