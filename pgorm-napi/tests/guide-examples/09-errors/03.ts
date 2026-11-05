// Example 3 of guide/09-errors.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const missing = await pool.query("SELECT * FROM pastry").catch((error: unknown) => error);
  assert.ok(missing instanceof DatabaseError);
  // 42P01 is undefined_table.
  assert.equal(missing.sqlstate, "42P01");
  assert.equal(missing.severity, "ERROR");
}
