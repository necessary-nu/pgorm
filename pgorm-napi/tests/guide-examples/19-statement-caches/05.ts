// Example 5 of guide/19-statement-caches.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  assert.throws(() => new Pool(url, { statementCacheChannel: "" }), ConstructionError);
  assert.throws(() => new Pool(url, { statementCacheChannel: "c".repeat(64) }), ConstructionError);
}
