// Example 15 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = (column: string) => pl.col("cake", column);
  const running = pl.alias("running");

  const rows = await pl.from(Cake.table)
    .filter(cake("bakery_id").eq(pl.literal(1)))
    .window(pl.over().by(cake("bakery_id")).sortBy(cake("price")).rows(null, 0), pl.sum(cake("price")).as(running))
    .sort(cake("price"))
    .select(cake("name"), running)
    .all(pool);

  assert.deepEqual(
    rows.map((row) => [row.name, String(row.running)]),
    [["Lemon Drizzle", "3.20"], ["Chocolate Fudge", "7.70"], ["Chocolate Torte", "13.70"]],
  );
}
