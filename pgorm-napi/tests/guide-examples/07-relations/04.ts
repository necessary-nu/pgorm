// Example 4 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery, bakeryCakes, cakeBakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const bakeries = await Bakery.find().orderBy(Bakery.col("id").asc()).all(pool);

  // Each bakery's cakes.
  const cakes = await bakeryCakes.load(pool, bakeries);
  assert.deepEqual(
    cakes.map((list) => list.map((cake) => cake.name).sort()),
    [["Chocolate Fudge", "Chocolate Torte", "Lemon Drizzle"], ["Carrot Cake", "Vanilla Sponge"]],
  );

  // Each of Sunrise's cakes, with its bakery.
  const theirBakery = await cakeBakery.load(pool, cakes[0]!);
  assert.ok(theirBakery.every((bakery) => bakery?.name === "Sunrise"));
}
