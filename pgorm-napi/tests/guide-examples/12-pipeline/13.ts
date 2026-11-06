// Example 13 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Other = Cake.as("other");

  const pairs = pl.from(Cake)
    .join(Other, pl.col(Cake, "bakeryId").eq(pl.col(Other, "bakeryId")))
    .filter(pl.col(Cake, "id").lt(pl.col(Other, "id")))
    .sort(pl.col(Cake, "id"), pl.col(Other, "id"))
    .select(pl.col(Cake, "name").as(pl.alias("earlier")), pl.col(Other, "name").as(pl.alias("later")));

  assert.deepEqual(await pairs.all(pool), [
    { earlier: "Chocolate Fudge", later: "Lemon Drizzle" },
    { earlier: "Chocolate Fudge", later: "Chocolate Torte" },
    { earlier: "Lemon Drizzle", later: "Chocolate Torte" },
    { earlier: "Carrot Cake", later: "Vanilla Sponge" },
  ]);
}
