// Example 3 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = (column: string) => pl.col("cake", column);
  const cakes = pl.alias("cakes");
  const mean = pl.alias("mean_price");

  const busy = pl.from(Cake.table)
    .group(cake("bakery_id"))
    .aggregate(pl.countRows().as(cakes), pl.average(cake("price")).as(mean))
    .filter(cakes.gt(pl.literal(2)));

  assert.equal(
    busy.inspect().sql,
    "SELECT bakery_id, COUNT(*) AS cakes, AVG(price) AS mean_price FROM cake " +
      "GROUP BY bakery_id HAVING COUNT(*) > 2",
  );

  const rows = await busy.all(pool);
  assert.equal(rows.length, 1);
  assert.deepEqual([rows[0]!.bakery_id, rows[0]!.cakes], [1, 3n]);
  assert.ok(String(rows[0]!.mean_price).startsWith("4.5666"));
}
