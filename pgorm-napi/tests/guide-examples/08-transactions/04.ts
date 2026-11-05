// Example 4 of guide/08-transactions.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import { Order, OrderLine } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const placedAt = Temporal.Instant.from("2026-11-04T10:00:00Z");
  const { order, skipped } = await pool.transaction(async (tx) => {
    const order = await Order.insert({ customerId: 1, bakeryId: 2, placedAt }).returning().one(tx);
    const skipped: number[] = [];
    for (const cakeId of [3, 99, 4]) {
      try {
        await tx.transaction((savepoint) =>
          OrderLine.insert({ orderId: order.id, cakeId, quantity: 1, price: new Decimal("3.00") }).execute(savepoint)
        );
      } catch {
        skipped.push(cakeId);
      }
    }
    return { order, skipped };
  });

  const lines = await OrderLine.find().where(OrderLine.col("orderId").eq(order.id)).count(pool);
  assert.deepEqual([lines, skipped], [2, [99]]);
}
