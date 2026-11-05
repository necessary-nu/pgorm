// Example 7 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal, LifecycleError, pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const price = pl.col("cake", "price");
  let leaked: pl.PipelineExpr | undefined;
  pl.from(Cake.table).filterWith((binder) => {
    leaked = binder.bind(new Decimal("3.50"));
    return price.lte(leaked);
  });

  assert.throws(() => pl.from(Cake.table).filter(price.gt(leaked!)), LifecycleError);
}
