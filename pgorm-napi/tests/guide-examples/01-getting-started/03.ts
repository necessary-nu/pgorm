// Example 3 of guide/01-getting-started.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const glutenFree = await Cake.find()
    .where(Cake.col("glutenFree").eq(true))
    .orderBy(Cake.col("name").asc())
    .all(pool);

  assert.deepEqual(
    glutenFree.map((cake) => cake.name),
    ["Chocolate Torte", "Lemon Drizzle", "Vanilla Sponge"],
  );
}
