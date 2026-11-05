// Example 12 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery, bakeryCakes } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // Sunrise has three cakes, and the limit cut its list short.
  const cut = await Bakery.graph().joinMaybe(bakeryCakes).limit(2).allGrouped(pool);
  assert.deepEqual(cut.map(([bakery, cakes]) => [bakery.name, cakes.length]), [["Sunrise", 2]]);
}
