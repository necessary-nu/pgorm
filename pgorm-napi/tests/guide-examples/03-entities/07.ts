// Example 7 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, column, DataType, Decimal, fromModel, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const money = new DataType("numeric", { precision: 10, scale: 2 });
  const OrderLine = model("order_line", {
    renameAll: "snake_case",
    columns: {
      id: column("i32", { primaryKey: true, autoIncrement: true }),
      orderId: column("i32"),
      cakeId: column("i32"),
      quantity: column("i32", { defaultValue: 1 }),
      price: column("decimal", { columnType: money }),
      total: column("decimal", { columnType: money, generatedStored: col("price").mul(col("quantity")) }),
    },
    uniqueKeys: [{ name: "one_line_per_cake", fields: ["orderId", "cakeId"] }],
  });

  const create = fromModel(OrderLine).table;
  assert.equal(
    create.inspect().sql,
    `CREATE TABLE "order_line" ( "id" serial NOT NULL, "order_id" integer NOT NULL, "cake_id" integer NOT NULL, "quantity" integer NOT NULL DEFAULT 1, "price" decimal(10, 2) NOT NULL, "total" decimal(10, 2) NOT NULL GENERATED ALWAYS AS ("price" * "quantity") STORED, PRIMARY KEY ("id"), CONSTRAINT "one_line_per_cake" UNIQUE ("order_id", "cake_id") )`,
  );
  await pool.execute(create);

  // The quantity takes its default, and the database computes the total.
  const line = await OrderLine.insert({ orderId: 1, cakeId: 2, price: new Decimal("3.20") })
    .returning("quantity", "total")
    .one(pool);
  assert.equal(line.quantity, 1);
  assert.equal(line.total.toString(), "3.20");
}
