// Example 2 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const sizes: number[] = [];
  for await (const page of Cake.find().orderBy(Cake.col("id").asc()).paginate(2).pages(pool)) {
    sizes.push(page.length);
  }
  assert.deepEqual(sizes, [2, 2, 1]);
}
