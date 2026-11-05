// Example 1 of guide/02-promises.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const [cakes, bakers] = await Promise.all([
    pool.one("SELECT count(*) AS n FROM cake"),
    pool.one("SELECT count(*) AS n FROM baker"),
  ]);
  assert.deepEqual([cakes.n, bakers.n], [5n, 3n]);
}
