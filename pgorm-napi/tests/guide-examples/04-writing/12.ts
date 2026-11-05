// Example 12 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Conflict } from "@necessary/pgorm";
import { CakeBaker } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // Ada already bakes the Chocolate Fudge.
  const again = CakeBaker.insert({ cakeId: 1, bakerId: 1 }).onConflict(Conflict.on("cake_id", "baker_id").doNothing());
  assert.equal(
    again.inspect().sql,
    `INSERT INTO "cake_baker" ("cake_id", "baker_id") VALUES ($1, $2) ON CONFLICT ("cake_id", "baker_id") DO NOTHING`,
  );
  assert.equal(await again.execute(pool), 0);
  assert.equal(await again.returningUpsert(pool), null);

  // Bo starts baking the Chocolate Fudge too.
  const bo = CakeBaker.insert({ cakeId: 1, bakerId: 2 }).onConflict(Conflict.doNothing());
  assert.equal(await bo.execute(pool), 1);

  assert.deepEqual(await CakeBaker.insertMany([]).returningUpserts(pool), []);
}
