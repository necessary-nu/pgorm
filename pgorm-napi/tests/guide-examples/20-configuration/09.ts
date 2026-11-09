// Example 9 of guide/20-configuration.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { MultiPool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  await using pools = new MultiPool(url, { web: { maxSize: 8 }, jobs: { maxSize: 2 } }, { recycle: "clean" });
  const jobs = pools.get("jobs");
  assert.ok(jobs !== null);
  assert.equal(jobs.tag, "jobs");
  assert.equal((await jobs.one("SELECT 1 AS one")).one, 1);

  assert.deepEqual(
    Object.entries(pools.status()).map(([tag, status]) => [tag, status.maxSize]),
    [["jobs", 2], ["web", 8]],
  );
  assert.equal(pools.get("reports"), null);
}
