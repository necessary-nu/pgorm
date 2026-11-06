// Example 5 of guide/08-transactions.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError, ForeignKeyViolationError } from "@necessary/pgorm";
import { Order } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const placedAt = Temporal.Instant.from("2026-11-04T10:00:00Z");
  await assert.rejects(
    pool.transaction(async (tx) => {
      await Order.insert({ customerId: 1, bakeryId: 1, placedAt }).execute(tx);
      try {
        await Order.insert({ customerId: 99, bakeryId: 1, placedAt }).execute(tx);
      } catch (error) {
        // A mistake: the callback carries on.
        if (!(error instanceof ForeignKeyViolationError)) throw error;
      }
    }),
    (error) =>
      error instanceof DatabaseError && error.sqlstate === "25P02" &&
      error.message.startsWith("the transaction was rolled back, not committed"),
  );

  // Neither order was written.
  assert.equal(await Order.find().count(pool), 2);
}
