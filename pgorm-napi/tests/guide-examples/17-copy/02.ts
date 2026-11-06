// Example 2 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await assert.rejects(pool.query("COPY cake TO STDOUT"), ConstructionError);
}
