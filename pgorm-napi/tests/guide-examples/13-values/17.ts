// Example 17 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute("CREATE TABLE opening (bakery_id integer PRIMARY KEY, days bit(7) NOT NULL)");

  // Monday to Saturday.
  await pool.execute("INSERT INTO opening (bakery_id, days) VALUES ($1, $2::int4::bit(7))", [1, 0b111_1110]);

  const sunrise = await pool.one("SELECT days::int8 AS days FROM opening WHERE bakery_id = $1", [1]);
  assert.equal(sunrise.days, 0b111_1110n);
}
