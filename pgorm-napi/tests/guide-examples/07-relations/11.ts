// Example 11 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery, bakeryCakes } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // Every bakery, with its carrot cakes if it has any.
  const carrot = await Bakery.graph()
    .joinMaybeFiltered(bakeryCakes, (_bakery, cake) => cake.col("flavor").eq("carrot"))
    .allGrouped(pool);
  assert.deepEqual(carrot.map(([bakery, cakes]) => [bakery.name, cakes.length]), [["Sunrise", 0], ["Moonlight", 1]]);
}
