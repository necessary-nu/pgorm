// Example 6 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery, Cake, bakeryCakes } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // A third bakery, which sells nothing yet.
  await Bakery.insert({ name: "Noon", city: "Tromsø" }).execute(pool);

  const rows = await Bakery.graph()
    .joinMaybe(bakeryCakes)
    .where(Bakery.col("name").ne("Sunrise"))
    .orderBy(Bakery.col("id").asc(), Cake.col("name").asc())
    .all(pool);
  assert.deepEqual(
    rows.map(([bakery, cake]) => [bakery.name, cake?.name ?? null]),
    [["Moonlight", "Carrot Cake"], ["Moonlight", "Vanilla Sponge"], ["Noon", null]],
  );

  const grouped = await Bakery.graph().joinMaybe(bakeryCakes).orderBy(Cake.col("price").asc()).allGrouped(pool);
  assert.deepEqual(
    grouped.map(([bakery, cakes]) => [bakery.name, cakes.length]),
    [["Moonlight", 2], ["Sunrise", 3], ["Noon", 0]],
  );
}
