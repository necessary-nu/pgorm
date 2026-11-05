// Example 6 of guide/20-configuration.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  await using web = new Pool(url, { maxSize: 8 });
  await using jobs = new Pool(url, { maxSize: 2 });

  assert.equal(web.status().maxSize, 8);
  assert.equal(jobs.status().maxSize, 2);
}
