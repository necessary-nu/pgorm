// Example 7 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const page = (number: number) =>
    pool.query("SELECT name, price FROM cake WHERE bakery_id = $1 ORDER BY price LIMIT $2 OFFSET $3", [1, 2, number * 2]);

  const first = await page(0);
  assert.equal(first[0]!.name, "Lemon Drizzle");
  assert.equal(String(first[0]!.price), "3.20");
  assert.equal((await page(1)).length, 1);
}
