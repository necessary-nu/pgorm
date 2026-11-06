// Example 12 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, pipeline as pl } from "@necessary/pgorm";
import { Bakery, Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // A third bakery, which sells nothing yet.
  await Bakery.insert({ name: "Noon", city: "Tromsø" }).execute(pool);

  const menus = pl.from(Cake)
    .join(Bakery, pl.col(Cake, "bakeryId").eq(pl.col(Bakery, "id")), { kind: "right" })
    .filter(pl.col(Bakery, "city").ne("Oslo"))
    .sort(pl.col(Bakery, "id"), pl.col(Cake, "id"))
    .selectSources(Cake, Bakery);
  const rows = await menus.all(pool);
  assert.deepEqual(rows.map(([cake, bakery]) => [cake?.name ?? null, bakery?.name ?? null]), [
    ["Carrot Cake", "Moonlight"],
    ["Vanilla Sponge", "Moonlight"],
    [null, "Noon"],
  ]);

  const reshaped = pl.from(Cake).select(pl.col(Cake, "name")).selectSources(Cake);
  assert.throws(() => reshaped.inspect(), ConstructionError);
}
