// Example 3 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const names = (cakes: { name: string }[]) => cakes.map((cake) => cake.name);

  const feed = Cake.find().cursor("price", "id");
  const first = await feed.first(2).all(pool);
  assert.deepEqual(names(first), ["Vanilla Sponge", "Lemon Drizzle"]);

  // The next window starts after the last row read.
  const lastSeen = first.at(-1)!;
  const next = await feed.after(lastSeen.price, lastSeen.id).first(2).all(pool);
  assert.deepEqual(names(next), ["Carrot Cake", "Chocolate Fudge"]);

  // The two dearest, from the far end, still in ascending order.
  const dearest = await Cake.find().cursor("price", "id").last(2).all(pool);
  assert.deepEqual(names(dearest), ["Chocolate Fudge", "Chocolate Torte"]);
}
