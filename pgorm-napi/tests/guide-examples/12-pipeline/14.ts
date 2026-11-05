// Example 14 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = (column: string) => pl.col("cake", column);
  const place = pl.alias("place");

  const dearest = pl.from(Cake.table)
    .window(pl.over().by(cake("bakery_id")).sortBy(cake("price").desc()), pl.rowNumber().as(place))
    .filter(place.eq(pl.literal(1)))
    .sort(cake("bakery_id"))
    .select(cake("bakery_id"), cake("name"));

  assert.equal(
    dearest.inspect().sql,
    "WITH table_0 AS (SELECT bakery_id, name, ROW_NUMBER() OVER (PARTITION BY bakery_id " +
      "ORDER BY price DESC) AS _expr_0 FROM cake) SELECT bakery_id, name FROM table_0 " +
      "WHERE _expr_0 = 1 ORDER BY bakery_id",
  );

  assert.deepEqual(await dearest.all(pool), [
    { bakery_id: 1, name: "Chocolate Torte" },
    { bakery_id: 2, name: "Carrot Cake" },
  ]);
}
