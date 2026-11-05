// Example 2 of guide/01-getting-started.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using db = await pool.acquire();
  await db.execute("SET application_name = 'bakery-web'");
  const row = await db.one("SELECT current_setting('application_name') AS name");
  assert.equal(row.name, "bakery-web");
}
