// Example 7 of guide/18-listen-notify.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using listener = await pool.listener();
  assert.equal("query" in listener, false);
  assert.equal("execute" in listener, false);
  assert.equal("notify" in listener, false);
}
