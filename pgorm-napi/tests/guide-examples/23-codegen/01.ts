// Example 1 of guide/23-codegen.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, DecodeError, model } from "@necessary/pgorm";
import { OrderLine } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Mistaken = model("order_line", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      orderId: column("i32", { name: "order_id" }),
      cakeId: column("i32", { name: "cake_id" }),
      quantity: column("i64"),
      price: column("decimal"),
    },
  });
  await assert.rejects(Mistaken.find().all(pool), DecodeError);

  // The bakery's own declaration matches the table.
  const lines = await OrderLine.find().all(pool);
  assert.equal(lines.length, 3);
}
