// Example 11 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { copyFrom, DatabaseError, LifecycleError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using db = await pool.acquire();
  await assert.rejects(db.copyIn(copyFrom("pastry", "text"), new Uint8Array()), DatabaseError);

  // The connection is closed now.
  await assert.rejects(db.one("SELECT 1 AS one"), LifecycleError);

  // The pool does not lend it again.
  const { one } = await pool.one("SELECT 1 AS one");
  assert.equal(one, 1);
}
