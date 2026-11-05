// Example 5 of guide/20-configuration.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  await using single = new Pool(url, { maxSize: 1 });
  await single.transaction(async (tx) => {
    await tx.execute("SET LOCAL enable_seqscan = off");
    assert.equal((await tx.one("SELECT current_setting('enable_seqscan') AS seen")).seen, "off");
  });
  assert.equal((await single.one("SELECT current_setting('enable_seqscan') AS seen")).seen, "on");
}
