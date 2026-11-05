// Example 7 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const names = await Cake.select("name").orderBy(Cake.col("id").asc()).all(pool);
  assert.deepEqual(names[0], { name: "Chocolate Fudge" });

  const priced = await Cake.select("name", "price")
    .where(Cake.col("glutenFree").eq(true))
    .orderBy(Cake.col("price").asc())
    .all(pool);
  assert.equal(priced[0]!.name, "Vanilla Sponge");
  assert.equal(priced[0]!.price.toString(), "2.90");
}
