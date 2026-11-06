// Example 13 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery, bakeryCakes } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const first = await Bakery.find().orderBy(Bakery.col("id").asc()).limit(1).all(pool);
  const cakes = await bakeryCakes.load(pool, first);
  assert.deepEqual([first.length, cakes[0]!.length], [1, 3]);
}
