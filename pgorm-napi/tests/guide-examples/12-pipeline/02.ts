// Example 2 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = (column: string) => pl.col("cake", column);

  const dearest = await pl.from(Cake.table)
    .filter(cake("gluten_free").eq(pl.literal(true)))
    .sort(cake("price").desc())
    .take(2)
    .select(cake("name"), cake("price"))
    .all(pool);

  assert.deepEqual(
    dearest.map((row) => [row.name, String(row.price)]),
    [["Chocolate Torte", "6.00"], ["Lemon Drizzle", "3.20"]],
  );
}
