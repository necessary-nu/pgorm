// Example 19 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = (column: string) => pl.col("cake", column);
  const sunrise = pl.from(Cake.table).filter(cake("bakery_id").eq(pl.literal(1)));

  const notGlutenFree = sunrise
    .remove(pl.from(Cake.table).filter(cake("gluten_free").eq(pl.literal(true))))
    .select(cake("name"));
  assert.equal(
    notGlutenFree.inspect().sql,
    "WITH table_1 AS (SELECT * FROM cake WHERE bakery_id = 1 " +
      "EXCEPT ALL SELECT * FROM cake WHERE gluten_free = true) SELECT name FROM table_1",
  );
  assert.deepEqual(await notGlutenFree.all(pool), [{ name: "Chocolate Fudge" }]);

  const affordable = sunrise
    .intersect(pl.from(Cake.table).filter(cake("price").lt(pl.literal(5))))
    .sort(cake("name"))
    .select(cake("name"));
  assert.deepEqual(await affordable.all(pool), [
    { name: "Chocolate Fudge" },
    { name: "Lemon Drizzle" },
  ]);
}
