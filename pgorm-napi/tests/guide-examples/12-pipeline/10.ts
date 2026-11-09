// Example 10 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake, flavor } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const chocolate = pl.from(Cake.table)
    .filter(pl.col("cake", "flavor").comparesAs(flavor).eq("chocolate"))
    .sort(pl.col("cake", "id"))
    .select(pl.col("cake", "name"));

  assert.equal(
    chocolate.inspect().sql,
    "WITH table_0 AS (SELECT name, id FROM cake WHERE CAST(flavor AS public.flavor) = " +
      "CAST($1::text AS public.flavor)) SELECT name FROM table_0 ORDER BY id",
  );
  assert.deepEqual(await chocolate.all(pool), [{ name: "Chocolate Fudge" }, { name: "Chocolate Torte" }]);
}
