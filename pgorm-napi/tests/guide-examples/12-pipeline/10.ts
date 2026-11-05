// Example 10 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = (column: string) => pl.col("cake", column);
  const other = (column: string) => pl.col("other", column);

  const pairs = pl.from(Cake.table)
    .join(pl.source(Cake.table).named("other"), cake("bakery_id").eq(other("bakery_id")))
    .filter(cake("id").lt(other("id")))
    .sort(cake("id"), other("id"))
    .select(cake("name").as(pl.alias("earlier")), other("name").as(pl.alias("later")));

  assert.deepEqual(await pairs.all(pool), [
    { earlier: "Chocolate Fudge", later: "Lemon Drizzle" },
    { earlier: "Chocolate Fudge", later: "Chocolate Torte" },
    { earlier: "Lemon Drizzle", later: "Chocolate Torte" },
    { earlier: "Carrot Cake", later: "Vanilla Sponge" },
  ]);
}
