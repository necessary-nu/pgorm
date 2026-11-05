// Example 1 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const pages = Cake.find().orderBy(Cake.col("name").asc()).paginate(2);

  assert.deepEqual(await pages.numItemsAndPages(pool), { items: 5, pages: 3 });

  const second = await pages.fetchPage(pool, 1);
  assert.deepEqual(second.map((cake) => cake.name), ["Chocolate Torte", "Lemon Drizzle"]);
  assert.deepEqual(await pages.fetchPage(pool, 3), []);

  assert.throws(() => Cake.find().paginate(0), ConstructionError);
}
