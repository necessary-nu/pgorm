// Example 8 of guide/09-errors.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError, Pool, TimeoutError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  const elsewhere = new URL(url);
  elsewhere.pathname = "/no_such_bakery";
  await using missing = new Pool(elsewhere.toString());
  await assert.rejects(missing.one("SELECT 1"), (error) =>
    error instanceof DatabaseError && error.sqlstate === "3D000");

  // One connection, held: the next operation waits 200 ms for it, then gives up.
  await using small = new Pool(url, { maxSize: 1, acquireTimeout: 200 });
  await using held = await small.acquire();
  await assert.rejects(small.one("SELECT 1"), TimeoutError);
}
