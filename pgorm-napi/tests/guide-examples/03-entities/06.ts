// Example 6 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, Decimal, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute(
    `CREATE TABLE order_line ( id serial NOT NULL, order_id integer NOT NULL, cake_id integer NOT NULL, quantity integer NOT NULL DEFAULT 1, price decimal(10, 2) NOT NULL, total decimal(10, 2) NOT NULL GENERATED ALWAYS AS (price * quantity) STORED, PRIMARY KEY (id), CONSTRAINT one_line_per_cake UNIQUE (order_id, cake_id) )`,
  );

  const OrderLine = model("order_line", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      orderId: column("i32", { name: "order_id" }),
      cakeId: column("i32", { name: "cake_id" }),
      quantity: column("i32", { default: true }),
      price: column("decimal"),
      total: column("decimal", { generated: "always" }),
    },
    uniqueKeys: [{ name: "one_line_per_cake", fields: ["orderId", "cakeId"] }],
  });

  // The quantity takes its default, and the database computes the total.
  const line = await OrderLine.insert({ orderId: 1, cakeId: 2, price: new Decimal("3.20") })
    .returning("quantity", "total")
    .one(pool);
  assert.equal(line.quantity, 1);
  assert.equal(line.total.toString(), "3.20");
}
