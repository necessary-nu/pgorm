// Example 4 of guide/27-postgres-releases.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { sql } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  // REPACK and ON CONFLICT .. DO SELECT are PostgreSQL 19's.
  assert.equal(sql("REPACK cake"), "REPACK cake");
  const upsert = "INSERT INTO cake (id) VALUES (1) ON CONFLICT (id) DO SELECT RETURNING *";
  assert.equal(sql(upsert), upsert);
}
