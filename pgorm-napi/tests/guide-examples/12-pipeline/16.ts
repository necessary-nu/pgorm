// Example 16 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  /** The cakes a customer who avoids gluten can eat. */
  function glutenFree(cakes: pl.Pipeline): pl.Pipeline {
    return cakes.filter(pl.col("cake", "gluten_free").eq(pl.literal(true)));
  }

  assert.equal(
    glutenFree(pl.from(Cake.table)).select(pl.col("cake", "name")).inspect().sql,
    "SELECT name FROM cake WHERE gluten_free = true",
  );
}
