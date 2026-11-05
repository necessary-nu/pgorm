// Example 14 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake, OrderLine } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const line = (column: string) => pl.col("order_line", column);
  const cake = (column: string) => pl.col("cake", column);
  const sold = pl.alias("sold");

  const units = pl.from(OrderLine.table)
    .group(line("cake_id"))
    .aggregate(pl.sum(line("quantity")).as(sold))
    .filterWith((binder) => sold.gte(binder.bind(1n)));

  const report = pl.from(Cake.table)
    .filterWith((binder) => cake("bakery_id").eq(binder.bind(1)))
    .join(units, cake("id").eq(pl.alias("cake_id")))
    .sort(sold.desc())
    .select(cake("name"), sold);

  assert.deepEqual(report.inspect().values.map((value) => value.value), [1n, 1n]);
  assert.deepEqual(await report.all(pool), [
    { name: "Chocolate Fudge", sold: 2n },
    { name: "Lemon Drizzle", sold: 1n },
  ]);
}
