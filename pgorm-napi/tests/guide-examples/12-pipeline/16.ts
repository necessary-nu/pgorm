// Example 16 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const cake = (column: string) => pl.col("cake", column);

  const dropped = pl.from(Cake.table).select(cake("name")).filter(cake("price").gt(pl.literal(4)));
  assert.throws(() => dropped.inspect(), /PRQL compilation failed/);

  const quoted = pl.from(Cake.table).select(pl.col("cake", 'na"me'));
  assert.throws(() => quoted.inspect(), ConstructionError);
}
