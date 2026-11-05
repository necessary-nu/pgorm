// Example 5 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Order, orderBakery, orderCustomer } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const orders = await Order.graph()
    .joinOne(orderCustomer)
    .joinOne(orderBakery)
    .orderBy(Order.col("placedAt").asc())
    .all(pool);

  const summary = orders.map(([order, customer, bakery]) => [order.id, customer.name, bakery.name]);
  assert.deepEqual(summary, [[1, "Kim", "Sunrise"], [2, "Lee", "Moonlight"]]);
}
