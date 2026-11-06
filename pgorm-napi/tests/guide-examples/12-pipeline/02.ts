// Example 2 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const lemon = pl.from(Cake).filter(pl.col(Cake, "flavor").eq("lemon")).select(pl.col(Cake, "name"));
  assert.equal(lemon.inspect().sql, "SELECT name FROM cake WHERE flavor = CAST($1::text AS public.flavor)");
  assert.deepEqual((await lemon.all(pool)).map((row) => row.name), ["Lemon Drizzle"]);

  // @ts-expect-error: "banana" is not one of the flavor's labels
  assert.throws(() => pl.col(Cake, "flavor").eq("banana"), ConstructionError);
}
