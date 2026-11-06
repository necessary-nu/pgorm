// Example 6 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Baker, Bakery, Cake, bakeryCakes, cakeBakers, cakeBakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const bakeries = await Bakery.find().orderBy(Bakery.col("id").asc()).all(pool);

  // Each bakery's gluten-free cakes, cheapest first.
  const glutenFree = Cake.find().where(Cake.col("glutenFree").eq(true)).orderBy(Cake.col("price").asc());
  const cakes = await bakeryCakes.load(pool, bakeries, { query: glutenFree });
  assert.deepEqual(cakes.map((list) => list.map((cake) => cake.name)), [["Lemon Drizzle", "Chocolate Torte"], ["Vanilla Sponge"]]);

  // Each of Sunrise's cakes, with its bakery and its bakers.
  const sunriseCakes = cakes[0]!;
  const theirBakery = await cakeBakery.load(pool, sunriseCakes);
  assert.ok(theirBakery.every((bakery) => bakery?.name === "Sunrise"));

  const bakers = await cakeBakers.load(pool, sunriseCakes, { query: Baker.find().orderBy(Baker.col("name").asc()) });
  assert.deepEqual(bakers.map((list) => list.map((baker) => baker.name)), [["Ada", "Bo"], ["Bo"]]);
}
