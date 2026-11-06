// Example 4 of guide/20-configuration.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  // What the next caller sees of a setting the last one changed.
  for (const [recycle, seen] of [["verified", "off"], ["fast", "off"], ["clean", "on"]] as const) {
    await using single = new Pool(url, { maxSize: 1, recycle });

    let db = await single.acquire();
    await db.execute("SET enable_seqscan = off");
    await db.close();

    // The pool has one connection, so this is the same session.
    db = await single.acquire();
    const row = await db.one("SELECT current_setting('enable_seqscan') AS seen");
    assert.equal(row.seen, seen, recycle);
    await db.close();
  }
}
