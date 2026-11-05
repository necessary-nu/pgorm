// Example 2 of guide/27-postgres-releases.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { version } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  assert.match(version, /^\d+\.\d+\.\d+/);
}
