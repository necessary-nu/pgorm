// Example 6 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = (column: string) => pl.col("cake", column);
  const tier = pl.alias("tier");
  const flavorName = pl.alias("flavor_name");

  const tiers = pl.from(Cake.table)
    .derive(
      pl.caseWhen([[cake("price").gte(pl.literal(4)), pl.literal("premium")]], pl.literal("standard")).as(tier),
      cake("flavor").cast("text").as(flavorName),
    )
    .sort(cake("id"))
    .takeRange(2, 3)
    .select(cake("name"), tier, flavorName);

  assert.equal(
    tiers.inspect().sql,
    "WITH table_0 AS (SELECT name, CASE WHEN price >= 4 THEN 'premium' ELSE 'standard' END " +
      "AS tier, CAST(flavor AS text) AS flavor_name, id FROM cake ORDER BY id LIMIT 2 OFFSET 1) " +
      "SELECT name, tier, flavor_name FROM table_0 ORDER BY id",
  );

  assert.deepEqual(await tiers.all(pool), [
    { name: "Lemon Drizzle", tier: "standard", flavor_name: "lemon" },
    { name: "Carrot Cake", tier: "standard", flavor_name: "carrot" },
  ]);
}
