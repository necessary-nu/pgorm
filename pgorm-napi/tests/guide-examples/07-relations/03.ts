// Example 3 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { CakeBaker, Customer, cakeCakeBakers, customerOrders, lineCake, orderLines } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // From a customer, through what they ordered, to who bakes it.
  const bakedForCy = Customer.find()
    .join(customerOrders)
    .join(orderLines)
    .join(lineCake)
    .join(cakeCakeBakers)
    .where(CakeBaker.col("bakerId").eq(3));
  assert.equal(
    bakedForCy.inspect().sql,
    `SELECT "customer"."id", "customer"."name", "customer"."email", "customer"."notes" FROM "customer" INNER JOIN "order" ON "customer"."id" = "order"."customer_id" INNER JOIN "order_line" ON "order"."id" = "order_line"."order_id" INNER JOIN "cake" ON "order_line"."cake_id" = "cake"."id" INNER JOIN "cake_baker" ON "cake"."id" = "cake_baker"."cake_id" WHERE "cake_baker"."baker_id" = $1`,
  );
  assert.deepEqual((await bakedForCy.all(pool)).map((customer) => customer.name), ["Lee"]);
}
