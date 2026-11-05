// Example 6 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // By bakery, then by id within each bakery.
  const byBakery = Cake.find().cursor("bakeryId", "id");

  const first = await byBakery.first(2).all(pool);
  assert.deepEqual(first.map((cake) => cake.id), [1, 2]);

  // Bakery 1 has a third cake, which comes next.
  const rest = await byBakery.after(1, 2).first(2).all(pool);
  assert.deepEqual(rest.map((cake) => cake.id), [5, 3]);
}
