// Example 2 of guide/08-transactions.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { LifecycleError } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using db = await pool.acquire();
  await db.transaction(async (tx) => {
    // The connection is the transaction's for now: use `tx`.
    await assert.rejects(Cake.find().all(db), LifecycleError);
    assert.equal(await Cake.find().count(tx), 5);
  });
}
