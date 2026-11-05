// Example 1 of guide/08-transactions.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import { Order, OrderLine } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const placedAt = Temporal.Instant.from("2026-11-04T10:00:00Z");
  const order = await pool.transaction(async (tx) => {
    const order = await Order.insert({ customerId: 2, bakeryId: 1, placedAt }).returning().one(tx);
    await OrderLine.insertMany([
      { orderId: order.id, cakeId: 5, quantity: 1, price: new Decimal("6.00") },
      { orderId: order.id, cakeId: 2, quantity: 2, price: new Decimal("3.20") },
    ]).execute(tx);
    return order;
  });

  const lines = await OrderLine.find().where(OrderLine.col("orderId").eq(order.id)).count(pool);
  assert.deepEqual([order.id, lines], [3, 2]);
}
