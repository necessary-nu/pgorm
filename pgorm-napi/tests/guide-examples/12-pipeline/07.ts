// Example 7 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const refused = pl.from(Cake.table)
    .group(pl.col("cake", "bakery_id"))
    .aggregate(pl.countRows().as(pl.alias("sum")));

  assert.throws(() => refused.inspect(), ConstructionError);
  await assert.rejects(refused.all(pool), /alias `sum` collides with a PRQL built-in name/);
}
