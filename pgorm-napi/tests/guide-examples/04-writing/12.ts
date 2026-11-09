// Example 12 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Baker, CakeBaker } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // Bo hands every cake on to the others.
  const handed = CakeBaker.delete()
    .using(Baker)
    .where(CakeBaker.col("bakerId").eq(Baker.col("id")))
    .where(Baker.col("name").eq("Bo"));
  assert.equal(
    handed.inspect().sql,
    `DELETE FROM "cake_baker" USING "baker" WHERE "cake_baker"."baker_id" = "baker"."id" AND "baker"."name" = $1`,
  );
  assert.equal(await handed.execute(pool), 2);
}
