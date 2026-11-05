// Example 2 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using db = await pool.acquire();
  await db.query("SELECT name FROM bakery");
  await db.query("SELECT name FROM bakery");

  const prepared = await db.one(
    "SELECT count(*) AS n FROM pg_prepared_statements WHERE starts_with(statement, $1)",
    ["SELECT name FROM bakery"],
  );
  assert.equal(prepared.n, 1n);
}
