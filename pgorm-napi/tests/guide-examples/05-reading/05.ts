// Example 5 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const twoDearest = await Cake.find()
    .orderBy(Cake.col("price").desc(), Cake.col("name").asc())
    .limit(2)
    .all(pool);
  assert.deepEqual(twoDearest.map((cake) => cake.name), ["Chocolate Torte", "Chocolate Fudge"]);

  const third = await Cake.find().orderBy(Cake.col("price").desc()).offset(2).limit(1).one(pool);
  assert.equal(third.name, "Carrot Cake");
}
