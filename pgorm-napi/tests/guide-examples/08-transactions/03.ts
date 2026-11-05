// Example 3 of guide/08-transactions.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal, ForeignKeyViolationError } from "@necessary/pgorm";
import { Order, OrderLine } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const placedAt = Temporal.Instant.from("2026-11-04T10:00:00Z");
  await assert.rejects(
    pool.transaction(async (tx) => {
      const order = await Order.insert({ customerId: 1, bakeryId: 1, placedAt }).returning().one(tx);
      await OrderLine.insert({ orderId: order.id, cakeId: 99, quantity: 1, price: new Decimal("1.00") }).execute(tx);
      return order;
    }),
    ForeignKeyViolationError,
  );

  assert.equal(await Order.find().count(pool), 2);
}
