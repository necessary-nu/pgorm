// Example 6 of guide/08-transactions.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { LifecycleError } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.transaction(async (tx) => {
    await tx.transaction(async (savepoint) => {
      assert.equal(savepoint.parent, tx);
      await assert.rejects(tx.query("SELECT 1"), LifecycleError);
      assert.equal(await Cake.find().count(savepoint), 5);
    });
  });
}
