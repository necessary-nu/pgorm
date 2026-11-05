// Example 1 of guide/21-metrics.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  const named = new URL(url);
  named.search += "&application_name=bakery-web";
  await using web = new Pool(named.toString(), { maxSize: 2 });

  const row = await web.one(
    "SELECT count(*) AS sessions FROM pg_stat_activity WHERE application_name = $1 AND datname = current_database()",
    ["bakery-web"],
  );
  assert.equal(row.sessions, 1n);
}
