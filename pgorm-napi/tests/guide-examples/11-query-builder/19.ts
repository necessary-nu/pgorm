// Example 19 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { literal, select } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const order = literal({ customer: "kim@example.com", lines: [{ cake: 2, qty: 3 }] }).cast("jsonb");
  const query = select(
    order.castJsonField("customer").as("customer"),
    order.getJsonField("lines").getJsonField(literal(0)).as("first"),
    order.castJsonPath(["lines", "0", "qty"]).as("qty"),
    order.hasJsonKey("lines").as("lined"),
    order.hasAllJsonKeys(["customer", "notes"]).as("noted"),
  );

  const row = await pool.one(query);
  assert.deepEqual(
    [row.customer, row.first, row.qty, row.lined, row.noted],
    ["kim@example.com", { cake: 2, qty: 3 }, "3", true, false],
  );
}
