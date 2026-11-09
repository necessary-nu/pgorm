// Example 10 of guide/08-transactions.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const [noon, dusk] = await Promise.all([
    pool.transaction((tx) => Bakery.insert({ name: "Noon", city: "Tromsø" }).returning().one(tx)),
    pool.transaction((tx) => Bakery.insert({ name: "Dusk", city: "Bodø" }).returning().one(tx)),
  ]);

  assert.deepEqual([noon.name, dusk.name], ["Noon", "Dusk"]);
  assert.deepEqual([noon.id, dusk.id].sort(), [3, 4]);
}
