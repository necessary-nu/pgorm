// Example 11 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Bakery, Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cakeName = pl.alias("cake");

  const inBergen = pl.from(Cake)
    .join(Bakery, pl.col(Cake, "bakeryId").eq(pl.col(Bakery, "id")), { kind: "inner" })
    .filter(pl.col(Bakery, "city").eq(pl.literal("Bergen")))
    .select(pl.col(Cake, "name").as(cakeName), pl.col(Bakery, "name"))
    .sort(cakeName);

  assert.equal(
    inBergen.inspect().sql,
    "SELECT cake.name AS cake, bakery.name FROM cake INNER JOIN bakery " +
      "ON cake.bakery_id = bakery.id WHERE bakery.city = 'Bergen' ORDER BY cake",
  );

  assert.deepEqual(await inBergen.all(pool), [
    { cake: "Carrot Cake", name: "Moonlight" },
    { cake: "Vanilla Sponge", name: "Moonlight" },
  ]);
}
