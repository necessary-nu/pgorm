// Example 1 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const raised = await pool.execute(
    "UPDATE cake SET price = price + $1 WHERE bakery_id = $2",
    [new Decimal("0.50"), 2],
  );
  assert.equal(raised, 2);

  const row = await pool.one("SELECT name, price FROM cake WHERE id = $1", [3]);
  assert.equal(row.name, "Carrot Cake");
  assert.equal(String(row.price), "4.30");

  const none = await pool.optional("SELECT name FROM cake WHERE id = $1", [99]);
  assert.equal(none, null);
}
