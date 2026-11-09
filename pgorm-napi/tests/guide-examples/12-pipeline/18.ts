// Example 18 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Bakery, Customer } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const everyone = pl.from(Bakery.table)
    .select(pl.col("bakery", "name"))
    .append(pl.from(Customer.table).select(pl.col("customer", "name")))
    .distinct();

  const names = (await everyone.all(pool)).map((row) => row.name).sort();
  assert.deepEqual(names, ["Kim", "Lee", "Moonlight", "Sunrise"]);
}
