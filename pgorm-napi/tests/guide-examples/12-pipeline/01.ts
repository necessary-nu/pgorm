// Example 1 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const { sql, values } = pl.from(Cake)
    .filter(pl.col(Cake, "glutenFree").eq(pl.literal(true)))
    .sort(pl.col(Cake, "price").desc())
    .take(2)
    .select(pl.col(Cake, "name"), pl.col(Cake, "price"))
    .inspect();

  assert.equal(
    sql,
    "SELECT name, price FROM cake WHERE gluten_free = true ORDER BY price DESC LIMIT 2",
  );
  assert.deepEqual(values, []);
}
