// Example 2 of guide/21-metrics.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool, url }: GuideContext): Promise<void> {
  /** Whether the pool has room, and a warning when it is running low. */
  function poolHealth(pool: Pool): { ok: true; warning?: string } | { ok: false; error: string } {
    const status = pool.status();
    if (status.size === status.maxSize && status.available === 0) {
      return { ok: false, error: `all ${status.maxSize} connections are in use` };
    }
    if (status.available < status.size / 4) {
      return { ok: true, warning: `pool running low: ${status.available}/${status.size} idle` };
    }
    return { ok: true };
  }

  await using small = new Pool(url, { maxSize: 2 });
  assert.deepEqual(poolHealth(small), { ok: true });

  const busy = [await small.acquire(), await small.acquire()];
  assert.deepEqual(poolHealth(small), { ok: false, error: "all 2 connections are in use" });
  for (const db of busy) await db.close();
  assert.deepEqual(poolHealth(small), { ok: true });
}
