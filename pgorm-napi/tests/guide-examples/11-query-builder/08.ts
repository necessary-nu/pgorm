// Example 8 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, ConstructionError, Decimal, deleteFrom, Table, update } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = new Table("cake");
  const bakery = new Table("bakery");

  // Every cake sold in Bergen goes up by fifty cents.
  const raise = update(cake)
    .set("price", cake.col("price").add(new Decimal("0.50")))
    .from(bakery)
    .where(cake.col("bakery_id").eq(bakery.col("id")))
    .where(bakery.col("city").eq("Bergen"));
  assert.equal(
    raise.inspect().sql,
    `UPDATE "cake" SET "price" = "cake"."price" + $1 FROM "bakery" WHERE "cake"."bakery_id" = "bakery"."id" AND "bakery"."city" = $2`,
  );
  assert.equal(await pool.execute(raise), 2);

  // Lee's order lines go.
  const orderLine = new Table("order_line");
  const order = new Table("order");
  const forget = deleteFrom(orderLine)
    .using(order)
    .where(orderLine.col("order_id").eq(order.col("id")))
    .where(order.col("customer_id").eq(2));
  assert.equal(
    forget.inspect().sql,
    `DELETE FROM "order_line" USING "order" WHERE "order_line"."order_id" = "order"."id" AND "order"."customer_id" = $1`,
  );
  assert.equal(await pool.execute(forget), 1);

  // A delete that says nothing of which rows is refused.
  assert.throws(() => deleteFrom(orderLine).inspect(), ConstructionError);
}
