// Example 9 of guide/08-transactions.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Customer } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await assert.rejects(
    pool.transaction(async (tx) => {
      await Customer.insert({ name: "Mo", email: "mo@example.com" }).execute(tx);
      await tx.execute("SELECT pg_sleep(30)", [], { signal: AbortSignal.timeout(100) });
    }),
    (error) => error instanceof DOMException && error.name === "TimeoutError",
  );

  // Mo's insert was rolled back with the transaction.
  assert.equal(await Customer.find().count(pool), 2);
}
