// Example 14 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { CakeBaker } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const bakes = (cakeId: number, bakerId: number) => CakeBaker.insert({ cakeId, bakerId }).onConflictDoNothing();

  // Ada already bakes the Chocolate Fudge.
  const again = bakes(1, 1);
  assert.equal(
    again.inspect().sql,
    `INSERT INTO "cake_baker" ("cake_id", "baker_id") VALUES ($1, $2) ON CONFLICT ("cake_id", "baker_id") DO NOTHING`,
  );
  assert.deepEqual(await again.execute(pool), { kind: "conflicted" });

  // Bo starts baking the Chocolate Fudge too.
  assert.deepEqual(await bakes(1, 2).execute(pool), { kind: "inserted", value: 1 });

  assert.deepEqual(await CakeBaker.insertMany([]).onEmptyDoNothing().execute(pool), { kind: "empty" });
}
