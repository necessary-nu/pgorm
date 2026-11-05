// Example 11 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake, cakeBakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // Every cake with its bakery, two rows at a time, cheapest first.
  const cursor = Cake.graph().joinOne(cakeBakery).cursor("price");
  const page = await cursor.first(2).all(pool);
  assert.deepEqual(
    page.map(([cake, bakery]) => [cake.name, bakery.name]),
    [["Vanilla Sponge", "Moonlight"], ["Lemon Drizzle", "Sunrise"]],
  );

  const lastPrice = page[1]![0].price;
  const next = await cursor.after(lastPrice).first(2).all(pool);
  assert.deepEqual(next.map(([cake]) => cake.name), ["Carrot Cake", "Chocolate Fudge"]);
}
