// Example 7 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, fromStatement } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const pages = fromStatement("SELECT name, price FROM cake WHERE bakery_id = $1 ORDER BY price", [1]).paginate(2);
  assert.deepEqual(await pages.numItemsAndPages(pool), { items: 3, pages: 2 });
  const first = await pages.fetchPage(pool, 0);
  assert.deepEqual(first.map((row) => [row.name, String(row.price)]), [["Lemon Drizzle", "3.20"], ["Chocolate Fudge", "4.50"]]);

  const cakes = Cake.findByStatement("SELECT * FROM cake WHERE gluten_free ORDER BY id").paginate(2);
  assert.deepEqual((await cakes.fetchPage(pool, 1)).map((cake) => cake.name), ["Chocolate Torte"]);
  assert.equal(await Cake.findByStatement("SELECT * FROM cake").count(pool), 5);

  assert.throws(() => fromStatement("DELETE FROM cake").paginate(2), ConstructionError);
}
