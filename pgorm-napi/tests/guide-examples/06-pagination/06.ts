// Example 6 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // By bakery, then by id within each bakery.
  const byBakery = Cake.find().cursor("bakeryId").addSecondaryOrderBy(Cake, "id");

  const first = await byBakery.first(2).all(pool);
  assert.deepEqual(first.map((cake) => cake.id), [1, 2]);

  // Bakery 1 has a third cake, which `after(1)` would skip.
  const rest = await byBakery.afterWith(1, 2).first(2).all(pool);
  assert.deepEqual(rest.map((cake) => cake.id), [5, 3]);
  const skipped = await byBakery.after(1).first(2).all(pool);
  assert.deepEqual(skipped.map((cake) => cake.id), [3, 4]);
}
