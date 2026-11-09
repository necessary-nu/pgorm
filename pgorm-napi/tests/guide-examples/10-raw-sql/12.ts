// Example 12 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal, prql } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const under = new Decimal("4.00");
  const { sql, values } = prql("from cake | filter price < $1 | sort name | select {name, price}", [under]);
  assert.equal(sql, "SELECT name, price FROM cake WHERE price < $1 ORDER BY name");
  assert.deepEqual(values, [under]);

  const rows = await pool.query(sql, values);
  assert.deepEqual(rows.map((row) => row.name), ["Carrot Cake", "Lemon Drizzle", "Vanilla Sponge"]);
}
